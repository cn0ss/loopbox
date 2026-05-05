use super::{
    config_path, service_runtime_status, LoopboxConfig, ServiceRuntimeKind, ServiceRuntimeState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RESOURCE_METRICS_MAX_SERIES_LIMIT: usize = 20_000;
const RESOURCE_METRICS_CLEANUP_WRITES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceMetricsSettings {
    #[serde(default = "default_resource_metrics_enabled")]
    pub enabled: bool,
    #[serde(default = "default_resource_metrics_sample_interval_secs")]
    pub sample_interval_secs: u64,
    #[serde(default = "default_resource_metrics_retention_days")]
    pub retention_days: u16,
    #[serde(default = "default_resource_metrics_max_storage_mb")]
    pub max_storage_mb: usize,
}

impl Default for ResourceMetricsSettings {
    fn default() -> Self {
        Self {
            enabled: default_resource_metrics_enabled(),
            sample_interval_secs: default_resource_metrics_sample_interval_secs(),
            retention_days: default_resource_metrics_retention_days(),
            max_storage_mb: default_resource_metrics_max_storage_mb(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceResourceSample {
    pub project_name: String,
    pub service_name: String,
    pub sampled_at_unix_ms: u64,
    pub sampled_at_utc: String,
    pub runtime: ServiceRuntimeKind,
    pub state: ServiceRuntimeState,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub process_count: Option<usize>,
    pub container_name: Option<String>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMetricsDiskStats {
    pub dropped_samples: u64,
    pub total_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Default)]
struct ResourceMetricsStore {
    latest: HashMap<String, ServiceResourceSample>,
}

#[derive(Debug, Default)]
struct ResourceMetricsSamplerState {
    running: Option<RunningResourceMetricsSampler>,
    dropped_samples: u64,
}

#[derive(Debug)]
struct RunningResourceMetricsSampler {
    config: LoopboxConfig,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

static RESOURCE_METRICS_STORE: OnceLock<Mutex<ResourceMetricsStore>> = OnceLock::new();
static RESOURCE_METRICS_SAMPLER_STATE: OnceLock<Mutex<ResourceMetricsSamplerState>> =
    OnceLock::new();
#[cfg(test)]
static RESOURCE_METRICS_TEST_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
#[cfg(test)]
static RESOURCE_METRICS_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn sync_resource_metrics_sampler(config: &LoopboxConfig) -> Result<(), String> {
    let settings = &config.global.resource_metrics;
    let old = {
        let mut state = resource_metrics_sampler_state()
            .lock()
            .map_err(|_| "Resource metrics sampler state lock poisoned.".to_string())?;
        if !settings.enabled {
            state.running.take()
        } else if state
            .running
            .as_ref()
            .is_some_and(|running| running.config == *config)
        {
            return Ok(());
        } else {
            state.running.take()
        }
    };
    stop_running_sampler(old);

    if !settings.enabled {
        return Ok(());
    }

    let sampler_config = config.clone();
    let interval = Duration::from_secs(settings.sample_interval_secs.clamp(2, 60));
    let retention_days = settings.retention_days.clamp(1, 90);
    let max_storage_mb = settings.max_storage_mb.clamp(25, 5_000);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread_config = sampler_config.clone();
    let thread = thread::Builder::new()
        .name("loopbox-resource-metrics".to_string())
        .spawn(move || {
            run_resource_metrics_sampler(
                thread_config,
                interval,
                retention_days,
                max_storage_mb,
                thread_stop,
            )
        })
        .map_err(|err| format!("Failed to start resource metrics sampler: {err}"))?;

    let mut state = resource_metrics_sampler_state()
        .lock()
        .map_err(|_| "Resource metrics sampler state lock poisoned.".to_string())?;
    state.running = Some(RunningResourceMetricsSampler {
        config: sampler_config,
        stop,
        thread: Some(thread),
    });
    Ok(())
}

pub fn resource_metrics_latest_for_config(
    config: &LoopboxConfig,
) -> Result<BTreeMap<String, ServiceResourceSample>, String> {
    let keys = configured_resource_sample_keys(config);
    let mut latest = load_latest_resource_samples_from_disk(&keys)?;
    if let Ok(store) = resource_metrics_store().lock() {
        for key in &keys {
            if let Some(sample) = store.latest.get(key) {
                if latest
                    .get(key)
                    .is_none_or(|existing| sample.sampled_at_unix_ms >= existing.sampled_at_unix_ms)
                {
                    latest.insert(key.clone(), sample.clone());
                }
            }
        }
    }
    Ok(latest.into_iter().collect())
}

pub fn resource_metrics_series_for_project(
    project_name: &str,
    service_filter: Option<&str>,
    window: &str,
    limit: usize,
) -> Result<Vec<ServiceResourceSample>, String> {
    let window_ms = resource_metrics_window_millis(window)
        .ok_or_else(|| "Resource metrics window must be one of 15m, 1h, 24h, or 7d.".to_string())?;
    let cutoff = current_unix_millis().saturating_sub(window_ms);
    let effective_limit = limit.clamp(1, RESOURCE_METRICS_MAX_SERIES_LIMIT);
    let normalized_service_filter = service_filter
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut samples = load_resource_samples_from_disk(
        project_name,
        normalized_service_filter,
        cutoff,
        effective_limit.saturating_mul(4),
    )?;
    if let Ok(store) = resource_metrics_store().lock() {
        for sample in store.latest.values() {
            if sample.project_name != project_name || sample.sampled_at_unix_ms < cutoff {
                continue;
            }
            if let Some(service_name) = normalized_service_filter {
                if sample.service_name != service_name {
                    continue;
                }
            }
            samples.push(sample.clone());
        }
    }

    let mut seen = HashSet::new();
    samples.retain(|sample| seen.insert(resource_sample_dedupe_key(sample)));
    samples.sort_by_key(|sample| sample.sampled_at_unix_ms);
    if samples.len() > effective_limit {
        samples = samples[samples.len() - effective_limit..].to_vec();
    }
    Ok(samples)
}

pub fn resource_metrics_disk_stats() -> ResourceMetricsDiskStats {
    let dropped_samples = resource_metrics_sampler_state()
        .lock()
        .map(|state| state.dropped_samples)
        .unwrap_or(0);
    let storage_dir = resource_metrics_dir();
    let (total_files, total_bytes) = resource_metrics_storage_totals(&storage_dir);
    ResourceMetricsDiskStats {
        dropped_samples,
        total_files,
        total_bytes,
    }
}

pub(crate) fn sanitize_resource_metrics_settings(settings: &mut ResourceMetricsSettings) {
    settings.sample_interval_secs = if settings.sample_interval_secs == 0 {
        default_resource_metrics_sample_interval_secs()
    } else {
        settings.sample_interval_secs.clamp(2, 60)
    };
    settings.retention_days = if settings.retention_days == 0 {
        default_resource_metrics_retention_days()
    } else {
        settings.retention_days.clamp(1, 90)
    };
    settings.max_storage_mb = if settings.max_storage_mb == 0 {
        default_resource_metrics_max_storage_mb()
    } else {
        settings.max_storage_mb.clamp(25, 5_000)
    };
}

pub(crate) fn resource_metrics_window_millis(window: &str) -> Option<u64> {
    match window.trim() {
        "15m" => Some(15 * 60 * 1000),
        "1h" => Some(60 * 60 * 1000),
        "24h" => Some(24 * 60 * 60 * 1000),
        "7d" => Some(7 * 24 * 60 * 60 * 1000),
        _ => None,
    }
}

fn run_resource_metrics_sampler(
    config: LoopboxConfig,
    interval: Duration,
    retention_days: u16,
    max_storage_mb: usize,
    stop: Arc<AtomicBool>,
) {
    let storage_dir = resource_metrics_dir();
    let _ = cleanup_resource_metrics_storage(&storage_dir, retention_days, max_storage_mb);
    let mut writes_since_cleanup = 0_usize;
    while !stop.load(Ordering::Relaxed) {
        let samples = collect_resource_samples(&config);
        for sample in samples {
            push_latest_resource_sample(sample.clone());
            if let Err(err) = append_resource_sample_to_disk(&sample) {
                if let Ok(mut state) = resource_metrics_sampler_state().lock() {
                    state.dropped_samples = state.dropped_samples.saturating_add(1);
                }
                eprintln!("Loopbox resource metrics persistence warning: {err}");
            } else {
                writes_since_cleanup = writes_since_cleanup.saturating_add(1);
            }
        }
        if writes_since_cleanup >= RESOURCE_METRICS_CLEANUP_WRITES {
            writes_since_cleanup = 0;
            let _ = cleanup_resource_metrics_storage(&storage_dir, retention_days, max_storage_mb);
        }
        sleep_until_next_sample(interval, &stop);
    }
}

fn stop_running_sampler(running: Option<RunningResourceMetricsSampler>) {
    let Some(mut running) = running else {
        return;
    };
    running.stop.store(true, Ordering::Relaxed);
    if let Some(thread) = running.thread.take() {
        let _ = thread.join();
    }
}

fn sleep_until_next_sample(interval: Duration, stop: &AtomicBool) {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < interval && !stop.load(Ordering::Relaxed) {
        let remaining = interval.saturating_sub(slept);
        let nap = remaining.min(step);
        thread::sleep(nap);
        slept = slept.saturating_add(nap);
    }
}

fn collect_resource_samples(config: &LoopboxConfig) -> Vec<ServiceResourceSample> {
    let mut samples = Vec::new();
    for (project_name, project) in &config.projects {
        for service in &project.services {
            let Ok(snapshot) = service_runtime_status(config, project_name, &service.name) else {
                continue;
            };
            if !resource_metrics_state_is_active(snapshot.state) {
                continue;
            }
            let sampled_at_unix_ms = current_unix_millis();
            let sampled_at_utc = format_unix_utc_millis(sampled_at_unix_ms);
            let sample = match service.runtime {
                ServiceRuntimeKind::Process => sample_process_service(
                    project_name,
                    &service.name,
                    snapshot.state,
                    snapshot.pid,
                    sampled_at_unix_ms,
                    sampled_at_utc,
                ),
                ServiceRuntimeKind::Container => sample_container_service(
                    project_name,
                    &service.name,
                    snapshot.state,
                    sampled_at_unix_ms,
                    sampled_at_utc,
                ),
            };
            samples.push(sample);
        }
    }
    samples
}

fn sample_process_service(
    project_name: &str,
    service_name: &str,
    state: ServiceRuntimeState,
    pid: Option<u32>,
    sampled_at_unix_ms: u64,
    sampled_at_utc: String,
) -> ServiceResourceSample {
    let Some(pid) = pid else {
        return unavailable_sample(
            project_name,
            service_name,
            ServiceRuntimeKind::Process,
            state,
            None,
            None,
            sampled_at_unix_ms,
            sampled_at_utc,
            "Process PID is unavailable.",
        );
    };

    match crate::platform::process::process_tree_resource_usage(pid) {
        Ok(usage) => ServiceResourceSample {
            project_name: project_name.to_string(),
            service_name: service_name.to_string(),
            sampled_at_unix_ms,
            sampled_at_utc,
            runtime: ServiceRuntimeKind::Process,
            state,
            pid: Some(pid),
            cpu_percent: Some(usage.cpu_percent),
            memory_bytes: Some(usage.memory_bytes),
            process_count: Some(usage.process_count),
            container_name: None,
            unavailable_reason: None,
        },
        Err(err) => unavailable_sample(
            project_name,
            service_name,
            ServiceRuntimeKind::Process,
            state,
            Some(pid),
            None,
            sampled_at_unix_ms,
            sampled_at_utc,
            &err,
        ),
    }
}

fn sample_container_service(
    project_name: &str,
    service_name: &str,
    state: ServiceRuntimeState,
    sampled_at_unix_ms: u64,
    sampled_at_utc: String,
) -> ServiceResourceSample {
    let container_name =
        super::internal::runtime_container::runtime_container_name(project_name, service_name);
    match super::internal::runtime_container::container_resource_stats(&container_name) {
        Ok(Some(stats)) => ServiceResourceSample {
            project_name: project_name.to_string(),
            service_name: service_name.to_string(),
            sampled_at_unix_ms,
            sampled_at_utc,
            runtime: ServiceRuntimeKind::Container,
            state,
            pid: None,
            cpu_percent: stats.cpu_percent,
            memory_bytes: stats.memory_bytes,
            process_count: stats.process_count,
            container_name: Some(container_name),
            unavailable_reason: None,
        },
        Ok(None) => unavailable_sample(
            project_name,
            service_name,
            ServiceRuntimeKind::Container,
            state,
            None,
            Some(container_name),
            sampled_at_unix_ms,
            sampled_at_utc,
            "Container is unavailable.",
        ),
        Err(err) => unavailable_sample(
            project_name,
            service_name,
            ServiceRuntimeKind::Container,
            state,
            None,
            Some(container_name),
            sampled_at_unix_ms,
            sampled_at_utc,
            &err,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn unavailable_sample(
    project_name: &str,
    service_name: &str,
    runtime: ServiceRuntimeKind,
    state: ServiceRuntimeState,
    pid: Option<u32>,
    container_name: Option<String>,
    sampled_at_unix_ms: u64,
    sampled_at_utc: String,
    reason: &str,
) -> ServiceResourceSample {
    ServiceResourceSample {
        project_name: project_name.to_string(),
        service_name: service_name.to_string(),
        sampled_at_unix_ms,
        sampled_at_utc,
        runtime,
        state,
        pid,
        cpu_percent: None,
        memory_bytes: None,
        process_count: None,
        container_name,
        unavailable_reason: Some(reason.to_string()),
    }
}

fn resource_metrics_state_is_active(state: ServiceRuntimeState) -> bool {
    matches!(
        state,
        ServiceRuntimeState::Starting
            | ServiceRuntimeState::Running
            | ServiceRuntimeState::Unhealthy
    )
}

fn push_latest_resource_sample(sample: ServiceResourceSample) {
    if let Ok(mut store) = resource_metrics_store().lock() {
        store.latest.insert(resource_sample_key(&sample), sample);
    }
}

fn append_resource_sample_to_disk(sample: &ServiceResourceSample) -> Result<(), String> {
    let day_key = sample_day_key(sample).unwrap_or_else(current_utc_day_key);
    let storage_dir = resource_metrics_dir();
    open_resource_metrics_jsonl_file(&storage_dir, &day_key).and_then(|mut file| {
        let line = serde_json::to_string(sample)
            .map_err(|err| format!("Failed to encode sample: {err}"))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|err| format!("Failed to write resource metrics sample: {err}"))
    })
}

fn load_latest_resource_samples_from_disk(
    keys: &HashSet<String>,
) -> Result<HashMap<String, ServiceResourceSample>, String> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut latest = HashMap::new();
    let storage_dir = resource_metrics_dir();
    let mut files = resource_metrics_sample_files(&storage_dir)?;
    files.sort_by_key(|file| std::cmp::Reverse(file.day_serial));

    for file in files {
        for sample in resource_samples_from_file(&file.path) {
            let key = resource_sample_key(&sample);
            if !keys.contains(&key) {
                continue;
            }
            if latest
                .get(&key)
                .is_none_or(|existing: &ServiceResourceSample| {
                    sample.sampled_at_unix_ms >= existing.sampled_at_unix_ms
                })
            {
                latest.insert(key, sample);
            }
        }
        if latest.len() == keys.len() {
            break;
        }
    }
    Ok(latest)
}

fn load_resource_samples_from_disk(
    project_name: &str,
    service_filter: Option<&str>,
    cutoff_unix_ms: u64,
    limit: usize,
) -> Result<Vec<ServiceResourceSample>, String> {
    let cutoff_day_serial = millis_to_epoch_days(cutoff_unix_ms);
    let mut samples = Vec::new();
    let storage_dir = resource_metrics_dir();
    let mut files = resource_metrics_sample_files(&storage_dir)?;
    files.retain(|file| file.day_serial >= cutoff_day_serial);
    files.sort_by_key(|file| file.day_serial);

    for file in files {
        for sample in resource_samples_from_file(&file.path) {
            if sample.project_name != project_name || sample.sampled_at_unix_ms < cutoff_unix_ms {
                continue;
            }
            if let Some(service_name) = service_filter {
                if sample.service_name != service_name {
                    continue;
                }
            }
            samples.push(sample);
        }
    }
    samples.sort_by_key(|sample| sample.sampled_at_unix_ms);
    if samples.len() > limit {
        samples = samples[samples.len() - limit..].to_vec();
    }
    Ok(samples)
}

fn resource_metrics_sample_files(
    storage_dir: &Path,
) -> Result<Vec<ResourceMetricsFileMeta>, String> {
    if !storage_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let entries = fs::read_dir(storage_dir).map_err(|err| {
        format!(
            "Failed to list resource metrics dir {}: {err}",
            storage_dir.display()
        )
    })?;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(day_serial) = parse_day_from_resource_metrics_filename(name) else {
            continue;
        };
        files.push(ResourceMetricsFileMeta {
            path,
            day_serial,
            size_bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
        });
    }
    Ok(files)
}

fn resource_samples_from_file(path: &Path) -> Vec<ServiceResourceSample> {
    let mut samples = Vec::new();
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return samples,
    };
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(sample) = serde_json::from_str::<ServiceResourceSample>(&line) {
            samples.push(sample);
        }
    }
    samples
}

fn open_resource_metrics_jsonl_file(storage_dir: &Path, day_key: &str) -> Result<File, String> {
    fs::create_dir_all(storage_dir).map_err(|err| {
        format!(
            "Failed to create resource metrics dir {}: {err}",
            storage_dir.display()
        )
    })?;
    let path = storage_dir.join(format!("samples-{day_key}.jsonl"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| {
            format!(
                "Failed to open resource metrics log {}: {err}",
                path.display()
            )
        })
}

fn resource_metrics_storage_totals(storage_dir: &Path) -> (usize, u64) {
    let Ok(entries) = fs::read_dir(storage_dir) else {
        return (0, 0);
    };
    let mut total_files = 0_usize;
    let mut total_bytes = 0_u64;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if parse_day_from_resource_metrics_filename(name).is_none() {
            continue;
        }
        total_files = total_files.saturating_add(1);
        total_bytes =
            total_bytes.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
    }
    (total_files, total_bytes)
}

fn cleanup_resource_metrics_storage(
    storage_dir: &Path,
    retention_days: u16,
    max_storage_mb: usize,
) -> Result<(), String> {
    if !storage_dir.exists() {
        return Ok(());
    }
    let max_storage_bytes = (max_storage_mb as u64).saturating_mul(1024 * 1024);
    let current_day_serial = current_utc_epoch_days();
    let cutoff_day_serial = current_day_serial.saturating_sub((retention_days as i64) - 1);
    let mut files = Vec::new();

    for file in resource_metrics_sample_files(storage_dir)? {
        if file.day_serial < cutoff_day_serial {
            let _ = fs::remove_file(&file.path);
            continue;
        }
        files.push(file);
    }

    files.sort_by_key(|file| file.day_serial);
    let mut total_size_bytes = files.iter().map(|file| file.size_bytes).sum::<u64>();
    while total_size_bytes > max_storage_bytes && files.len() > 1 {
        let oldest = files.remove(0);
        if fs::remove_file(&oldest.path).is_ok() {
            total_size_bytes = total_size_bytes.saturating_sub(oldest.size_bytes);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ResourceMetricsFileMeta {
    path: PathBuf,
    day_serial: i64,
    size_bytes: u64,
}

fn configured_resource_sample_keys(config: &LoopboxConfig) -> HashSet<String> {
    config
        .projects
        .iter()
        .flat_map(|(project_name, project)| {
            project
                .services
                .iter()
                .map(move |service| resource_key(project_name, &service.name))
        })
        .collect()
}

fn resource_sample_key(sample: &ServiceResourceSample) -> String {
    resource_key(&sample.project_name, &sample.service_name)
}

fn resource_key(project_name: &str, service_name: &str) -> String {
    format!("{project_name}::{service_name}")
}

fn resource_sample_dedupe_key(sample: &ServiceResourceSample) -> String {
    format!(
        "{}|{}|{}",
        sample.project_name, sample.service_name, sample.sampled_at_unix_ms
    )
}

fn resource_metrics_store() -> &'static Mutex<ResourceMetricsStore> {
    RESOURCE_METRICS_STORE.get_or_init(|| Mutex::new(ResourceMetricsStore::default()))
}

fn resource_metrics_sampler_state() -> &'static Mutex<ResourceMetricsSamplerState> {
    RESOURCE_METRICS_SAMPLER_STATE
        .get_or_init(|| Mutex::new(ResourceMetricsSamplerState::default()))
}

fn resource_metrics_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = resource_metrics_test_dir()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
    {
        return path;
    }

    let config_file = config_path();
    let base_dir = config_file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".loopbox"));
    base_dir.join("resource-metrics")
}

fn sample_day_key(sample: &ServiceResourceSample) -> Option<String> {
    if sample.sampled_at_utc.len() < 10 {
        return None;
    }
    let day_key = &sample.sampled_at_utc[..10];
    parse_day_key(day_key).map(|_| day_key.to_string())
}

fn parse_day_from_resource_metrics_filename(name: &str) -> Option<i64> {
    if !name.starts_with("samples-") || !name.ends_with(".jsonl") {
        return None;
    }
    let day_key = name.strip_prefix("samples-")?.strip_suffix(".jsonl")?;
    parse_day_key(day_key)
}

fn parse_day_key(day_key: &str) -> Option<i64> {
    let mut parts = day_key.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let day = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let day_serial = days_from_civil(year, month, day);
    let (check_year, check_month, check_day) = civil_from_days(day_serial);
    if (year, month, day) == (check_year, check_month, check_day) {
        Some(day_serial)
    } else {
        None
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn current_utc_day_key() -> String {
    let (year, month, day) = civil_from_days(current_utc_epoch_days());
    format!("{year:04}-{month:02}-{day:02}")
}

fn current_utc_epoch_days() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let epoch_seconds = i64::try_from(secs).unwrap_or(i64::MAX);
    epoch_seconds.div_euclid(86_400)
}

fn millis_to_epoch_days(epoch_ms: u64) -> i64 {
    let epoch_seconds = i64::try_from(epoch_ms / 1000).unwrap_or(i64::MAX);
    epoch_seconds.div_euclid(86_400)
}

fn format_unix_utc_millis(epoch_ms: u64) -> String {
    let epoch_seconds = i64::try_from(epoch_ms / 1000).unwrap_or(i64::MAX);
    let days = epoch_seconds.div_euclid(86_400);
    let day_remainder = epoch_seconds.rem_euclid(86_400);
    let hour = day_remainder / 3_600;
    let minute = (day_remainder % 3_600) / 60;
    let second = day_remainder % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn default_resource_metrics_enabled() -> bool {
    true
}

fn default_resource_metrics_sample_interval_secs() -> u64 {
    5
}

fn default_resource_metrics_retention_days() -> u16 {
    7
}

fn default_resource_metrics_max_storage_mb() -> usize {
    250
}

#[cfg(test)]
fn resource_metrics_test_dir() -> &'static Mutex<Option<PathBuf>> {
    RESOURCE_METRICS_TEST_DIR.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub(crate) fn resource_metrics_test_lock() -> &'static Mutex<()> {
    RESOURCE_METRICS_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn set_resource_metrics_test_dir(path: Option<PathBuf>) {
    if let Ok(mut guard) = resource_metrics_test_dir().lock() {
        *guard = path;
    }
}

#[cfg(test)]
fn clear_resource_metrics_for_test() {
    if let Ok(mut store) = resource_metrics_store().lock() {
        store.latest.clear();
    }
    if let Ok(mut state) = resource_metrics_sampler_state().lock() {
        state.dropped_samples = 0;
    }
    let dir = resource_metrics_dir();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
}

#[cfg(test)]
fn append_resource_sample_to_disk_for_test(sample: &ServiceResourceSample) -> Result<(), String> {
    append_resource_sample_to_disk(sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{ServiceRuntimeKind, ServiceRuntimeState};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn sample(project: &str, service: &str, sampled_at_unix_ms: u64) -> ServiceResourceSample {
        ServiceResourceSample {
            project_name: project.to_string(),
            service_name: service.to_string(),
            sampled_at_unix_ms,
            sampled_at_utc: format_unix_utc_millis(sampled_at_unix_ms),
            runtime: ServiceRuntimeKind::Process,
            state: ServiceRuntimeState::Running,
            pid: Some(123),
            cpu_percent: Some(12.5),
            memory_bytes: Some(42 * 1024 * 1024),
            process_count: Some(3),
            container_name: None,
            unavailable_reason: None,
        }
    }

    fn day_key_for_serial(day_serial: i64) -> String {
        let (year, month, day) = civil_from_days(day_serial);
        format!("{year:04}-{month:02}-{day:02}")
    }

    fn append_raw_line_to_day(dir: &Path, day_key: &str, line: &str) {
        fs::create_dir_all(dir).expect("create metrics dir");
        let path = dir.join(format!("samples-{day_key}.jsonl"));
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open sample file");
        file.write_all(line.as_bytes()).expect("write line");
        file.write_all(b"\n").expect("write newline");
    }

    fn append_sample_to_day(dir: &Path, day_key: &str, sample: &ServiceResourceSample) {
        let line = serde_json::to_string(sample).expect("serialize sample");
        append_raw_line_to_day(dir, day_key, &line);
    }

    #[test]
    fn resource_metrics_window_parser_accepts_supported_ranges() {
        assert_eq!(resource_metrics_window_millis("15m"), Some(15 * 60 * 1000));
        assert_eq!(resource_metrics_window_millis("1h"), Some(60 * 60 * 1000));
        assert_eq!(
            resource_metrics_window_millis("24h"),
            Some(24 * 60 * 60 * 1000)
        );
        assert_eq!(
            resource_metrics_window_millis("7d"),
            Some(7 * 24 * 60 * 60 * 1000)
        );
        assert_eq!(resource_metrics_window_millis("30d"), None);
    }

    #[test]
    fn persisted_resource_metrics_load_filtered_window_oldest_to_newest() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir = std::env::temp_dir().join(format!("loopbox-resource-metrics-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        let now = now_ms();
        append_resource_sample_to_disk_for_test(&sample("demo", "api", now - 50_000))
            .expect("write api sample");
        append_resource_sample_to_disk_for_test(&sample("demo", "web", now - 40_000))
            .expect("write first web sample");
        append_resource_sample_to_disk_for_test(&sample("demo", "web", now - 20_000))
            .expect("write second web sample");
        append_resource_sample_to_disk_for_test(&sample("demo", "web", now - 20 * 60 * 1000))
            .expect("write stale web sample");

        let series = resource_metrics_series_for_project("demo", Some("web"), "15m", 10)
            .expect("load series");

        assert_eq!(series.len(), 2);
        assert!(series[0].sampled_at_unix_ms < series[1].sampled_at_unix_ms);
        assert!(series.iter().all(|sample| sample.service_name == "web"));

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn latest_resource_metrics_read_newest_files_first_and_ignore_malformed_lines() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir =
            std::env::temp_dir().join(format!("loopbox-resource-metrics-latest-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        let now = now_ms();
        let today = day_key_for_serial(current_utc_epoch_days());
        let yesterday = day_key_for_serial(current_utc_epoch_days() - 1);
        let older = sample("demo", "web", now - 86_400_000);
        let newer = sample("demo", "web", now - 1_000);
        append_sample_to_day(&dir, &yesterday, &older);
        append_raw_line_to_day(&dir, &today, "{not-json");
        append_sample_to_day(&dir, &today, &newer);

        let keys = HashSet::from([resource_key("demo", "web")]);
        let latest = load_latest_resource_samples_from_disk(&keys).expect("latest samples");

        assert_eq!(
            latest
                .get(&resource_key("demo", "web"))
                .map(|sample| sample.sampled_at_unix_ms),
            Some(newer.sampled_at_unix_ms)
        );

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resource_metrics_disk_series_skips_files_older_than_cutoff_day() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir =
            std::env::temp_dir().join(format!("loopbox-resource-metrics-window-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        let now = now_ms();
        let today = day_key_for_serial(current_utc_epoch_days());
        let old_day = day_key_for_serial(current_utc_epoch_days() - 3);
        append_sample_to_day(&dir, &old_day, &sample("demo", "web", now - 3 * 86_400_000));
        append_sample_to_day(&dir, &today, &sample("demo", "web", now - 2_000));
        append_sample_to_day(&dir, &today, &sample("demo", "api", now - 1_000));

        let samples = load_resource_samples_from_disk("demo", None, now - 15 * 60 * 1000, 10)
            .expect("filtered samples");

        assert_eq!(samples.len(), 2);
        assert!(samples
            .iter()
            .all(|sample| sample.sampled_at_unix_ms >= now - 15 * 60 * 1000));
        assert!(samples[0].sampled_at_unix_ms <= samples[1].sampled_at_unix_ms);

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resource_metrics_series_dedupes_disk_and_memory_samples() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir =
            std::env::temp_dir().join(format!("loopbox-resource-metrics-dedupe-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        let duplicated = sample("demo", "web", now_ms() - 1_000);
        append_resource_sample_to_disk_for_test(&duplicated).expect("write disk sample");
        push_latest_resource_sample(duplicated);

        let samples = resource_metrics_series_for_project("demo", Some("web"), "15m", 10)
            .expect("deduped samples");

        assert_eq!(samples.len(), 1);

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_resource_metrics_storage_removes_expired_and_caps_to_newest_file() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir =
            std::env::temp_dir().join(format!("loopbox-resource-metrics-cleanup-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        let now = now_ms();
        let today_serial = current_utc_epoch_days();
        let today = day_key_for_serial(today_serial);
        let yesterday = day_key_for_serial(today_serial - 1);
        let expired = day_key_for_serial(today_serial - 10);
        append_sample_to_day(
            &dir,
            &expired,
            &sample("demo", "web", now - 10 * 86_400_000),
        );
        append_sample_to_day(&dir, &yesterday, &sample("demo", "web", now - 86_400_000));
        append_sample_to_day(&dir, &today, &sample("demo", "web", now));

        cleanup_resource_metrics_storage(&dir, 2, 0).expect("cleanup storage");

        assert!(!dir.join(format!("samples-{expired}.jsonl")).exists());
        assert!(!dir.join(format!("samples-{yesterday}.jsonl")).exists());
        assert!(dir.join(format!("samples-{today}.jsonl")).exists());

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resource_metrics_disk_stats_counts_sample_files() {
        let _guard = resource_metrics_test_lock().lock().expect("test lock");
        let dir = std::env::temp_dir().join(format!("loopbox-resource-metrics-stats-{}", now_ms()));
        fs::create_dir_all(&dir).expect("create test dir");
        set_resource_metrics_test_dir(Some(dir.clone()));
        clear_resource_metrics_for_test();

        append_resource_sample_to_disk_for_test(&sample("demo", "web", now_ms()))
            .expect("write sample");

        let stats = resource_metrics_disk_stats();

        assert_eq!(stats.total_files, 1);
        assert!(stats.total_bytes > 0);
        assert_eq!(stats.dropped_samples, 0);

        set_resource_metrics_test_dir(None);
        let _ = fs::remove_dir_all(dir);
    }
}
