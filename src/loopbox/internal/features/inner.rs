// Integrated feature implementation shared by the public app.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

#[path = "grpc.rs"]
mod grpc;

#[path = "agent_api_audit.rs"]
mod agent_api_audit;

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

pub use grpc::render_grpc_preview;

#[cfg(test)]
pub(crate) use test_support::{
    beautify_protoc_text_output_for_test, parse_day_from_traffic_filename_for_test,
    parse_day_key_for_test, proxy_event_to_har_entry_for_test, split_grpc_frames_for_test,
    GrpcFrameMetaForTest,
};
pub fn project_proxy_traffic_enabled(config: &super::LoopboxConfig, project_name: &str) -> bool {
    traffic::project_proxy_traffic_enabled(config, project_name)
}

pub fn project_proxy_traffic_capture_mode(
    config: &super::LoopboxConfig,
    project_name: &str,
) -> super::ProxyCaptureMode {
    traffic::project_proxy_traffic_capture_mode(config, project_name)
}

pub fn proxy_traffic_events_for_project_with_persisted(
    project_name: &str,
    service_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<super::ProxyTrafficEvent>, String> {
    traffic::proxy_traffic_events_for_project_with_persisted(project_name, service_filter, limit)
}

pub fn clear_proxy_traffic_events_for_project(project_name: &str) -> Result<usize, String> {
    traffic::clear_proxy_traffic_events_for_project(project_name)
}

pub fn proxy_traffic_disk_stats() -> super::ProxyTrafficDiskStats {
    traffic::proxy_traffic_disk_stats()
}

pub fn export_proxy_traffic_har_for_project(
    project_name: &str,
    service_filter: Option<&str>,
    output_path: &std::path::Path,
) -> Result<usize, String> {
    traffic::export_proxy_traffic_har_for_project(project_name, service_filter, output_path)
}

pub fn ensure_proxy_traffic_writer_running(
    queue_size: usize,
    retention_days: u16,
    max_storage_mb: usize,
) -> Result<(), String> {
    traffic::ensure_proxy_traffic_writer_running(queue_size, retention_days, max_storage_mb)
}

pub fn push_proxy_traffic_event(event: super::ProxyTrafficEvent, max_events: usize) {
    traffic::push_proxy_traffic_event(event, max_events);
}

pub fn agent_api_audit_events(limit: usize) -> Result<Vec<super::AgentApiAuditEvent>, String> {
    agent_api_audit::agent_api_audit_events(limit)
}

pub fn clear_agent_api_audit_events() -> Result<usize, String> {
    agent_api_audit::clear_agent_api_audit_events()
}

pub async fn run_agent_api_audit_middleware(
    auth_enabled: bool,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    agent_api_audit::run_agent_api_audit_middleware(auth_enabled, request, next).await
}

pub fn doctor_service_extra_issues(
    _config: &super::LoopboxConfig,
    project_name: &str,
    _project: &super::ProjectConfig,
    service: &super::ServiceConfig,
) -> Vec<super::DoctorIssue> {
    let mut issues = Vec::new();

    if matches!(service.runtime, super::ServiceRuntimeKind::Container)
        && service
            .container
            .as_ref()
            .is_none_or(|cfg| cfg.image.trim().is_empty())
    {
        issues.push(super::DoctorIssue {
            level: super::DoctorLevel::Error,
            project: Some(project_name.to_string()),
            message: format!(
                "Service '{}' runtime is container but container.image is empty.",
                service.name
            ),
            fix: None,
        });
    }

    issues
}

pub fn doctor_requires_start_command(service: &super::ServiceConfig) -> bool {
    matches!(service.runtime, super::ServiceRuntimeKind::Process)
}

pub fn doctor_global_extra_issues(config: &super::LoopboxConfig) -> Vec<super::DoctorIssue> {
    let docker_required = config.projects.values().any(|project| {
        project
            .services
            .iter()
            .any(|service| matches!(service.runtime, super::ServiceRuntimeKind::Container))
    });

    if docker_required {
        let Some(docker_message) = docker_unavailable_message() else {
            return Vec::new();
        };
        vec![super::DoctorIssue {
            level: super::DoctorLevel::Warning,
            project: None,
            message:
                format!("One or more services use runtime 'container', but Docker is unavailable. {docker_message}"),
            fix: None,
        }]
    } else {
        Vec::new()
    }
}

fn docker_unavailable_message() -> Option<String> {
    crate::loopbox::internal::runtime_container::docker_runtime_unavailable_message(
        &crate::loopbox::internal::runtime_container::docker_runtime_status(),
    )
}

mod traffic {
    use super::super::{
        config_path, enforce_traffic_capture_mode, LoopboxConfig, ProxyCaptureMode,
        ProxyTrafficDiskStats, ProxyTrafficEvent, ProxyTrafficHeader,
    };
    use std::collections::{HashSet, VecDeque};
    use std::fs::{self, File, OpenOptions};
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, SyncSender, TrySendError};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    const DEFAULT_PROXY_TRAFFIC_WRITER_QUEUE_SIZE: usize = 10_000;

    #[derive(Debug, Default)]
    struct ProxyTrafficStore {
        next_id: u64,
        events: VecDeque<ProxyTrafficEvent>,
    }

    #[derive(Debug)]
    struct ProxyTrafficWriterState {
        queue_size: usize,
        retention_days: u16,
        max_storage_mb: usize,
        sender: Option<SyncSender<ProxyTrafficEvent>>,
        dropped_events: u64,
    }

    impl Default for ProxyTrafficWriterState {
        fn default() -> Self {
            Self {
                queue_size: DEFAULT_PROXY_TRAFFIC_WRITER_QUEUE_SIZE,
                retention_days: 7,
                max_storage_mb: 500,
                sender: None,
                dropped_events: 0,
            }
        }
    }

    static PROXY_TRAFFIC_STORE: OnceLock<Mutex<ProxyTrafficStore>> = OnceLock::new();
    static PROXY_TRAFFIC_WRITER_STATE: OnceLock<Mutex<ProxyTrafficWriterState>> = OnceLock::new();
    #[cfg(test)]
    static PROXY_TRAFFIC_TEST_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    #[cfg(test)]
    static PROXY_TRAFFIC_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn proxy_traffic_store() -> &'static Mutex<ProxyTrafficStore> {
        PROXY_TRAFFIC_STORE.get_or_init(|| Mutex::new(ProxyTrafficStore::default()))
    }

    fn proxy_traffic_writer_state() -> &'static Mutex<ProxyTrafficWriterState> {
        PROXY_TRAFFIC_WRITER_STATE.get_or_init(|| Mutex::new(ProxyTrafficWriterState::default()))
    }

    pub(super) fn project_proxy_traffic_enabled(
        config: &LoopboxConfig,
        project_name: &str,
    ) -> bool {
        config
            .projects
            .get(project_name)
            .map(|project| {
                project
                    .proxy_traffic_capture_enabled
                    .unwrap_or(config.global.proxy_traffic.capture_enabled_by_default)
            })
            .unwrap_or(false)
    }

    pub(super) fn project_proxy_traffic_capture_mode(
        config: &LoopboxConfig,
        project_name: &str,
    ) -> ProxyCaptureMode {
        let selected = config
            .projects
            .get(project_name)
            .and_then(|project| project.proxy_traffic_capture_mode.clone())
            .unwrap_or_else(|| config.global.proxy_traffic.capture_mode_default.clone());
        enforce_traffic_capture_mode(selected)
    }

    pub(super) fn proxy_traffic_events_for_project(
        project_name: &str,
        limit: usize,
    ) -> Result<Vec<ProxyTrafficEvent>, String> {
        let store = proxy_traffic_store()
            .lock()
            .map_err(|_| "Proxy traffic store lock poisoned.".to_string())?;
        let mut events = Vec::with_capacity(limit.min(store.events.len()));
        for event in store.events.iter().rev() {
            if event.project_name != project_name {
                continue;
            }
            events.push(event.clone());
            if events.len() >= limit {
                break;
            }
        }
        Ok(events)
    }

    pub(super) fn proxy_traffic_events_for_project_with_persisted(
        project_name: &str,
        service_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ProxyTrafficEvent>, String> {
        let normalized_service_filter = service_filter
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());

        let in_memory_events = proxy_traffic_events_for_project(project_name, limit)?;
        let persisted_events = load_proxy_traffic_events_from_disk(
            project_name,
            normalized_service_filter,
            limit.saturating_mul(4),
        )?;

        let mut seen = HashSet::new();
        let mut merged = Vec::with_capacity(limit);
        for event in in_memory_events {
            if let Some(service_name) = normalized_service_filter {
                if event.service_name != service_name {
                    continue;
                }
            }
            let key = proxy_traffic_dedupe_key(&event);
            if seen.insert(key) {
                merged.push(event);
                if merged.len() >= limit {
                    return Ok(merged);
                }
            }
        }

        for event in persisted_events {
            let key = proxy_traffic_dedupe_key(&event);
            if seen.insert(key) {
                merged.push(event);
                if merged.len() >= limit {
                    break;
                }
            }
        }

        Ok(merged)
    }

    pub(super) fn clear_proxy_traffic_events_for_project(
        project_name: &str,
    ) -> Result<usize, String> {
        let in_memory_removed = {
            let mut store = proxy_traffic_store()
                .lock()
                .map_err(|_| "Proxy traffic store lock poisoned.".to_string())?;
            let before = store.events.len();
            store
                .events
                .retain(|event| event.project_name != project_name);
            before.saturating_sub(store.events.len())
        };
        let disk_removed = clear_proxy_traffic_disk_for_project(project_name)?;
        Ok(in_memory_removed.saturating_add(disk_removed))
    }

    pub(super) fn proxy_traffic_disk_stats() -> ProxyTrafficDiskStats {
        let (dropped_events, _queue_size) = proxy_traffic_writer_state()
            .lock()
            .map(|state| (state.dropped_events, state.queue_size))
            .unwrap_or((0, DEFAULT_PROXY_TRAFFIC_WRITER_QUEUE_SIZE));
        let storage_dir = proxy_traffic_dir();
        let (total_files, total_bytes) = proxy_traffic_storage_totals(&storage_dir);
        ProxyTrafficDiskStats {
            dropped_events,
            total_files,
            total_bytes,
        }
    }

    pub(super) fn export_proxy_traffic_har_for_project(
        project_name: &str,
        service_filter: Option<&str>,
        output_path: &Path,
    ) -> Result<usize, String> {
        let events =
            proxy_traffic_events_for_project_with_persisted(project_name, service_filter, 100_000)?;

        let entries = events
            .iter()
            .map(proxy_event_to_har_entry)
            .collect::<Vec<serde_json::Value>>();
        let payload = serde_json::json!({
            "log": {
                "version": "1.2",
                "creator": {
                    "name": "Loopbox",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "entries": entries
            }
        });
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!("Failed to create HAR directory {}: {err}", parent.display())
            })?;
        }
        let har_json = serde_json::to_string_pretty(&payload)
            .map_err(|err| format!("Failed to encode HAR payload: {err}"))?;
        fs::write(output_path, har_json)
            .map_err(|err| format!("Failed to write HAR file {}: {err}", output_path.display()))?;
        Ok(events.len())
    }

    pub(super) fn ensure_proxy_traffic_writer_running(
        queue_size: usize,
        retention_days: u16,
        max_storage_mb: usize,
    ) -> Result<(), String> {
        let mut state = proxy_traffic_writer_state()
            .lock()
            .map_err(|_| "Proxy traffic writer state lock poisoned.".to_string())?;
        let needs_restart = state.sender.is_none()
            || state.queue_size != queue_size
            || state.retention_days != retention_days
            || state.max_storage_mb != max_storage_mb;
        if !needs_restart {
            return Ok(());
        }

        let storage_dir = proxy_traffic_dir();
        let (sender, receiver) = mpsc::sync_channel::<ProxyTrafficEvent>(queue_size);
        std::thread::spawn(move || {
            run_proxy_traffic_writer(receiver, storage_dir, retention_days, max_storage_mb)
        });
        state.queue_size = queue_size;
        state.retention_days = retention_days;
        state.max_storage_mb = max_storage_mb;
        state.sender = Some(sender);
        state.dropped_events = 0;
        Ok(())
    }

    pub(super) fn push_proxy_traffic_event(mut event: ProxyTrafficEvent, max_events: usize) {
        let Ok(mut store) = proxy_traffic_store().lock() else {
            return;
        };
        store.next_id = store.next_id.wrapping_add(1);
        event.id = store.next_id;
        store.events.push_back(event);
        while store.events.len() > max_events {
            store.events.pop_front();
        }
        if let Some(persisted_event) = store.events.back() {
            enqueue_proxy_traffic_event_for_disk(persisted_event);
        }
    }

    fn enqueue_proxy_traffic_event_for_disk(event: &ProxyTrafficEvent) {
        let Ok(mut state) = proxy_traffic_writer_state().lock() else {
            return;
        };
        let Some(sender) = state.sender.as_ref() else {
            return;
        };
        match sender.try_send(event.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                state.dropped_events = state.dropped_events.saturating_add(1);
                if state.dropped_events == 1 || state.dropped_events % 100 == 0 {
                    eprintln!(
                        "Loopbox proxy traffic writer queue full. Dropped {} event(s).",
                        state.dropped_events
                    );
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                state.sender = None;
            }
        }
    }

    fn run_proxy_traffic_writer(
        receiver: mpsc::Receiver<ProxyTrafficEvent>,
        storage_dir: PathBuf,
        retention_days: u16,
        max_storage_mb: usize,
    ) {
        let mut file = None::<File>;
        let mut active_day = String::new();
        let mut writes_since_cleanup = 0_usize;
        let _ = cleanup_proxy_traffic_storage(&storage_dir, retention_days, max_storage_mb);
        for event in receiver {
            let day_key = event_day_key(&event.started_at_utc).unwrap_or_else(current_utc_day_key);
            if file.is_none() || day_key != active_day {
                file = open_proxy_traffic_jsonl_file(&storage_dir, &day_key).ok();
                active_day = day_key;
            }
            let Some(active_file) = file.as_mut() else {
                continue;
            };
            let Ok(line) = serde_json::to_string(&event) else {
                continue;
            };
            if active_file.write_all(line.as_bytes()).is_err()
                || active_file.write_all(b"\n").is_err()
            {
                file = None;
                continue;
            }
            writes_since_cleanup = writes_since_cleanup.saturating_add(1);
            if writes_since_cleanup >= 200 {
                writes_since_cleanup = 0;
                let _ = cleanup_proxy_traffic_storage(&storage_dir, retention_days, max_storage_mb);
            }
        }
    }

    fn proxy_traffic_dir() -> PathBuf {
        #[cfg(test)]
        if let Some(path) = proxy_traffic_test_dir()
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
        base_dir.join("traffic")
    }

    #[cfg(test)]
    fn proxy_traffic_test_dir() -> &'static Mutex<Option<PathBuf>> {
        PROXY_TRAFFIC_TEST_DIR.get_or_init(|| Mutex::new(None))
    }

    #[cfg(test)]
    fn proxy_traffic_test_lock() -> &'static Mutex<()> {
        PROXY_TRAFFIC_TEST_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn load_proxy_traffic_events_from_disk(
        project_name: &str,
        service_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ProxyTrafficEvent>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let storage_dir = proxy_traffic_dir();
        if !storage_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let entries = fs::read_dir(&storage_dir).map_err(|err| {
            format!(
                "Failed to list proxy traffic dir {}: {err}",
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
            let Some(day_serial) = parse_day_from_traffic_filename(name) else {
                continue;
            };
            files.push((day_serial, path));
        }

        files.sort_by(|(a, _), (b, _)| b.cmp(a));

        let mut collected = Vec::new();
        for (_, path) in files {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let reader = BufReader::new(file);
            let mut day_events = Vec::new();
            for line in reader.lines() {
                let Ok(line) = line else {
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(event) = serde_json::from_str::<ProxyTrafficEvent>(&line) else {
                    continue;
                };
                if event.project_name != project_name {
                    continue;
                }
                if let Some(service_name) = service_filter {
                    if event.service_name != service_name {
                        continue;
                    }
                }
                day_events.push(event);
            }

            day_events.reverse();
            for event in day_events {
                collected.push(event);
                if collected.len() >= limit {
                    return Ok(collected);
                }
            }
        }

        Ok(collected)
    }

    fn proxy_traffic_dedupe_key(event: &ProxyTrafficEvent) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            event.id,
            event.started_at_utc,
            event.project_name,
            event.service_name,
            event.method,
            event.path,
            event.status_code.unwrap_or(0)
        )
    }

    fn clear_proxy_traffic_disk_for_project(project_name: &str) -> Result<usize, String> {
        let storage_dir = proxy_traffic_dir();
        if !storage_dir.exists() {
            return Ok(0);
        }

        let mut removed = 0_usize;
        let entries = fs::read_dir(&storage_dir).map_err(|err| {
            format!(
                "Failed to list proxy traffic dir {}: {err}",
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
            if parse_day_from_traffic_filename(name).is_none() {
                continue;
            }

            let file = File::open(&path)
                .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
            let mut kept = Vec::new();
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else {
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<ProxyTrafficEvent>(&line) {
                    Ok(event) if event.project_name == project_name => {
                        removed = removed.saturating_add(1);
                    }
                    Ok(_) | Err(_) => kept.push(line),
                }
            }

            if kept.is_empty() {
                fs::remove_file(&path)
                    .map_err(|err| format!("Failed to remove {}: {err}", path.display()))?;
            } else {
                fs::write(&path, format!("{}\n", kept.join("\n")))
                    .map_err(|err| format!("Failed to rewrite {}: {err}", path.display()))?;
            }
        }

        Ok(removed)
    }

    fn open_proxy_traffic_jsonl_file(storage_dir: &PathBuf, day_key: &str) -> Result<File, String> {
        fs::create_dir_all(storage_dir).map_err(|err| {
            format!(
                "Failed to create proxy traffic dir {}: {err}",
                storage_dir.display()
            )
        })?;
        let path = storage_dir.join(format!("events-{day_key}.jsonl"));
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| format!("Failed to open proxy traffic log {}: {err}", path.display()))
    }

    fn proxy_traffic_storage_totals(storage_dir: &PathBuf) -> (usize, u64) {
        let Ok(entries) = fs::read_dir(storage_dir) else {
            return (0, 0);
        };
        let mut file_count = 0_usize;
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
            if parse_day_from_traffic_filename(name).is_none() {
                continue;
            }
            file_count = file_count.saturating_add(1);
            total_bytes =
                total_bytes.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
        }
        (file_count, total_bytes)
    }

    pub(super) fn proxy_event_to_har_entry(event: &ProxyTrafficEvent) -> serde_json::Value {
        let request_url = har_url_for_event(event);
        let request_headers = har_headers_from_proxy(&event.request_headers);
        let response_headers = har_headers_from_proxy(&event.response_headers);
        let request_query = har_query_from_path(&event.path);
        let request_mime = har_mime_type_from_headers(&event.request_headers);
        let response_mime = har_mime_type_from_headers(&event.response_headers);
        let request_post_data = event.request_body_preview.as_ref().map(|preview| {
            serde_json::json!({
                "mimeType": request_mime,
                "text": preview,
            })
        });
        let response_content = if let Some(preview) = event.response_body_preview.as_ref() {
            serde_json::json!({
                "size": event.response_body_bytes,
                "mimeType": response_mime,
                "text": preview,
            })
        } else {
            serde_json::json!({
                "size": event.response_body_bytes,
                "mimeType": response_mime,
            })
        };

        serde_json::json!({
            "startedDateTime": har_started_at_iso8601(&event.started_at_utc),
            "time": event.duration_ms,
            "request": {
                "method": event.method,
                "url": request_url,
                "httpVersion": "HTTP/1.1",
                "headers": request_headers,
                "queryString": request_query,
                "headersSize": event.request_header_bytes,
                "bodySize": event.request_body_bytes,
                "postData": request_post_data,
            },
            "response": {
                "status": event.status_code.unwrap_or(0),
                "statusText": "",
                "httpVersion": "HTTP/1.1",
                "headers": response_headers,
                "content": response_content,
                "redirectURL": "",
                "headersSize": event.response_header_bytes,
                "bodySize": event.response_body_bytes,
            },
            "cache": {},
            "timings": {
                "send": 0,
                "wait": event.duration_ms,
                "receive": 0,
            },
            "_loopbox": {
                "project": event.project_name,
                "service": event.service_name,
                "error": event.error,
                "request_body_truncated": event.request_body_truncated,
                "response_body_truncated": event.response_body_truncated,
                "request_body_binary": event.request_body_binary,
                "response_body_binary": event.response_body_binary,
                "request_header_bytes": event.request_header_bytes,
                "request_body_bytes": event.request_body_bytes,
                "response_header_bytes": event.response_header_bytes,
                "response_body_bytes": event.response_body_bytes,
            }
        })
    }

    fn har_headers_from_proxy(headers: &[ProxyTrafficHeader]) -> Vec<serde_json::Value> {
        headers
            .iter()
            .map(|header| {
                serde_json::json!({
                    "name": header.name,
                    "value": header.value,
                })
            })
            .collect()
    }

    fn har_query_from_path(path: &str) -> Vec<serde_json::Value> {
        let Some((_, query)) = path.split_once('?') else {
            return Vec::new();
        };
        if query.is_empty() {
            return Vec::new();
        }
        query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                serde_json::json!({
                    "name": name,
                    "value": value,
                })
            })
            .collect()
    }

    fn har_mime_type_from_headers(headers: &[ProxyTrafficHeader]) -> String {
        headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case("content-type"))
            .map(|header| header.value.clone())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    }

    fn har_started_at_iso8601(raw: &str) -> String {
        if let Some(stripped) = raw.strip_suffix(" UTC") {
            return format!("{}Z", stripped.replace(' ', "T"));
        }
        raw.to_string()
    }

    fn har_url_for_event(event: &ProxyTrafficEvent) -> String {
        if event.path.starts_with("http://") || event.path.starts_with("https://") {
            return event.path.clone();
        }
        format!("http://{}{}", event.host, event.path)
    }

    fn cleanup_proxy_traffic_storage(
        storage_dir: &PathBuf,
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
        let entries = fs::read_dir(storage_dir).map_err(|err| {
            format!(
                "Failed to list proxy traffic dir {}: {err}",
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
            let Some(day_serial) = parse_day_from_traffic_filename(name) else {
                continue;
            };
            if day_serial < cutoff_day_serial {
                let _ = fs::remove_file(&path);
                continue;
            }
            let size_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            files.push(TrafficFileMeta {
                path,
                day_serial,
                size_bytes,
            });
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
    struct TrafficFileMeta {
        path: PathBuf,
        day_serial: i64,
        size_bytes: u64,
    }

    fn event_day_key(timestamp: &str) -> Option<String> {
        if timestamp.len() < 10 {
            return None;
        }
        let day_key = &timestamp[..10];
        parse_day_key(day_key).map(|_| day_key.to_string())
    }

    pub(super) fn parse_day_from_traffic_filename(name: &str) -> Option<i64> {
        if !name.starts_with("events-") || !name.ends_with(".jsonl") {
            return None;
        }
        let day_key = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
        parse_day_key(day_key)
    }

    pub(super) fn parse_day_key(day_key: &str) -> Option<i64> {
        let mut parts = day_key.split('-');
        let year = parts.next()?.parse::<i64>().ok()?;
        let month = parts.next()?.parse::<i64>().ok()?;
        let day = parts.next()?.parse::<i64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let day_serial = days_from_civil(year, month, day);
        let (check_year, check_month, check_day) = civil_from_days(day_serial);
        if (year, month, day) != (check_year, check_month, check_day) {
            return None;
        }
        Some(day_serial)
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
        let adjusted_year = year - if month <= 2 { 1 } else { 0 };
        let era = if adjusted_year >= 0 {
            adjusted_year
        } else {
            adjusted_year - 399
        } / 400;
        let year_of_era = adjusted_year - era * 400;
        let month_prime = month + if month > 2 { -3 } else { 9 };
        let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct TrafficTestDir {
            path: PathBuf,
            _guard: std::sync::MutexGuard<'static, ()>,
        }

        impl Drop for TrafficTestDir {
            fn drop(&mut self) {
                if let Ok(mut dir) = proxy_traffic_test_dir().lock() {
                    *dir = None;
                }
                let _ = fs::remove_dir_all(&self.path);
            }
        }

        fn traffic_test_dir(name: &str) -> TrafficTestDir {
            let guard = proxy_traffic_test_lock().lock().expect("traffic test lock");
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "loopbox-traffic-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create traffic test dir");
            *proxy_traffic_test_dir()
                .lock()
                .expect("traffic test dir lock") = Some(path.clone());
            TrafficTestDir {
                path,
                _guard: guard,
            }
        }

        fn append_proxy_traffic_event_to_disk_for_test(
            event: &ProxyTrafficEvent,
        ) -> Result<(), String> {
            let day_key = event_day_key(&event.started_at_utc)
                .ok_or_else(|| "missing day key".to_string())?;
            let mut file = open_proxy_traffic_jsonl_file(&proxy_traffic_dir(), &day_key)?;
            let line = serde_json::to_string(event)
                .map_err(|err| format!("Failed to encode test event: {err}"))?;
            writeln!(file, "{line}").map_err(|err| format!("Failed to write test event: {err}"))
        }

        fn sample_proxy_event(project: &str, service: &str, path: &str) -> ProxyTrafficEvent {
            ProxyTrafficEvent {
                id: 0,
                started_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
                project_name: project.to_string(),
                service_name: service.to_string(),
                protocol: "http1".to_string(),
                host: format!("{service}.{project}.localhost"),
                method: "GET".to_string(),
                path: path.to_string(),
                status_code: Some(200),
                stream_id: None,
                grpc_service: None,
                grpc_method: None,
                grpc_status: None,
                grpc_message: None,
                duration_ms: 3,
                request_bytes: 24,
                response_bytes: 42,
                request_header_bytes: 24,
                request_body_bytes: 0,
                response_header_bytes: 36,
                response_body_bytes: 6,
                request_headers: Vec::new(),
                response_headers: Vec::new(),
                request_body_preview: None,
                response_body_preview: Some("hello".to_string()),
                request_body_truncated: false,
                response_body_truncated: false,
                request_body_binary: false,
                response_body_binary: false,
                error: None,
            }
        }

        #[test]
        fn proxy_traffic_with_persisted_merges_disk_and_memory() {
            let _scope = traffic_test_dir("merge");
            let project = "persist-demo";
            clear_proxy_traffic_events_for_project(project)
                .expect("clear existing project traffic");

            push_proxy_traffic_event(sample_proxy_event(project, "web", "/memory"), 100);
            append_proxy_traffic_event_to_disk_for_test(&sample_proxy_event(
                project, "web", "/disk",
            ))
            .expect("write persisted event");

            let events = proxy_traffic_events_for_project_with_persisted(project, Some("web"), 20)
                .expect("load merged traffic");

            assert!(events.iter().any(|event| event.path == "/memory"));
            assert!(events.iter().any(|event| event.path == "/disk"));
        }

        #[test]
        fn clear_proxy_traffic_removes_only_selected_project_from_disk() {
            let _scope = traffic_test_dir("clear");
            clear_proxy_traffic_events_for_project("demo-a").expect("clear demo-a");
            clear_proxy_traffic_events_for_project("demo-b").expect("clear demo-b");
            append_proxy_traffic_event_to_disk_for_test(&sample_proxy_event("demo-a", "web", "/a"))
                .expect("write demo-a");
            append_proxy_traffic_event_to_disk_for_test(&sample_proxy_event("demo-b", "web", "/b"))
                .expect("write demo-b");

            let removed = clear_proxy_traffic_events_for_project("demo-a").expect("clear demo-a");

            assert_eq!(removed, 1);
            assert!(
                proxy_traffic_events_for_project_with_persisted("demo-a", None, 20)
                    .expect("load demo-a")
                    .is_empty()
            );
            assert_eq!(
                proxy_traffic_events_for_project_with_persisted("demo-b", None, 20)
                    .expect("load demo-b")
                    .len(),
                1
            );
        }

        #[test]
        fn har_export_includes_persisted_events() {
            let scope = traffic_test_dir("har");
            let output_path = scope.path.join("export.har");
            append_proxy_traffic_event_to_disk_for_test(&sample_proxy_event(
                "har-demo",
                "web",
                "/persisted",
            ))
            .expect("write persisted event");

            let count = export_proxy_traffic_har_for_project("har-demo", Some("web"), &output_path)
                .expect("export har");

            let exported = fs::read_to_string(output_path).expect("read exported har");
            assert_eq!(count, 1);
            assert!(exported.contains("/persisted"));
        }
    }
}
