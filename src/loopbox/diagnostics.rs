use super::{
    config_path, doctor_report, incident_timeline_for_project,
    proxy_traffic_events_for_project_with_persisted, resource_metrics_series_for_project,
    service_logs_tail, service_runtime_status, DoctorIssue, DoctorLevel, IncidentSeverity,
    IncidentTimelineEvent, LoopboxConfig, ProxyTrafficEvent, ServiceResourceSample,
    ServiceRuntimeSnapshot,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION_CAP: usize = 200;
const INCIDENT_CAP: usize = 50;
const LOG_LINES_PER_SERVICE_CAP: usize = 80;
const REQUEST_CAP: usize = 50;
const RESOURCE_CAP: usize = 200;
const DOCTOR_ISSUE_CAP: usize = 50;
const REPORT_SUMMARY_LIMIT: usize = 240;

static DIAGNOSIS_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisStatus {
    Draft,
    InProgress,
    Completed,
    Resolved,
    Archived,
}

impl DiagnosisStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::InProgress => "in progress",
            Self::Completed => "completed",
            Self::Resolved => "resolved",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisSource {
    Sandbox,
    Service,
    Incident,
    RuntimeAlert,
}

impl DiagnosisSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Service => "service",
            Self::Incident => "incident",
            Self::RuntimeAlert => "runtime alert",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDiagnosisSessionInput {
    pub project_name: String,
    pub service_name: Option<String>,
    pub source: DiagnosisSource,
    pub window: String,
    pub incident_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisSession {
    pub id: String,
    pub created_at_unix_ms: u64,
    pub created_at_utc: String,
    pub updated_at_unix_ms: u64,
    pub status: DiagnosisStatus,
    pub source: DiagnosisSource,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    pub window: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<DiagnosisReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_utc: Option<String>,
    pub evidence: DiagnosisEvidenceSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub captured_at_unix_ms: u64,
    pub captured_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub summary: String,
    pub agent_message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisEvidenceSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_incident: Option<IncidentTimelineEvent>,
    #[serde(default)]
    pub incidents: Vec<IncidentTimelineEvent>,
    #[serde(default)]
    pub runtime: Vec<ServiceRuntimeSnapshot>,
    #[serde(default)]
    pub log_tails: Vec<DiagnosisLogTail>,
    #[serde(default)]
    pub requests: Vec<DiagnosisRequestSummary>,
    #[serde(default)]
    pub resources: Vec<ServiceResourceSample>,
    #[serde(default)]
    pub doctor_issues: Vec<DiagnosisDoctorIssue>,
}

impl DiagnosisEvidenceSnapshot {
    pub(crate) fn enforce_caps(&mut self) {
        truncate_to_newest(&mut self.incidents, INCIDENT_CAP, |event| {
            event.occurred_at_unix_ms
        });
        for tail in &mut self.log_tails {
            truncate_to_last(&mut tail.lines, LOG_LINES_PER_SERVICE_CAP);
        }
        truncate_to_last(&mut self.requests, REQUEST_CAP);
        truncate_to_newest(&mut self.resources, RESOURCE_CAP, |sample| {
            sample.sampled_at_unix_ms
        });
        truncate_to_last(&mut self.doctor_issues, DOCTOR_ISSUE_CAP);
    }

    pub fn evidence_count(&self) -> usize {
        usize::from(self.selected_incident.is_some())
            .saturating_add(self.incidents.len())
            .saturating_add(self.runtime.len())
            .saturating_add(
                self.log_tails
                    .iter()
                    .map(|tail| tail.lines.len())
                    .sum::<usize>(),
            )
            .saturating_add(self.requests.len())
            .saturating_add(self.resources.len())
            .saturating_add(self.doctor_issues.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosisLogTail {
    pub service_name: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosisRequestSummary {
    pub started_at_utc: String,
    pub service_name: String,
    pub protocol: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status_code: Option<u16>,
    pub grpc_status: Option<i32>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

impl From<ProxyTrafficEvent> for DiagnosisRequestSummary {
    fn from(event: ProxyTrafficEvent) -> Self {
        Self {
            started_at_utc: event.started_at_utc,
            service_name: event.service_name,
            protocol: event.protocol,
            host: event.host,
            method: event.method,
            path: event.path,
            status_code: event.status_code,
            grpc_status: event.grpc_status,
            duration_ms: event.duration_ms,
            error: event.error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosisDoctorIssue {
    pub level: DoctorLevel,
    pub project: Option<String>,
    pub message: String,
    pub fix_label: Option<String>,
}

impl From<DoctorIssue> for DiagnosisDoctorIssue {
    fn from(issue: DoctorIssue) -> Self {
        Self {
            level: issue.level,
            project: issue.project,
            message: issue.message,
            fix_label: issue.fix.map(|fix| fix.label().to_string()),
        }
    }
}

pub fn create_diagnosis_session(
    config: &LoopboxConfig,
    input: CreateDiagnosisSessionInput,
) -> Result<DiagnosisSession, String> {
    let project_name = input.project_name.trim().to_string();
    let project = config
        .projects
        .get(&project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let service_name = input
        .service_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(service) = service_name.as_ref() {
        if !project.services.iter().any(|entry| &entry.name == service) {
            return Err(format!(
                "Service '{service}' not found in project '{project_name}'."
            ));
        }
    }
    let window = normalize_window(&input.window);
    let evidence = collect_evidence_snapshot(
        config,
        &project_name,
        service_name.as_deref(),
        input.source,
        &window,
        input.incident_id.as_deref(),
    )?;
    let now = current_unix_millis();
    let title = input.title.unwrap_or_else(|| {
        default_session_title(
            input.source,
            &project_name,
            service_name.as_deref(),
            evidence.selected_incident.as_ref(),
        )
    });
    let session = DiagnosisSession {
        id: next_diagnosis_id(now),
        created_at_unix_ms: now,
        created_at_utc: format_unix_utc_millis(now),
        updated_at_unix_ms: now,
        status: DiagnosisStatus::Draft,
        source: input.source,
        project_name,
        service_name,
        window,
        title,
        linked_thread_id: None,
        report: None,
        resolution_note: None,
        resolved_at_unix_ms: None,
        resolved_at_utc: None,
        evidence,
    };
    save_diagnosis_session(session.clone())?;
    Ok(session)
}

pub fn diagnosis_sessions(limit: usize) -> Result<Vec<DiagnosisSession>, String> {
    let mut sessions = load_sessions()?;
    sessions.sort_by(|left, right| {
        right
            .created_at_unix_ms
            .cmp(&left.created_at_unix_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    sessions.truncate(limit.clamp(1, SESSION_CAP));
    Ok(sessions)
}

#[allow(dead_code)]
pub fn read_diagnosis_session(id: &str) -> Result<DiagnosisSession, String> {
    let id = id.trim();
    load_sessions()?
        .into_iter()
        .find(|session| session.id == id)
        .ok_or_else(|| format!("Diagnosis session '{id}' not found."))
}

pub fn link_diagnosis_session_thread(
    id: &str,
    thread_id: &str,
) -> Result<DiagnosisSession, String> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err("Codex thread id cannot be empty.".to_string());
    }
    update_session(id, |session| {
        session.linked_thread_id = Some(thread_id.to_string());
        if matches!(session.status, DiagnosisStatus::Draft) {
            session.status = DiagnosisStatus::InProgress;
        }
    })
}

pub fn update_diagnosis_session_status(
    id: &str,
    status: DiagnosisStatus,
) -> Result<DiagnosisSession, String> {
    update_session(id, |session| {
        session.status = status;
    })
}

pub fn complete_diagnosis_session(
    id: &str,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
    agent_message: Option<&str>,
) -> Result<DiagnosisSession, String> {
    let captured_at_unix_ms = current_unix_millis();
    let captured_at_utc = format_unix_utc_millis(captured_at_unix_ms);
    let normalized_thread_id = normalize_optional_text(thread_id);
    let normalized_turn_id = normalize_optional_text(turn_id);
    let report = agent_message
        .filter(|message| !message.trim().is_empty())
        .map(|message| DiagnosisReport {
            captured_at_unix_ms,
            captured_at_utc,
            thread_id: normalized_thread_id.clone(),
            turn_id: normalized_turn_id.clone(),
            summary: diagnosis_report_summary(message),
            agent_message: message.to_string(),
        });

    update_session(id, |session| {
        session.status = DiagnosisStatus::Completed;
        if let Some(thread_id) = normalized_thread_id.clone() {
            session.linked_thread_id = Some(thread_id);
        }
        if let Some(report) = report {
            session.report = Some(report);
        }
    })
}

pub fn resolve_diagnosis_session(
    id: &str,
    resolution_note: &str,
) -> Result<DiagnosisSession, String> {
    let resolved_at_unix_ms = current_unix_millis();
    let resolved_at_utc = format_unix_utc_millis(resolved_at_unix_ms);
    let resolution_note = resolution_note.trim().to_string();

    update_session(id, |session| {
        session.status = DiagnosisStatus::Resolved;
        session.resolved_at_unix_ms = Some(resolved_at_unix_ms);
        session.resolved_at_utc = Some(resolved_at_utc);
        session.resolution_note = if resolution_note.is_empty() {
            None
        } else {
            Some(resolution_note)
        };
    })
}

pub fn diagnosis_prompt_for_session(session: &DiagnosisSession) -> String {
    let service_clause = session
        .service_name
        .as_deref()
        .map(|service| format!(" service `{service}`"))
        .unwrap_or_else(|| " all services".to_string());
    let selected_incident = session
        .evidence
        .selected_incident
        .as_ref()
        .map(|event| format!(" Selected incident: `{}` - {}.", event.id, event.summary))
        .unwrap_or_default();

    format!(
        "Diagnose Loopbox diagnosis session `{}` for sandbox `{}` and{}. Source: {}. Window: `{}`.{} Use Loopbox MCP tools for fresh evidence instead of relying only on the stored snapshot. Start with `loopbox_incidents` for project `{}` and window `{}`{}, then inspect runtime, logs, requests, and resources only as needed. Stored evidence counts: {} incident(s), {} runtime snapshot(s), {} log line(s), {} request(s), {} resource sample(s), {} doctor issue(s). Return the likely cause, supporting evidence, suggested fix, and whether a mutation is needed.",
        session.id,
        session.project_name,
        service_clause,
        session.source.label(),
        session.window,
        selected_incident,
        session.project_name,
        session.window,
        session
            .service_name
            .as_deref()
            .map(|service| format!(", service `{service}`"))
            .unwrap_or_default(),
        session.evidence.incidents.len(),
        session.evidence.runtime.len(),
        session
            .evidence
            .log_tails
            .iter()
            .map(|tail| tail.lines.len())
            .sum::<usize>(),
        session.evidence.requests.len(),
        session.evidence.resources.len(),
        session.evidence.doctor_issues.len()
    )
}

fn diagnosis_report_summary(message: &str) -> String {
    let line = message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("No agent report.");
    clamp_report_summary(line)
}

fn clamp_report_summary(value: &str) -> String {
    if value.len() <= REPORT_SUMMARY_LIMIT {
        return value.to_string();
    }

    let suffix = "...";
    let target_len = REPORT_SUMMARY_LIMIT.saturating_sub(suffix.len());
    let mut summary = String::new();
    for ch in value.chars() {
        if summary.len().saturating_add(ch.len_utf8()) > target_len {
            break;
        }
        summary.push(ch);
    }
    summary = summary.trim_end().to_string();
    summary.push_str(suffix);
    summary
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn save_diagnosis_session(session: DiagnosisSession) -> Result<(), String> {
    let mut sessions = load_sessions()?;
    if let Some(existing) = sessions
        .iter_mut()
        .find(|existing| existing.id == session.id)
    {
        *existing = session;
    } else {
        sessions.push(session);
    }
    save_sessions(sessions)
}

fn update_session(
    id: &str,
    update: impl FnOnce(&mut DiagnosisSession),
) -> Result<DiagnosisSession, String> {
    let id = id.trim();
    let mut sessions = load_sessions()?;
    let Some(session) = sessions.iter_mut().find(|session| session.id == id) else {
        return Err(format!("Diagnosis session '{id}' not found."));
    };
    update(session);
    session.updated_at_unix_ms = current_unix_millis();
    let updated = session.clone();
    save_sessions(sessions)?;
    Ok(updated)
}

fn collect_evidence_snapshot(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: Option<&str>,
    source: DiagnosisSource,
    window: &str,
    incident_id: Option<&str>,
) -> Result<DiagnosisEvidenceSnapshot, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let incidents =
        incident_timeline_for_project(config, project_name, service_name, window, INCIDENT_CAP)?;
    let selected_incident = incident_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| {
            incidents
                .iter()
                .find(|event| event.id == id)
                .cloned()
                .ok_or_else(|| format!("Incident '{id}' not found in the {window} window."))
        })
        .transpose()?;
    let services = evidence_services(project, service_name, &incidents, source);

    let mut runtime = Vec::new();
    let mut log_tails = Vec::new();
    for service in &services {
        if let Ok(snapshot) = service_runtime_status(config, project_name, service) {
            runtime.push(snapshot);
        }
        if let Ok(lines) = service_logs_tail(project_name, service, LOG_LINES_PER_SERVICE_CAP) {
            log_tails.push(DiagnosisLogTail {
                service_name: service.clone(),
                lines,
            });
        }
    }

    let requests =
        proxy_traffic_events_for_project_with_persisted(project_name, service_name, REQUEST_CAP)
            .unwrap_or_default()
            .into_iter()
            .map(DiagnosisRequestSummary::from)
            .collect::<Vec<_>>();
    let resources =
        resource_metrics_series_for_project(project_name, service_name, window, RESOURCE_CAP)
            .unwrap_or_default();
    let doctor_issues = doctor_report(config)
        .into_iter()
        .filter(|issue| {
            issue
                .project
                .as_deref()
                .is_none_or(|project| project == project_name)
        })
        .take(DOCTOR_ISSUE_CAP)
        .map(DiagnosisDoctorIssue::from)
        .collect::<Vec<_>>();

    let mut snapshot = DiagnosisEvidenceSnapshot {
        selected_incident,
        incidents,
        runtime,
        log_tails,
        requests,
        resources,
        doctor_issues,
    };
    snapshot.enforce_caps();
    Ok(snapshot)
}

fn evidence_services(
    project: &super::ProjectConfig,
    service_name: Option<&str>,
    incidents: &[IncidentTimelineEvent],
    source: DiagnosisSource,
) -> Vec<String> {
    if let Some(service_name) = service_name {
        return vec![service_name.to_string()];
    }

    let mut services = incidents
        .iter()
        .filter(|event| {
            matches!(source, DiagnosisSource::Incident)
                || matches!(
                    event.severity,
                    IncidentSeverity::Warning | IncidentSeverity::Critical
                )
        })
        .filter_map(|event| event.service_name.clone())
        .collect::<Vec<_>>();
    services.sort();
    services.dedup();

    if services.is_empty() {
        services = project
            .services
            .iter()
            .map(|service| service.name.clone())
            .collect();
    }
    services
}

fn default_session_title(
    source: DiagnosisSource,
    project_name: &str,
    service_name: Option<&str>,
    selected_incident: Option<&IncidentTimelineEvent>,
) -> String {
    if let Some(event) = selected_incident {
        return event.summary.clone();
    }
    match service_name {
        Some(service) => format!("Diagnose {service} in {project_name}"),
        None => format!("Diagnose {project_name} {}", source.label()),
    }
}

fn load_sessions() -> Result<Vec<DiagnosisSession>, String> {
    let path = sessions_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "Failed to parse diagnosis sessions {}: {err}",
            path.display()
        )
    })
}

fn save_sessions(mut sessions: Vec<DiagnosisSession>) -> Result<(), String> {
    sessions.sort_by(|left, right| {
        right
            .created_at_unix_ms
            .cmp(&left.created_at_unix_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    sessions.truncate(SESSION_CAP);
    let path = sessions_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(&sessions)
        .map_err(|err| format!("Failed to serialize diagnosis sessions: {err}"))?;
    fs::write(&path, raw).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn sessions_path() -> PathBuf {
    diagnostics_dir().join("sessions.json")
}

fn diagnostics_dir() -> PathBuf {
    if let Some(path) = diagnostics_test_dir()
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
        .join("diagnostics")
}

fn next_diagnosis_id(now: u64) -> String {
    let counter = DIAGNOSIS_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("diag-{now}-{counter}")
}

fn normalize_window(window: &str) -> String {
    match window.trim() {
        "15m" | "1h" | "24h" | "7d" => window.trim().to_string(),
        _ => "1h".to_string(),
    }
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

fn truncate_to_last<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        let drain_to = values.len() - limit;
        values.drain(0..drain_to);
    }
}

fn truncate_to_newest<T>(values: &mut Vec<T>, limit: usize, timestamp: impl Fn(&T) -> u64) {
    values.sort_by_key(|value| timestamp(value));
    truncate_to_last(values, limit);
    values.sort_by_key(|value| Reverse(timestamp(value)));
}

fn diagnostics_test_dir() -> &'static Mutex<Option<PathBuf>> {
    static DIAGNOSTICS_TEST_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    DIAGNOSTICS_TEST_DIR.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn set_diagnostics_test_dir(path: Option<PathBuf>) {
    if let Ok(mut guard) = diagnostics_test_dir().lock() {
        *guard = path;
    }
}

#[cfg(test)]
fn clear_diagnostics_for_test() {
    let _ = fs::remove_dir_all(diagnostics_dir());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{
        DoctorFixAction, DoctorLevel, IncidentEvidence, IncidentKind, IncidentSeverity,
        IncidentTimelineEvent, LoopboxConfig, ProjectConfig, ProxyEndpointProtocol, ServiceConfig,
        ServiceRuntimeKind, ServiceRuntimeSnapshot, ServiceRuntimeState,
    };
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    fn diagnostics_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "loopbox-diagnostics-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        path
    }

    fn config() -> LoopboxConfig {
        let mut config = LoopboxConfig::default();
        config.projects.insert(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![ServiceConfig {
                    name: "web".to_string(),
                    runtime: ServiceRuntimeKind::Process,
                    container: None,
                    ports: Vec::new(),
                    port: Some(5173),
                    protocol: ProxyEndpointProtocol::Http1,
                    command: "npm run dev".to_string(),
                    workdir: "/tmp/demo".to_string(),
                    env_files: Vec::new(),
                    depends_on: Vec::new(),
                    autostart: false,
                    health_path: None,
                }],
                default_open_service: Some("web".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: Vec::new(),
                proxy_endpoints: Vec::new(),
            },
        );
        config
    }

    fn input(source: DiagnosisSource) -> CreateDiagnosisSessionInput {
        CreateDiagnosisSessionInput {
            project_name: "demo".to_string(),
            service_name: Some("web".to_string()),
            source,
            window: "1h".to_string(),
            incident_id: None,
            title: None,
        }
    }

    fn incident(id: &str, severity: IncidentSeverity) -> IncidentTimelineEvent {
        IncidentTimelineEvent {
            id: id.to_string(),
            occurred_at_unix_ms: 1_777_000_000_000,
            occurred_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            project_name: "demo".to_string(),
            service_name: Some("web".to_string()),
            severity,
            kind: IncidentKind::TrafficFailure,
            summary: "GET /api returned 503".to_string(),
            detail: Some("upstream failed".to_string()),
            evidence: vec![IncidentEvidence::RequestSummary {
                method: "GET".to_string(),
                path: "/api".to_string(),
                status_code: Some(503),
                duration_ms: 122,
                error: None,
            }],
            source: "traffic".to_string(),
        }
    }

    fn stored_session(id: &str, created_at_unix_ms: u64) -> DiagnosisSession {
        DiagnosisSession {
            id: id.to_string(),
            created_at_unix_ms,
            created_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            updated_at_unix_ms: created_at_unix_ms,
            status: DiagnosisStatus::Draft,
            source: DiagnosisSource::Sandbox,
            project_name: "demo".to_string(),
            service_name: Some("web".to_string()),
            window: "1h".to_string(),
            title: "Diagnose web in demo".to_string(),
            linked_thread_id: None,
            report: None,
            resolution_note: None,
            resolved_at_unix_ms: None,
            resolved_at_utc: None,
            evidence: DiagnosisEvidenceSnapshot::default(),
        }
    }

    #[test]
    fn session_store_round_trips_and_caps_to_newest_records() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("cap");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();

        for index in 0..205 {
            save_diagnosis_session(stored_session(&format!("diag-{index}"), index))
                .expect("save session");
        }

        let sessions = diagnosis_sessions(500).expect("load sessions");
        assert_eq!(sessions.len(), 200);
        assert!(sessions
            .iter()
            .any(|session| session.created_at_unix_ms == 204));
        assert!(!sessions
            .iter()
            .any(|session| session.created_at_unix_ms == 0));

        let reloaded = read_diagnosis_session(&sessions[0].id).expect("read newest session");
        assert_eq!(reloaded.id, sessions[0].id);

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_ids_are_unique_and_status_updates_target_one_record() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("status");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();

        let first =
            create_diagnosis_session(&config(), input(DiagnosisSource::Sandbox)).expect("first");
        let second =
            create_diagnosis_session(&config(), input(DiagnosisSource::Service)).expect("second");
        assert_ne!(first.id, second.id);

        link_diagnosis_session_thread(&first.id, "thread-1").expect("link thread");
        update_diagnosis_session_status(&second.id, DiagnosisStatus::Resolved)
            .expect("resolve second");

        let first = read_diagnosis_session(&first.id).expect("read first");
        let second = read_diagnosis_session(&second.id).expect("read second");
        assert_eq!(first.status, DiagnosisStatus::InProgress);
        assert_eq!(first.linked_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(second.status, DiagnosisStatus::Resolved);
        assert!(second.linked_thread_id.is_none());

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_session_json_without_report_fields_still_loads() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("backcompat");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();
        fs::create_dir_all(diagnostics_dir()).expect("create diagnostics dir");
        fs::write(
            sessions_path(),
            r#"[
  {
    "id": "diag-old",
    "created_at_unix_ms": 1777000000000,
    "created_at_utc": "2026-05-06 12:00:00 UTC",
    "updated_at_unix_ms": 1777000000000,
    "status": "draft",
    "source": "sandbox",
    "project_name": "demo",
    "service_name": "web",
    "window": "1h",
    "title": "Old session",
    "linked_thread_id": "thread-old",
    "evidence": {}
  }
]"#,
        )
        .expect("write old sessions");

        let sessions = diagnosis_sessions(10).expect("load old sessions");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "diag-old");
        assert!(sessions[0].report.is_none());
        assert!(sessions[0].resolution_note.is_none());
        assert!(sessions[0].resolved_at_unix_ms.is_none());
        assert!(sessions[0].resolved_at_utc.is_none());

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn complete_diagnosis_session_persists_agent_report() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("complete");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();
        let session =
            create_diagnosis_session(&config(), input(DiagnosisSource::Service)).expect("session");
        let message = "Likely cause: backend returned 503.\n\nThe request evidence points at an upstream service crash.";

        let updated = complete_diagnosis_session(
            &session.id,
            Some("thread-1"),
            Some("turn-1"),
            Some(message),
        )
        .expect("complete session");

        assert_eq!(updated.status, DiagnosisStatus::Completed);
        assert_eq!(updated.linked_thread_id.as_deref(), Some("thread-1"));
        let report = updated.report.expect("report");
        assert_eq!(report.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(report.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(report.summary, "Likely cause: backend returned 503.");
        assert_eq!(report.agent_message, message);
        assert!(report.captured_at_unix_ms >= session.created_at_unix_ms);
        assert!(report.captured_at_utc.ends_with("UTC"));

        let reloaded = read_diagnosis_session(&session.id).expect("read session");
        assert_eq!(
            reloaded
                .report
                .as_ref()
                .map(|report| report.summary.as_str()),
            Some("Likely cause: backend returned 503.")
        );

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn complete_diagnosis_session_without_agent_message_only_updates_status() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("complete-empty");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();
        let session =
            create_diagnosis_session(&config(), input(DiagnosisSource::Service)).expect("session");

        let updated =
            complete_diagnosis_session(&session.id, Some("thread-1"), Some("turn-1"), None)
                .expect("complete session");

        assert_eq!(updated.status, DiagnosisStatus::Completed);
        assert_eq!(updated.linked_thread_id.as_deref(), Some("thread-1"));
        assert!(updated.report.is_none());

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_diagnosis_session_persists_resolution_note_and_timestamp() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("resolve");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();
        let session =
            create_diagnosis_session(&config(), input(DiagnosisSource::Service)).expect("session");

        let updated = resolve_diagnosis_session(&session.id, "Restarted backend after fixing env.")
            .expect("resolve session");

        assert_eq!(updated.status, DiagnosisStatus::Resolved);
        assert_eq!(
            updated.resolution_note.as_deref(),
            Some("Restarted backend after fixing env.")
        );
        assert!(updated.resolved_at_unix_ms.is_some());
        assert!(updated
            .resolved_at_utc
            .as_deref()
            .is_some_and(|ts| ts.ends_with("UTC")));

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn report_summary_uses_first_non_empty_line_and_ascii_clamp() {
        assert_eq!(
            diagnosis_report_summary("\n\nLikely cause: missing env.\nSecond line"),
            "Likely cause: missing env."
        );

        let long_line = "a".repeat(260);
        let summary = diagnosis_report_summary(&long_line);

        assert_eq!(summary.len(), 240);
        assert!(summary.ends_with("..."));
        assert!(summary.is_ascii());
    }

    #[test]
    fn evidence_snapshot_filters_selected_incident_and_caps_collections() {
        let mut snapshot = DiagnosisEvidenceSnapshot {
            selected_incident: Some(incident("selected", IncidentSeverity::Critical)),
            incidents: (0..60)
                .map(|index| incident(&format!("incident-{index}"), IncidentSeverity::Warning))
                .collect(),
            runtime: vec![ServiceRuntimeSnapshot {
                project: "demo".to_string(),
                service: "web".to_string(),
                state: ServiceRuntimeState::Running,
                pid: Some(123),
                started_at: Some(1),
                exit_code: None,
                last_error: None,
            }],
            log_tails: vec![DiagnosisLogTail {
                service_name: "web".to_string(),
                lines: (0..100).map(|index| format!("line-{index}")).collect(),
            }],
            doctor_issues: vec![DiagnosisDoctorIssue {
                level: DoctorLevel::Warning,
                project: Some("demo".to_string()),
                message: "Loopback alias missing.".to_string(),
                fix_label: Some(DoctorFixAction::ApplySystemSetup.label().to_string()),
            }],
            ..DiagnosisEvidenceSnapshot::default()
        };

        snapshot.enforce_caps();

        assert_eq!(
            snapshot
                .selected_incident
                .as_ref()
                .map(|event| event.id.as_str()),
            Some("selected")
        );
        assert_eq!(snapshot.incidents.len(), 50);
        assert_eq!(snapshot.log_tails[0].lines.len(), 80);
        assert_eq!(snapshot.doctor_issues[0].level, DoctorLevel::Warning);
    }

    #[test]
    fn prompt_includes_session_identity_and_mcp_first_workflow() {
        let _guard = diagnostics_test_lock().lock().expect("test lock");
        let dir = temp_dir("prompt");
        set_diagnostics_test_dir(Some(dir.clone()));
        clear_diagnostics_for_test();

        let mut session =
            create_diagnosis_session(&config(), input(DiagnosisSource::Service)).expect("session");
        session.id = "diag-test".to_string();
        session.evidence.incidents = vec![incident("incident-1", IncidentSeverity::Warning)];

        let prompt = diagnosis_prompt_for_session(&session);

        assert!(prompt.contains("diag-test"));
        assert!(prompt.contains("sandbox `demo`"));
        assert!(prompt.contains("service `web`"));
        assert!(prompt.contains("loopbox_incidents"));
        assert!(prompt.contains("likely cause"));
        assert!(prompt.contains("whether a mutation is needed"));

        clear_diagnostics_for_test();
        set_diagnostics_test_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
