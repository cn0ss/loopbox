use super::{
    config_path, proxy_traffic_events_for_project_with_persisted,
    resource_metrics_series_for_project, service_logs_tail, LoopboxConfig, ProxyTrafficEvent,
    ServiceResourceSample, ServiceRuntimeSnapshot, ServiceRuntimeState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const INCIDENT_RETENTION_DAYS: u16 = 7;
const TRAFFIC_SLOW_REQUEST_MS: u64 = 1_000;
const RESOURCE_CPU_PRESSURE_PERCENT: f64 = 80.0;
const RESOURCE_CPU_PRESSURE_MIN_SAMPLES: usize = 3;
const RESOURCE_MEMORY_GROWTH_MIN_BYTES: u64 = 256 * 1024 * 1024;
const RESOURCE_MEMORY_GROWTH_FACTOR: f64 = 2.0;
const LOG_EVIDENCE_LIMIT: usize = 3;
const LOG_EVIDENCE_TAIL_LIMIT: usize = 500;
const MAX_INCIDENT_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentKind {
    RuntimeTransition,
    TrafficFailure,
    SlowRequest,
    ResourcePressure,
    ResourceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncidentEvidence {
    RuntimeSnapshot {
        state: ServiceRuntimeState,
        pid: Option<u32>,
        started_at: Option<u64>,
        exit_code: Option<i32>,
        last_error: Option<String>,
    },
    RequestSummary {
        method: String,
        path: String,
        status_code: Option<u16>,
        duration_ms: u64,
        error: Option<String>,
    },
    ResourceSampleSummary {
        sampled_at_utc: String,
        cpu_percent: Option<f64>,
        memory_bytes: Option<u64>,
        process_count: Option<usize>,
        unavailable_reason: Option<String>,
    },
    LogExcerpt {
        service_name: String,
        line: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncidentTimelineEvent {
    pub id: String,
    pub occurred_at_unix_ms: u64,
    pub occurred_at_utc: String,
    pub project_name: String,
    pub service_name: Option<String>,
    pub severity: IncidentSeverity,
    pub kind: IncidentKind,
    pub summary: String,
    pub detail: Option<String>,
    pub evidence: Vec<IncidentEvidence>,
    pub source: String,
}

pub fn incident_timeline_for_project(
    config: &LoopboxConfig,
    project_name: &str,
    service_filter: Option<&str>,
    window: &str,
    limit: usize,
) -> Result<Vec<IncidentTimelineEvent>, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let normalized_service = service_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(service_name) = normalized_service.as_ref() {
        if !project
            .services
            .iter()
            .any(|service| &service.name == service_name)
        {
            return Err(format!(
                "Service '{service_name}' not found in project '{project_name}'."
            ));
        }
    }

    let window_ms = incident_window_millis(window)
        .ok_or_else(|| "Incident window must be one of 15m, 1h, 24h, or 7d.".to_string())?;
    let now = current_unix_millis();
    let cutoff = now.saturating_sub(window_ms);
    let effective_limit = limit.clamp(1, MAX_INCIDENT_LIMIT);

    let _ = cleanup_incident_storage(&incident_events_dir(), INCIDENT_RETENTION_DAYS, now);
    let mut events = load_incident_events_from_disk(
        project_name,
        normalized_service.as_deref(),
        cutoff,
        effective_limit,
    )?;
    let log_lookup =
        collect_log_evidence_lookup(project_name, project, normalized_service.as_deref());

    let request_limit = effective_limit
        .saturating_mul(4)
        .clamp(1, MAX_INCIDENT_LIMIT * 4);
    if let Ok(requests) = proxy_traffic_events_for_project_with_persisted(
        project_name,
        normalized_service.as_deref(),
        request_limit,
    ) {
        events.extend(
            synthesize_traffic_incidents(&requests, &log_lookup)
                .into_iter()
                .filter(|event| event.occurred_at_unix_ms >= cutoff),
        );
    }

    let resource_limit = effective_limit.saturating_mul(10).clamp(1, 2_000);
    if let Ok(samples) = resource_metrics_series_for_project(
        project_name,
        normalized_service.as_deref(),
        window,
        resource_limit,
    ) {
        events.extend(
            synthesize_resource_incidents(&samples, &log_lookup)
                .into_iter()
                .filter(|event| event.occurred_at_unix_ms >= cutoff),
        );
    }

    events.retain(|event| {
        normalized_service
            .as_ref()
            .is_none_or(|service| event.service_name.as_ref() == Some(service))
    });
    events.sort_by(|left, right| {
        right
            .occurred_at_unix_ms
            .cmp(&left.occurred_at_unix_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    events.dedup_by(|left, right| left.id == right.id);
    events.truncate(effective_limit);
    Ok(events)
}

pub fn record_runtime_incident_transition(
    previous: Option<&ServiceRuntimeSnapshot>,
    snapshot: &ServiceRuntimeSnapshot,
) -> Result<bool, String> {
    if previous.is_some_and(|previous| runtime_signature(previous) == runtime_signature(snapshot)) {
        return Ok(false);
    }

    let event = runtime_transition_event(snapshot);
    append_incident_event_to_disk(&event)?;
    let _ = cleanup_incident_storage(
        &incident_events_dir(),
        INCIDENT_RETENTION_DAYS,
        event.occurred_at_unix_ms,
    );
    Ok(true)
}

pub(crate) fn incident_window_millis(window: &str) -> Option<u64> {
    match window.trim() {
        "15m" => Some(15 * 60 * 1000),
        "1h" => Some(60 * 60 * 1000),
        "24h" => Some(24 * 60 * 60 * 1000),
        "7d" => Some(7 * 24 * 60 * 60 * 1000),
        _ => None,
    }
}

fn runtime_signature(
    snapshot: &ServiceRuntimeSnapshot,
) -> (
    ServiceRuntimeState,
    Option<u32>,
    Option<i32>,
    Option<String>,
) {
    (
        snapshot.state,
        snapshot.pid,
        snapshot.exit_code,
        snapshot.last_error.clone(),
    )
}

fn runtime_transition_event(snapshot: &ServiceRuntimeSnapshot) -> IncidentTimelineEvent {
    let occurred_at_unix_ms = current_unix_millis();
    let severity = runtime_incident_severity(snapshot);
    let state = runtime_state_label(snapshot.state);
    let detail = snapshot.last_error.clone().or_else(|| {
        snapshot
            .exit_code
            .map(|code| format!("Process exited with status code {code}."))
    });
    IncidentTimelineEvent {
        id: format!(
            "runtime:{}:{}:{}:{}:{:?}:{:?}",
            snapshot.project,
            snapshot.service,
            occurred_at_unix_ms,
            state,
            snapshot.pid,
            snapshot.exit_code
        ),
        occurred_at_unix_ms,
        occurred_at_utc: format_unix_utc_millis(occurred_at_unix_ms),
        project_name: snapshot.project.clone(),
        service_name: Some(snapshot.service.clone()),
        severity,
        kind: IncidentKind::RuntimeTransition,
        summary: format!("{} became {state}", snapshot.service),
        detail,
        evidence: vec![IncidentEvidence::RuntimeSnapshot {
            state: snapshot.state,
            pid: snapshot.pid,
            started_at: snapshot.started_at,
            exit_code: snapshot.exit_code,
            last_error: snapshot.last_error.clone(),
        }],
        source: "runtime".to_string(),
    }
}

fn runtime_incident_severity(snapshot: &ServiceRuntimeSnapshot) -> IncidentSeverity {
    match snapshot.state {
        ServiceRuntimeState::Crashed => IncidentSeverity::Critical,
        ServiceRuntimeState::Unhealthy => IncidentSeverity::Warning,
        ServiceRuntimeState::Stopped
            if snapshot.exit_code.is_some_and(|code| code != 0)
                || snapshot.last_error.is_some() =>
        {
            IncidentSeverity::Warning
        }
        ServiceRuntimeState::Stopped
        | ServiceRuntimeState::Starting
        | ServiceRuntimeState::Running => IncidentSeverity::Info,
    }
}

pub(crate) fn synthesize_traffic_incidents(
    events: &[ProxyTrafficEvent],
    log_lookup: &[(String, Vec<String>)],
) -> Vec<IncidentTimelineEvent> {
    let mut incidents = Vec::new();
    for event in events {
        let (kind, severity, summary) = if event.error.is_some() {
            (
                IncidentKind::TrafficFailure,
                IncidentSeverity::Critical,
                format!("{} {} failed before a response", event.method, event.path),
            )
        } else if event.status_code.is_some_and(|code| code >= 500) {
            (
                IncidentKind::TrafficFailure,
                IncidentSeverity::Critical,
                format!(
                    "{} {} returned {}",
                    event.method,
                    event.path,
                    event.status_code.unwrap_or_default()
                ),
            )
        } else if event.duration_ms >= TRAFFIC_SLOW_REQUEST_MS {
            (
                IncidentKind::SlowRequest,
                IncidentSeverity::Warning,
                format!(
                    "{} {} took {}ms",
                    event.method, event.path, event.duration_ms
                ),
            )
        } else {
            continue;
        };

        let occurred_at_unix_ms =
            parse_utc_timestamp_millis(&event.started_at_utc).unwrap_or_else(current_unix_millis);
        let mut evidence = vec![IncidentEvidence::RequestSummary {
            method: event.method.clone(),
            path: event.path.clone(),
            status_code: event.status_code,
            duration_ms: event.duration_ms,
            error: event.error.clone(),
        }];
        evidence.extend(log_evidence_for_service_lookup(
            &event.service_name,
            log_lookup,
        ));
        incidents.push(IncidentTimelineEvent {
            id: format!(
                "traffic:{}:{}:{}:{}:{}",
                event.project_name,
                event.service_name,
                event.id,
                kind_label(kind),
                occurred_at_unix_ms
            ),
            occurred_at_unix_ms,
            occurred_at_utc: event.started_at_utc.clone(),
            project_name: event.project_name.clone(),
            service_name: Some(event.service_name.clone()),
            severity,
            kind,
            summary,
            detail: event.error.clone(),
            evidence,
            source: "traffic".to_string(),
        });
    }
    incidents
}

pub(crate) fn synthesize_resource_incidents(
    samples: &[ServiceResourceSample],
    log_lookup: &[(String, Vec<String>)],
) -> Vec<IncidentTimelineEvent> {
    let mut incidents = Vec::new();
    let mut by_service = BTreeMap::<String, Vec<&ServiceResourceSample>>::new();
    for sample in samples {
        by_service
            .entry(sample.service_name.clone())
            .or_default()
            .push(sample);
        if let Some(reason) = sample.unavailable_reason.as_ref() {
            let mut evidence = vec![resource_sample_evidence(sample)];
            evidence.extend(log_evidence_for_service_lookup(
                &sample.service_name,
                log_lookup,
            ));
            incidents.push(IncidentTimelineEvent {
                id: format!(
                    "resource-unavailable:{}:{}:{}",
                    sample.project_name, sample.service_name, sample.sampled_at_unix_ms
                ),
                occurred_at_unix_ms: sample.sampled_at_unix_ms,
                occurred_at_utc: sample.sampled_at_utc.clone(),
                project_name: sample.project_name.clone(),
                service_name: Some(sample.service_name.clone()),
                severity: IncidentSeverity::Warning,
                kind: IncidentKind::ResourceUnavailable,
                summary: format!("{} resource metrics unavailable", sample.service_name),
                detail: Some(reason.clone()),
                evidence,
                source: "resources".to_string(),
            });
        }
    }

    for (service_name, mut service_samples) in by_service {
        service_samples.sort_by_key(|sample| sample.sampled_at_unix_ms);
        let high_cpu = service_samples
            .iter()
            .rev()
            .take_while(|sample| {
                sample
                    .cpu_percent
                    .is_some_and(|value| value >= RESOURCE_CPU_PRESSURE_PERCENT)
            })
            .count();
        if high_cpu >= RESOURCE_CPU_PRESSURE_MIN_SAMPLES {
            if let Some(sample) = service_samples.last().copied() {
                let mut evidence = vec![resource_sample_evidence(sample)];
                evidence.extend(log_evidence_for_service_lookup(&service_name, log_lookup));
                incidents.push(IncidentTimelineEvent {
                    id: format!(
                        "resource-cpu:{}:{}:{}",
                        sample.project_name, service_name, sample.sampled_at_unix_ms
                    ),
                    occurred_at_unix_ms: sample.sampled_at_unix_ms,
                    occurred_at_utc: sample.sampled_at_utc.clone(),
                    project_name: sample.project_name.clone(),
                    service_name: Some(service_name.clone()),
                    severity: IncidentSeverity::Warning,
                    kind: IncidentKind::ResourcePressure,
                    summary: format!("{} sustained high CPU", service_name),
                    detail: Some(format!(
                        "{high_cpu} consecutive samples at or above {:.0}% CPU.",
                        RESOURCE_CPU_PRESSURE_PERCENT
                    )),
                    evidence,
                    source: "resources".to_string(),
                });
            }
        }

        let memory_samples = service_samples
            .iter()
            .filter_map(|sample| sample.memory_bytes.map(|bytes| (*sample, bytes)))
            .collect::<Vec<_>>();
        if let (Some((first_sample, first)), Some((last_sample, last))) = (
            memory_samples.first().copied(),
            memory_samples.last().copied(),
        ) {
            let grew_enough = last.saturating_sub(first) >= RESOURCE_MEMORY_GROWTH_MIN_BYTES;
            let doubled =
                first > 0 && (last as f64 / first as f64) >= RESOURCE_MEMORY_GROWTH_FACTOR;
            if grew_enough && doubled {
                let mut evidence = vec![
                    resource_sample_evidence(first_sample),
                    resource_sample_evidence(last_sample),
                ];
                evidence.extend(log_evidence_for_service_lookup(&service_name, log_lookup));
                incidents.push(IncidentTimelineEvent {
                    id: format!(
                        "resource-memory:{}:{}:{}",
                        last_sample.project_name, service_name, last_sample.sampled_at_unix_ms
                    ),
                    occurred_at_unix_ms: last_sample.sampled_at_unix_ms,
                    occurred_at_utc: last_sample.sampled_at_utc.clone(),
                    project_name: last_sample.project_name.clone(),
                    service_name: Some(service_name.clone()),
                    severity: IncidentSeverity::Warning,
                    kind: IncidentKind::ResourcePressure,
                    summary: format!("{} memory grew sharply", service_name),
                    detail: Some(format!(
                        "Memory increased by {} bytes across the selected window.",
                        last.saturating_sub(first)
                    )),
                    evidence,
                    source: "resources".to_string(),
                });
            }
        }
    }

    incidents
}

fn resource_sample_evidence(sample: &ServiceResourceSample) -> IncidentEvidence {
    IncidentEvidence::ResourceSampleSummary {
        sampled_at_utc: sample.sampled_at_utc.clone(),
        cpu_percent: sample.cpu_percent,
        memory_bytes: sample.memory_bytes,
        process_count: sample.process_count,
        unavailable_reason: sample.unavailable_reason.clone(),
    }
}

fn collect_log_evidence_lookup(
    project_name: &str,
    project: &super::ProjectConfig,
    service_filter: Option<&str>,
) -> Vec<(String, Vec<String>)> {
    project
        .services
        .iter()
        .filter(|service| service_filter.is_none_or(|filter| filter == service.name))
        .filter_map(|service| {
            service_logs_tail(project_name, &service.name, LOG_EVIDENCE_TAIL_LIMIT)
                .ok()
                .map(|lines| (service.name.clone(), lines))
        })
        .collect()
}

fn log_evidence_for_service_lookup(
    service_name: &str,
    log_lookup: &[(String, Vec<String>)],
) -> Vec<IncidentEvidence> {
    log_lookup
        .iter()
        .find(|(name, _)| name == service_name)
        .map(|(_, lines)| log_evidence_for_lines(service_name, lines))
        .unwrap_or_default()
}

pub(crate) fn log_evidence_for_lines(
    service_name: &str,
    lines: &[String],
) -> Vec<IncidentEvidence> {
    lines
        .iter()
        .rev()
        .filter(|line| log_line_indicates_incident(line))
        .take(LOG_EVIDENCE_LIMIT)
        .map(|line| IncidentEvidence::LogExcerpt {
            service_name: service_name.to_string(),
            line: line.clone(),
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn log_line_indicates_incident(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    [
        "error",
        "exception",
        "panic",
        "fatal",
        "failed",
        "eaddrinuse",
        "address already in use",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub(crate) fn append_incident_event_to_disk(event: &IncidentTimelineEvent) -> Result<(), String> {
    let storage_dir = incident_events_dir();
    let day_key = millis_to_day_key(event.occurred_at_unix_ms);
    open_incident_jsonl_file(&storage_dir, &day_key).and_then(|mut file| {
        let line = serde_json::to_string(event)
            .map_err(|err| format!("Failed to serialize incident event: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("Failed to write incident event: {err}"))
    })
}

pub(crate) fn load_incident_events_from_disk(
    project_name: &str,
    service_filter: Option<&str>,
    cutoff_unix_ms: u64,
    limit: usize,
) -> Result<Vec<IncidentTimelineEvent>, String> {
    let storage_dir = incident_events_dir();
    let mut files = incident_event_files(&storage_dir)?;
    let cutoff_day = millis_to_epoch_days(cutoff_unix_ms);
    files.retain(|file| file.day_serial >= cutoff_day);
    files.sort_by_key(|file| file.day_serial);

    let mut events = Vec::new();
    let mut seen = HashSet::new();
    for file in files {
        for event in incident_events_from_file(&file.path) {
            if event.project_name != project_name || event.occurred_at_unix_ms < cutoff_unix_ms {
                continue;
            }
            if service_filter.is_some_and(|service| event.service_name.as_deref() != Some(service))
            {
                continue;
            }
            if seen.insert(event.id.clone()) {
                events.push(event);
            }
        }
    }
    events.sort_by_key(|event| event.occurred_at_unix_ms);
    if events.len() > limit {
        let drain_to = events.len() - limit;
        events.drain(0..drain_to);
    }
    Ok(events)
}

pub(crate) fn cleanup_incident_storage(
    storage_dir: &Path,
    retention_days: u16,
    now_unix_ms: u64,
) -> Result<(), String> {
    if !storage_dir.exists() {
        return Ok(());
    }
    let retention_days = i64::from(retention_days.max(1));
    let min_day = millis_to_epoch_days(now_unix_ms).saturating_sub(retention_days);
    for file in incident_event_files(storage_dir)? {
        if file.day_serial < min_day {
            fs::remove_file(&file.path).map_err(|err| {
                format!(
                    "Failed to remove old incident file {}: {err}",
                    file.path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn open_incident_jsonl_file(storage_dir: &Path, day_key: &str) -> Result<File, String> {
    fs::create_dir_all(storage_dir)
        .map_err(|err| format!("Failed to create {}: {err}", storage_dir.display()))?;
    let path = storage_dir.join(format!("{day_key}.jsonl"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("Failed to open incident file {}: {err}", path.display()))
}

fn incident_events_from_file(path: &Path) -> Vec<IncidentTimelineEvent> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<IncidentTimelineEvent>(&line).ok())
        .collect()
}

fn incident_event_files(storage_dir: &Path) -> Result<Vec<IncidentFileMeta>, String> {
    if !storage_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(storage_dir).map_err(|err| {
        format!(
            "Failed to list incident dir {}: {err}",
            storage_dir.display()
        )
    })? {
        let entry = entry.map_err(|err| format!("Failed to read incident dir entry: {err}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(day_serial) = parse_day_from_incident_filename(name) else {
            continue;
        };
        files.push(IncidentFileMeta { path, day_serial });
    }
    Ok(files)
}

#[derive(Debug)]
struct IncidentFileMeta {
    path: PathBuf,
    day_serial: i64,
}

fn parse_day_from_incident_filename(name: &str) -> Option<i64> {
    let day = name.strip_suffix(".jsonl")?;
    parse_day_key(day).map(|(year, month, day)| days_from_civil(year, month, day))
}

fn incident_events_dir() -> PathBuf {
    if let Some(path) = incident_test_dir()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
    {
        return path;
    }
    config_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("incident-events")
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn format_unix_utc_millis(epoch_ms: u64) -> String {
    let seconds = i64::try_from(epoch_ms / 1000).unwrap_or(i64::MAX);
    let days = seconds.div_euclid(86_400);
    let day_remainder = seconds.rem_euclid(86_400);
    let hour = day_remainder / 3_600;
    let minute = (day_remainder % 3_600) / 60;
    let second = day_remainder % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn parse_utc_timestamp_millis(raw: &str) -> Option<u64> {
    let stripped = raw.trim().strip_suffix(" UTC")?;
    let (date, time) = stripped.split_once(' ')?;
    let (year, month, day) = parse_day_key(date)?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second = time_parts.next()?.parse::<i64>().ok()?;
    if time_parts.next().is_some() {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3_600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    u64::try_from(seconds).ok().map(|value| value * 1000)
}

fn millis_to_day_key(epoch_ms: u64) -> String {
    let days = millis_to_epoch_days(epoch_ms);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn millis_to_epoch_days(epoch_ms: u64) -> i64 {
    i64::try_from(epoch_ms / 1000)
        .unwrap_or(i64::MAX)
        .div_euclid(86_400)
}

fn parse_day_key(raw: &str) -> Option<(i64, i64, i64)> {
    let mut parts = raw.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let day = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn runtime_state_label(state: ServiceRuntimeState) -> &'static str {
    match state {
        ServiceRuntimeState::Stopped => "stopped",
        ServiceRuntimeState::Starting => "starting",
        ServiceRuntimeState::Running => "running",
        ServiceRuntimeState::Unhealthy => "unhealthy",
        ServiceRuntimeState::Crashed => "crashed",
    }
}

fn kind_label(kind: IncidentKind) -> &'static str {
    match kind {
        IncidentKind::RuntimeTransition => "runtime",
        IncidentKind::TrafficFailure => "traffic_failure",
        IncidentKind::SlowRequest => "slow_request",
        IncidentKind::ResourcePressure => "resource_pressure",
        IncidentKind::ResourceUnavailable => "resource_unavailable",
    }
}

fn incident_test_dir() -> &'static Mutex<Option<PathBuf>> {
    static INCIDENT_TEST_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    INCIDENT_TEST_DIR.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn incident_test_lock() -> &'static Mutex<()> {
    static INCIDENT_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    INCIDENT_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn set_incident_test_dir(path: Option<PathBuf>) {
    if let Ok(mut guard) = incident_test_dir().lock() {
        *guard = path;
    }
}

#[cfg(test)]
fn clear_incidents_for_test() {
    let dir = incident_events_dir();
    let _ = fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{
        ProxyTrafficEvent, ProxyTrafficHeader, ServiceResourceSample, ServiceRuntimeKind,
        ServiceRuntimeSnapshot, ServiceRuntimeState,
    };

    fn snapshot(
        project: &str,
        service: &str,
        state: ServiceRuntimeState,
    ) -> ServiceRuntimeSnapshot {
        ServiceRuntimeSnapshot {
            project: project.to_string(),
            service: service.to_string(),
            state,
            pid: Some(123),
            started_at: Some(1_776_000_000),
            exit_code: None,
            last_error: None,
        }
    }

    fn sample(
        service: &str,
        ts: u64,
        cpu: Option<f64>,
        memory: Option<u64>,
    ) -> ServiceResourceSample {
        ServiceResourceSample {
            project_name: "demo".to_string(),
            service_name: service.to_string(),
            sampled_at_unix_ms: ts,
            sampled_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
            runtime: ServiceRuntimeKind::Process,
            state: ServiceRuntimeState::Running,
            pid: Some(123),
            cpu_percent: cpu,
            memory_bytes: memory,
            process_count: Some(1),
            container_name: None,
            unavailable_reason: None,
        }
    }

    fn request(service: &str, status: Option<u16>, duration_ms: u64) -> ProxyTrafficEvent {
        ProxyTrafficEvent {
            id: 42,
            started_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
            project_name: "demo".to_string(),
            service_name: service.to_string(),
            protocol: "http1".to_string(),
            host: "web.demo.localhost".to_string(),
            method: "GET".to_string(),
            path: "/api".to_string(),
            status_code: status,
            stream_id: None,
            grpc_service: None,
            grpc_method: None,
            grpc_status: None,
            grpc_message: None,
            duration_ms,
            request_bytes: 120,
            response_bytes: 240,
            request_header_bytes: 80,
            request_body_bytes: 0,
            response_header_bytes: 90,
            response_body_bytes: 150,
            request_headers: vec![ProxyTrafficHeader {
                name: "accept".to_string(),
                value: "application/json".to_string(),
            }],
            response_headers: Vec::new(),
            request_body_preview: None,
            response_body_preview: None,
            request_body_truncated: false,
            response_body_truncated: false,
            request_body_binary: false,
            response_body_binary: false,
            error: None,
        }
    }

    #[test]
    fn incident_window_parser_accepts_supported_windows() {
        assert_eq!(incident_window_millis("15m"), Some(15 * 60 * 1000));
        assert_eq!(incident_window_millis("1h"), Some(60 * 60 * 1000));
        assert_eq!(incident_window_millis("24h"), Some(24 * 60 * 60 * 1000));
        assert_eq!(incident_window_millis("7d"), Some(7 * 24 * 60 * 60 * 1000));
        assert_eq!(incident_window_millis("30d"), None);
    }

    #[test]
    fn runtime_incident_dedup_records_only_material_transitions() {
        let _guard = incident_test_lock().lock().expect("incident test lock");
        let dir = std::env::temp_dir().join(format!("loopbox-incidents-{}", current_unix_millis()));
        set_incident_test_dir(Some(dir.clone()));
        clear_incidents_for_test();

        let first = snapshot("demo", "web", ServiceRuntimeState::Running);
        let duplicate = snapshot("demo", "web", ServiceRuntimeState::Running);
        let mut unhealthy = snapshot("demo", "web", ServiceRuntimeState::Unhealthy);
        unhealthy.last_error = Some("health check failed".to_string());

        assert!(record_runtime_incident_transition(None, &first).expect("record first"));
        assert!(
            !record_runtime_incident_transition(Some(&first), &duplicate)
                .expect("dedupe duplicate")
        );
        assert!(
            record_runtime_incident_transition(Some(&duplicate), &unhealthy)
                .expect("record changed")
        );

        let events = load_incident_events_from_disk("demo", Some("web"), 0, 20)
            .expect("load incident events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].severity, IncidentSeverity::Info);
        assert_eq!(events[1].severity, IncidentSeverity::Warning);
        assert!(events[1].summary.contains("unhealthy"));

        set_incident_test_dir(None);
    }

    #[test]
    fn incident_storage_round_trips_and_cleanup_removes_old_files() {
        let _guard = incident_test_lock().lock().expect("incident test lock");
        let dir = std::env::temp_dir().join(format!(
            "loopbox-incidents-cleanup-{}",
            current_unix_millis()
        ));
        set_incident_test_dir(Some(dir.clone()));
        clear_incidents_for_test();

        let now = current_unix_millis();
        append_incident_event_to_disk(&runtime_transition_event(&snapshot(
            "demo",
            "web",
            ServiceRuntimeState::Crashed,
        )))
        .expect("write incident event");
        let old_path = dir.join("2000-01-01.jsonl");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(&old_path, "{}\n").expect("write old file");

        cleanup_incident_storage(&dir, 7, now).expect("cleanup incidents");
        assert!(!old_path.exists());

        let events =
            load_incident_events_from_disk("demo", None, 0, 20).expect("load incident events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].project_name, "demo");

        set_incident_test_dir(None);
    }

    #[test]
    fn traffic_and_resource_synthesis_emit_expected_incidents() {
        let request_incidents = synthesize_traffic_incidents(
            &[
                request("web", Some(503), 120),
                request("web", Some(200), 1500),
            ],
            &[],
        );
        assert_eq!(request_incidents.len(), 2);
        assert_eq!(request_incidents[0].severity, IncidentSeverity::Critical);
        assert_eq!(request_incidents[1].kind, IncidentKind::SlowRequest);

        let resource_incidents = synthesize_resource_incidents(
            &[
                sample("web", 1, Some(82.0), Some(256 * 1024 * 1024)),
                sample("web", 2, Some(85.0), Some(384 * 1024 * 1024)),
                sample("web", 3, Some(90.0), Some(640 * 1024 * 1024)),
            ],
            &[],
        );
        assert!(resource_incidents
            .iter()
            .any(|event| event.kind == IncidentKind::ResourcePressure));
    }

    #[test]
    fn log_evidence_is_capped_and_case_insensitive() {
        let evidence = log_evidence_for_lines(
            "web",
            &[
                "normal".to_string(),
                "ERROR failed to bind".to_string(),
                "panic in worker".to_string(),
                "EADDRINUSE 5173".to_string(),
                "fatal shutdown".to_string(),
            ],
        );

        assert_eq!(evidence.len(), 3);
        assert!(matches!(evidence[0], IncidentEvidence::LogExcerpt { .. }));
    }
}
