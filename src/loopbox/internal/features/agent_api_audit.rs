use super::super::{
    config_path, AgentApiAuditBodyEncoding, AgentApiAuditEvent, AgentApiAuditHeader,
};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_AGENT_API_AUDIT_LIMIT: usize = 200;
const MAX_AGENT_API_AUDIT_LIMIT: usize = 5_000;
const MAX_AGENT_API_AUDIT_EVENTS: usize = 20_000;
const AGENT_API_AUDIT_BODY_CAPTURE_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
struct AgentApiAuditStore {
    next_id: u64,
    events: VecDeque<AgentApiAuditEvent>,
}

static AGENT_API_AUDIT_STORE: OnceLock<Mutex<AgentApiAuditStore>> = OnceLock::new();

fn agent_api_audit_store() -> &'static Mutex<AgentApiAuditStore> {
    AGENT_API_AUDIT_STORE.get_or_init(|| Mutex::new(AgentApiAuditStore::default()))
}

fn supports_agent_api_audit() -> bool {
    matches!(
        crate::loopbox::internal::license::current_license_tier(),
        crate::loopbox::internal::license::LicenseTier::Commercial
    )
}

fn normalize_agent_api_audit_limit(limit: usize) -> usize {
    if limit == 0 {
        return DEFAULT_AGENT_API_AUDIT_LIMIT;
    }

    limit.clamp(1, MAX_AGENT_API_AUDIT_LIMIT)
}

pub(super) async fn run_agent_api_audit_middleware(
    auth_enabled: bool,
    request: Request,
    next: Next,
) -> Response {
    if !supports_agent_api_audit() {
        return next.run(request).await;
    }

    let started_at_unix_ms = unix_timestamp_ms_now();
    let started = std::time::Instant::now();

    let (request_parts, request_body) = request.into_parts();
    let method = request_parts.method.to_string();
    let path = request_parts.uri.path().to_string();
    let query = request_parts.uri.query().map(|value| value.to_string());
    let matched_path = request_parts
        .extensions
        .get::<axum::extract::MatchedPath>()
        .map(|value| value.as_str().to_string());
    let request_version = http_version_label(request_parts.version).to_string();
    let request_headers = headers_for_audit(&request_parts.headers);
    let authorization_header_present = request_parts
        .headers
        .contains_key(axum::http::header::AUTHORIZATION);
    let request_body_bytes = match axum::body::to_bytes(request_body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("Loopbox agent API audit capture warning (request body): {err}");
            bytes::Bytes::new()
        }
    };
    let request_body_bytes_len = request_body_bytes.len();
    let (request_body, request_body_encoding, request_body_truncated) =
        body_snapshot_for_audit(&request_body_bytes);

    let request = Request::from_parts(request_parts, axum::body::Body::from(request_body_bytes));
    let response = next.run(request).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let (response_parts, response_body) = response.into_parts();
    let response_version = http_version_label(response_parts.version).to_string();
    let status_code = response_parts.status.as_u16();
    let response_headers = headers_for_audit(&response_parts.headers);
    let response_body_bytes = match axum::body::to_bytes(response_body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("Loopbox agent API audit capture warning (response body): {err}");
            bytes::Bytes::new()
        }
    };
    let response_body_bytes_len = response_body_bytes.len();
    let (response_body, response_body_encoding, response_body_truncated) =
        body_snapshot_for_audit(&response_body_bytes);

    push_agent_api_audit_event(
        AgentApiAuditEvent {
            id: 0,
            started_at_unix_ms,
            duration_ms,
            method,
            path,
            query,
            matched_path,
            request_version,
            response_version,
            status_code,
            auth_enabled,
            authorization_header_present,
            request_headers,
            request_body,
            request_body_encoding,
            request_body_truncated,
            request_body_bytes: request_body_bytes_len,
            response_headers,
            response_body,
            response_body_encoding,
            response_body_truncated,
            response_body_bytes: response_body_bytes_len,
        },
        MAX_AGENT_API_AUDIT_EVENTS,
    );

    Response::from_parts(response_parts, axum::body::Body::from(response_body_bytes))
}

pub(super) fn agent_api_audit_events(limit: usize) -> Result<Vec<AgentApiAuditEvent>, String> {
    if !supports_agent_api_audit() {
        return Ok(Vec::new());
    }
    let limit = normalize_agent_api_audit_limit(limit);

    let in_memory = {
        let store = agent_api_audit_store()
            .lock()
            .map_err(|_| "Agent API audit store lock poisoned.".to_string())?;
        let mut events = Vec::with_capacity(limit.min(store.events.len()));
        for event in store.events.iter().rev() {
            events.push(event.clone());
            if events.len() >= limit {
                break;
            }
        }
        events
    };

    let persisted = load_agent_api_audit_events_from_disk(limit.saturating_mul(4))?;
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(limit);

    for event in in_memory.into_iter().chain(persisted) {
        let key = agent_api_audit_dedupe_key(&event);
        if seen.insert(key) {
            merged.push(event);
            if merged.len() >= limit {
                break;
            }
        }
    }

    Ok(merged)
}

pub(super) fn clear_agent_api_audit_events() -> Result<usize, String> {
    if !supports_agent_api_audit() {
        return Ok(0);
    }

    let in_memory_removed = {
        let mut store = agent_api_audit_store()
            .lock()
            .map_err(|_| "Agent API audit store lock poisoned.".to_string())?;
        let removed = store.events.len();
        store.events.clear();
        removed
    };

    let disk_removed = count_agent_api_audit_events_on_disk();
    clear_agent_api_audit_disk()?;

    Ok(in_memory_removed.saturating_add(disk_removed))
}

pub(super) fn push_agent_api_audit_event(mut event: AgentApiAuditEvent, max_events: usize) {
    if !supports_agent_api_audit() || max_events == 0 {
        return;
    }

    {
        let Ok(mut store) = agent_api_audit_store().lock() else {
            return;
        };

        if store.next_id == 0 {
            store.next_id = max_agent_api_audit_event_id_on_disk().unwrap_or(0);
        }
        store.next_id = store.next_id.wrapping_add(1);
        event.id = store.next_id;
        store.events.push_back(event.clone());

        while store.events.len() > max_events {
            store.events.pop_front();
        }
    }

    if let Err(err) = append_agent_api_audit_event_to_disk(&event) {
        eprintln!("Loopbox failed to persist agent API audit event: {err}");
    }
}

fn body_snapshot_for_audit(bytes: &[u8]) -> (String, AgentApiAuditBodyEncoding, bool) {
    let truncated = bytes.len() > AGENT_API_AUDIT_BODY_CAPTURE_MAX_BYTES;
    let slice = if truncated {
        &bytes[..AGENT_API_AUDIT_BODY_CAPTURE_MAX_BYTES]
    } else {
        bytes
    };

    if slice.is_empty() {
        return (String::new(), AgentApiAuditBodyEncoding::Utf8, truncated);
    }

    match std::str::from_utf8(slice) {
        Ok(text) => (text.to_string(), AgentApiAuditBodyEncoding::Utf8, truncated),
        Err(_) => (hex_encode(slice), AgentApiAuditBodyEncoding::Hex, truncated),
    }
}

fn headers_for_audit(headers: &axum::http::HeaderMap) -> Vec<AgentApiAuditHeader> {
    headers
        .iter()
        .map(|(name, value)| AgentApiAuditHeader {
            name: name.as_str().to_string(),
            value: String::from_utf8_lossy(value.as_bytes()).to_string(),
        })
        .collect()
}

fn http_version_label(version: axum::http::Version) -> &'static str {
    match version {
        axum::http::Version::HTTP_09 => "HTTP/0.9",
        axum::http::Version::HTTP_10 => "HTTP/1.0",
        axum::http::Version::HTTP_11 => "HTTP/1.1",
        axum::http::Version::HTTP_2 => "HTTP/2.0",
        axum::http::Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/?",
    }
}

fn unix_timestamp_ms_now() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn append_agent_api_audit_event_to_disk(event: &AgentApiAuditEvent) -> Result<(), String> {
    let file = agent_api_audit_file();
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let mut handle = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|err| format!("Failed to open {}: {err}", file.display()))?;

    let line = serde_json::to_string(event)
        .map_err(|err| format!("Failed to serialize agent API audit event: {err}"))?;
    handle
        .write_all(line.as_bytes())
        .map_err(|err| format!("Failed to write {}: {err}", file.display()))?;
    handle
        .write_all(b"\n")
        .map_err(|err| format!("Failed to write {}: {err}", file.display()))?;
    Ok(())
}

fn load_agent_api_audit_events_from_disk(limit: usize) -> Result<Vec<AgentApiAuditEvent>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let file = agent_api_audit_file();
    if !file.exists() {
        return Ok(Vec::new());
    }

    let handle =
        File::open(&file).map_err(|err| format!("Failed to open {}: {err}", file.display()))?;
    let reader = BufReader::new(handle);

    let mut events = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<AgentApiAuditEvent>(&line) else {
            continue;
        };
        events.push(event);
    }

    events.reverse();
    if events.len() > limit {
        events.truncate(limit);
    }
    Ok(events)
}

fn count_agent_api_audit_events_on_disk() -> usize {
    let file = agent_api_audit_file();
    if !file.exists() {
        return 0;
    }

    let Ok(handle) = File::open(&file) else {
        return 0;
    };
    let reader = BufReader::new(handle);
    reader
        .lines()
        .filter(|line| {
            line.as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        })
        .count()
}

fn max_agent_api_audit_event_id_on_disk() -> Result<u64, String> {
    let file = agent_api_audit_file();
    if !file.exists() {
        return Ok(0);
    }

    let handle =
        File::open(&file).map_err(|err| format!("Failed to open {}: {err}", file.display()))?;
    let reader = BufReader::new(handle);
    let mut max_id = 0_u64;

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<AgentApiAuditEvent>(&line) else {
            continue;
        };
        max_id = max_id.max(event.id);
    }

    Ok(max_id)
}

fn clear_agent_api_audit_disk() -> Result<(), String> {
    let file = agent_api_audit_file();
    if file.exists() {
        fs::remove_file(&file)
            .map_err(|err| format!("Failed to remove {}: {err}", file.display()))?;
    }

    let dir = agent_api_audit_dir();
    if dir.exists() {
        let _ = fs::remove_dir(&dir);
    }

    Ok(())
}

fn agent_api_audit_dedupe_key(event: &AgentApiAuditEvent) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        event.id, event.started_at_unix_ms, event.method, event.path, event.status_code
    )
}

fn agent_api_audit_file() -> PathBuf {
    agent_api_audit_dir().join("events.jsonl")
}

fn agent_api_audit_dir() -> PathBuf {
    let config_file = config_path();
    let base_dir = config_file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".loopbox"));
    base_dir.join("agent-api-audit")
}

