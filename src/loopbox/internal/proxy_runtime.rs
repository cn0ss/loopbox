use super::features::{
    config_path, enforce_traffic_capture_mode, supports_traffic_capture, ProxyCaptureMode,
    ProxyTrafficEvent, ProxyTrafficHeader,
};
use axum::http::{HeaderMap, Request as HttpRequest, Response as HttpResponse, Version};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod connections;
mod decode;
mod http_io;
mod http_meta;
mod limits;
mod net;
mod redact;
mod routes;
mod time;

use connections::*;
use decode::*;
use http_io::*;
use http_meta::*;
use limits::*;
use net::*;
use redact::*;
use routes::*;
use time::*;

const PROXY_PRIMARY_PORT: u16 = 80;
const PROXY_FALLBACK_PORT: u16 = 18_080;
const MAX_REQUEST_HEADER_BYTES: usize = 256 * 1024;
const MAX_RESPONSE_HEADER_BYTES: usize = 256 * 1024;
const MAX_CAPTURED_HEADERS_PER_EVENT: usize = 128;
const MAX_CAPTURED_HEADER_NAME_LEN: usize = 128;
const MAX_CAPTURED_HEADER_VALUE_LEN: usize = 4 * 1024;
const DEFAULT_PROXY_TRAFFIC_MAX_EVENTS: usize = 2_000;
const DEFAULT_PROXY_REQUEST_BODY_PREVIEW_MAX_BYTES: usize = 64 * 1024;
const DEFAULT_PROXY_RESPONSE_BODY_PREVIEW_MAX_BYTES: usize = 128 * 1024;
const MIN_PROXY_BODY_PREVIEW_MAX_BYTES: usize = 256;
const MAX_PROXY_BODY_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
const MIN_PROXY_TRAFFIC_MAX_EVENTS: usize = 100;
const MAX_PROXY_TRAFFIC_MAX_EVENTS: usize = 100_000;
const DEFAULT_PROXY_TRAFFIC_WRITER_QUEUE_SIZE: usize = 10_000;
const MIN_PROXY_TRAFFIC_WRITER_QUEUE_SIZE: usize = 100;
const MAX_PROXY_TRAFFIC_WRITER_QUEUE_SIZE: usize = 100_000;
const DEFAULT_PROXY_TRAFFIC_RETENTION_DAYS: u16 = 7;
const MIN_PROXY_TRAFFIC_RETENTION_DAYS: u16 = 1;
const MAX_PROXY_TRAFFIC_RETENTION_DAYS: u16 = 90;
const DEFAULT_PROXY_TRAFFIC_MAX_STORAGE_MB: usize = 500;
const MIN_PROXY_TRAFFIC_MAX_STORAGE_MB: usize = 50;
const MAX_PROXY_TRAFFIC_MAX_STORAGE_MB: usize = 10_000;
const REDACTED_HEADER_VALUE: &str = "[redacted]";
const PROXY_STATUS_PROBE_TIMEOUT_MS: u64 = 120;
const PROXY_SIDECAR_STATUS_FILE_NAME: &str = "reverse-proxy-sidecar-status.json";
const PROXY_SIDECAR_STATUS_TTL_MS: u64 = 20_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LoopboxConfig {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_domain_suffix")]
    pub domain_suffix: String,
    #[serde(default = "default_ip_base")]
    pub ip_base: String,
    #[serde(default = "default_ip_range_start")]
    pub ip_range_start: u8,
    #[serde(default = "default_ip_range_end")]
    pub ip_range_end: u8,
    #[serde(default)]
    pub proxy_traffic: ProxyTrafficSettings,
    #[serde(default)]
    pub proxy_endpoints: Vec<ProxyEndpointConfig>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            domain_suffix: default_domain_suffix(),
            ip_base: default_ip_base(),
            ip_range_start: default_ip_range_start(),
            ip_range_end: default_ip_range_end(),
            proxy_traffic: ProxyTrafficSettings::default(),
            proxy_endpoints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyTrafficSettings {
    #[serde(default = "default_proxy_capture_enabled_by_default")]
    pub capture_enabled_by_default: bool,
    #[serde(default = "default_proxy_capture_mode")]
    pub capture_mode_default: ProxyCaptureMode,
    #[serde(default = "default_proxy_capture_text_only")]
    pub capture_text_only: bool,
    #[serde(default = "default_proxy_redact_headers")]
    pub redact_headers: Vec<String>,
    #[serde(default = "default_proxy_redact_query_keys")]
    pub redact_query_keys: Vec<String>,
    #[serde(default = "default_proxy_retention_days")]
    pub retention_days: u16,
    #[serde(default = "default_proxy_max_storage_mb")]
    pub max_storage_mb: usize,
    #[serde(default = "default_proxy_writer_queue_size")]
    pub writer_queue_size: usize,
    #[serde(default = "default_proxy_capture_body_preview")]
    pub capture_body_preview: bool,
    #[serde(default = "default_proxy_request_body_preview_max_bytes")]
    pub request_body_preview_max_bytes: usize,
    #[serde(default = "default_proxy_response_body_preview_max_bytes")]
    pub response_body_preview_max_bytes: usize,
    #[serde(default = "default_proxy_max_events")]
    pub max_events: usize,
}

impl Default for ProxyTrafficSettings {
    fn default() -> Self {
        Self {
            capture_enabled_by_default: default_proxy_capture_enabled_by_default(),
            capture_mode_default: default_proxy_capture_mode(),
            capture_text_only: default_proxy_capture_text_only(),
            redact_headers: default_proxy_redact_headers(),
            redact_query_keys: default_proxy_redact_query_keys(),
            retention_days: default_proxy_retention_days(),
            max_storage_mb: default_proxy_max_storage_mb(),
            writer_queue_size: default_proxy_writer_queue_size(),
            capture_body_preview: default_proxy_capture_body_preview(),
            request_body_preview_max_bytes: default_proxy_request_body_preview_max_bytes(),
            response_body_preview_max_bytes: default_proxy_response_body_preview_max_bytes(),
            max_events: default_proxy_max_events(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyEndpointConfig {
    pub name: String,
    #[serde(default = "default_proxy_endpoint_listen_host")]
    pub listen_host: String,
    pub listen_port: u16,
    #[serde(default = "default_proxy_endpoint_protocol")]
    pub protocol: ProxyEndpointProtocol,
    pub upstream_host: String,
    pub upstream_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyEndpointProtocol {
    Http1,
    GrpcH2c,
    TcpPassthrough,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    #[serde(default)]
    pub default_open_service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_traffic_capture_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_traffic_capture_mode: Option<ProxyCaptureMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grpc_proto_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proxy_endpoints: Vec<ProxyEndpointConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    #[serde(default = "default_service_runtime_kind")]
    pub runtime: ServiceRuntimeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerServiceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<ServicePortConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default = "default_service_port_protocol")]
    pub protocol: ProxyEndpointProtocol,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub workdir: String,
    #[serde(default)]
    pub env_files: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub health_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRuntimeKind {
    Process,
    Container,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerServiceConfig {
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub auto_remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePortConfig {
    pub port: u16,
    #[serde(default = "default_service_port_protocol")]
    pub protocol: ProxyEndpointProtocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseProxyStatus {
    pub running: bool,
    pub bind_port: u16,
    pub using_fallback_port: bool,
    pub note: Option<String>,
    pub listener_count: usize,
    pub endpoint_listener_count: usize,
    #[serde(default = "default_reverse_proxy_status_source")]
    pub source: String,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Default for ReverseProxyStatus {
    fn default() -> Self {
        Self {
            running: false,
            bind_port: 0,
            using_fallback_port: false,
            note: None,
            listener_count: 0,
            endpoint_listener_count: 0,
            source: default_reverse_proxy_status_source(),
            last_error: None,
        }
    }
}

fn default_reverse_proxy_status_source() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReverseProxySidecarStatusFile {
    pid: u32,
    updated_at_unix_ms: u64,
    status: ReverseProxyStatus,
    last_error: Option<String>,
}

pub fn load_config() -> Result<LoopboxConfig, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(LoopboxConfig::default());
    }

    let contents = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;

    toml::from_str::<LoopboxConfig>(&contents).map_err(|err| {
        format!(
            "Invalid TOML in {}: {err}. This version expects the new service-based loopbox schema.",
            path.display()
        )
    })
}

pub fn service_ports(service: &ServiceConfig) -> Vec<ServicePortConfig> {
    let mut result = Vec::new();
    let mut seen_ports = HashSet::new();

    for entry in &service.ports {
        if entry.port == 0 {
            continue;
        }
        if !seen_ports.insert(entry.port) {
            continue;
        }
        let health_path = entry
            .health_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        result.push(ServicePortConfig {
            port: entry.port,
            protocol: entry.protocol.clone(),
            health_path,
        });
    }

    if result.is_empty() {
        if let Some(port) = service.port.filter(|value| *value > 0) {
            let health_path = service
                .health_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            result.push(ServicePortConfig {
                port,
                protocol: service.protocol.clone(),
                health_path,
            });
        }
    }

    result
}

fn default_domain_suffix() -> String {
    "localhost".to_string()
}

fn default_ip_base() -> String {
    "127.0.0.".to_string()
}

fn default_ip_range_start() -> u8 {
    2
}

fn default_ip_range_end() -> u8 {
    254
}

fn default_proxy_capture_enabled_by_default() -> bool {
    false
}

fn default_proxy_capture_mode() -> ProxyCaptureMode {
    ProxyCaptureMode::Metadata
}

fn default_proxy_endpoint_listen_host() -> String {
    "127.0.0.1".to_string()
}

fn default_proxy_endpoint_protocol() -> ProxyEndpointProtocol {
    ProxyEndpointProtocol::GrpcH2c
}

fn default_service_port_protocol() -> ProxyEndpointProtocol {
    ProxyEndpointProtocol::Http1
}

fn default_service_runtime_kind() -> ServiceRuntimeKind {
    ServiceRuntimeKind::Process
}

fn default_proxy_capture_text_only() -> bool {
    true
}

fn default_proxy_redact_headers() -> Vec<String> {
    vec![
        "authorization".to_string(),
        "cookie".to_string(),
        "set-cookie".to_string(),
        "x-api-key".to_string(),
        "proxy-authorization".to_string(),
    ]
}

fn default_proxy_redact_query_keys() -> Vec<String> {
    vec![
        "token".to_string(),
        "key".to_string(),
        "secret".to_string(),
        "password".to_string(),
        "code".to_string(),
    ]
}

fn default_proxy_retention_days() -> u16 {
    7
}

fn default_proxy_max_storage_mb() -> usize {
    500
}

fn default_proxy_writer_queue_size() -> usize {
    10_000
}

fn default_proxy_capture_body_preview() -> bool {
    false
}

fn default_proxy_request_body_preview_max_bytes() -> usize {
    64 * 1024
}

fn default_proxy_response_body_preview_max_bytes() -> usize {
    128 * 1024
}

fn default_proxy_max_events() -> usize {
    2_000
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyRoute {
    project_name: String,
    service_name: String,
    target_ip: String,
    target_port: u16,
    capture_enabled: bool,
    capture_mode: ProxyCaptureMode,
    capture_text_only: bool,
    redacted_header_names: Vec<String>,
    redacted_query_keys: Vec<String>,
    request_body_preview_max_bytes: usize,
    response_body_preview_max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProxyEndpointKey {
    listen_host: String,
    listen_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyEndpointRoute {
    name: String,
    protocol: ProxyEndpointProtocol,
    upstream_host: String,
    upstream_port: u16,
    authority: Option<String>,
    project_name: String,
    service_name: String,
    grpc_proto_paths: Vec<String>,
    capture_enabled: bool,
    capture_mode: ProxyCaptureMode,
    capture_text_only: bool,
    redacted_header_names: Vec<String>,
    redacted_query_keys: Vec<String>,
    request_body_preview_max_bytes: usize,
    response_body_preview_max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForwardMetrics {
    status_code: Option<u16>,
    request_bytes: u64,
    response_bytes: u64,
    request_header_bytes: u64,
    request_body_bytes: u64,
    response_header_bytes: u64,
    response_body_bytes: u64,
    response_headers: Vec<ProxyTrafficHeader>,
    request_body: BodyPreviewResult,
    response_body: BodyPreviewResult,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BodyPreviewResult {
    preview: Option<String>,
    truncated: bool,
    binary: bool,
}

#[derive(Debug)]
struct ReverseProxyState {
    http_listener_started: bool,
    http_routes: Arc<RwLock<HashMap<String, ProxyRoute>>>,
    endpoint_routes: Arc<RwLock<HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>>>,
    started_endpoint_listeners: HashSet<ProxyEndpointKey>,
    base_note: Option<String>,
    status: ReverseProxyStatus,
    proxy_traffic_max_events: usize,
}

impl Default for ReverseProxyState {
    fn default() -> Self {
        Self {
            http_listener_started: false,
            http_routes: Arc::new(RwLock::new(HashMap::new())),
            endpoint_routes: Arc::new(RwLock::new(HashMap::new())),
            started_endpoint_listeners: HashSet::new(),
            base_note: None,
            status: ReverseProxyStatus::default(),
            proxy_traffic_max_events: DEFAULT_PROXY_TRAFFIC_MAX_EVENTS,
        }
    }
}

static REVERSE_PROXY_STATE: OnceLock<Mutex<ReverseProxyState>> = OnceLock::new();
static PROXY_ASYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn override_enabled() -> bool {
    true
}

fn reverse_proxy_state() -> &'static Mutex<ReverseProxyState> {
    REVERSE_PROXY_STATE.get_or_init(|| Mutex::new(ReverseProxyState::default()))
}

fn proxy_async_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    if let Some(runtime) = PROXY_ASYNC_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("loopbox-proxy")
        .build()
        .map_err(|err| format!("Failed to initialize proxy async runtime: {err}"))?;
    let _ = PROXY_ASYNC_RUNTIME.set(runtime);

    PROXY_ASYNC_RUNTIME
        .get()
        .ok_or_else(|| "Proxy async runtime initialization failed.".to_string())
}

pub fn sync_reverse_proxy(config: &LoopboxConfig) -> Result<ReverseProxyStatus, String> {
    let http_routes = build_proxy_routes(config);
    let endpoint_routes = build_proxy_endpoint_routes(config);
    let mut state = reverse_proxy_state()
        .lock()
        .map_err(|_| "Reverse proxy state lock poisoned.".to_string())?;

    state.proxy_traffic_max_events =
        sanitize_proxy_traffic_limit(config.global.proxy_traffic.max_events);

    {
        let mut route_guard = state
            .http_routes
            .write()
            .map_err(|_| "Reverse proxy route lock poisoned.".to_string())?;
        *route_guard = http_routes;
    }
    {
        let mut endpoint_guard = state
            .endpoint_routes
            .write()
            .map_err(|_| "Proxy endpoint route lock poisoned.".to_string())?;
        *endpoint_guard = endpoint_routes;
    }
    ensure_proxy_traffic_writer_running(config)?;

    if !state.http_listener_started {
        let (bind_port, using_fallback_port, note, listener) = bind_proxy_listener()?;
        let routes = state.http_routes.clone();
        thread::spawn(move || run_proxy_listener(listener, routes));
        state.http_listener_started = true;
        state.status.bind_port = bind_port;
        state.status.using_fallback_port = using_fallback_port;
        state.base_note = note;
    }
    let endpoint_notes = ensure_proxy_endpoint_listeners_running(&mut state);
    let mut notes = Vec::new();
    if let Some(base_note) = state.base_note.clone() {
        notes.push(base_note);
    }
    notes.extend(endpoint_notes);
    state.status.running =
        state.http_listener_started || !state.started_endpoint_listeners.is_empty();
    state.status.endpoint_listener_count = state.started_endpoint_listeners.len();
    state.status.listener_count = usize::from(state.http_listener_started)
        .saturating_add(state.status.endpoint_listener_count);
    state.status.note = if notes.is_empty() {
        None
    } else {
        Some(notes.join(" "))
    };
    state.status.source = "in_process".to_string();
    state.status.last_error = None;

    Ok(state.status.clone())
}

pub fn reverse_proxy_status() -> ReverseProxyStatus {
    reverse_proxy_state()
        .lock()
        .map(|state| state.status.clone())
        .unwrap_or_default()
}

pub fn effective_reverse_proxy_status(config: &LoopboxConfig) -> ReverseProxyStatus {
    let local = reverse_proxy_status();
    if local.running {
        return resolve_effective_reverse_proxy_status_with_probe(
            local,
            &[],
            |_host, _port| false,
            None,
        );
    }

    let hosts = proxy_probe_hosts(config);
    let sidecar_status = reverse_proxy_sidecar_status_from_disk();
    if let Some(status) = sidecar_status.as_ref().filter(|status| status.running) {
        return status.clone();
    }

    resolve_effective_reverse_proxy_status_with_probe(
        local,
        &hosts,
        |host, port| probe_reverse_proxy_port(host, port, PROXY_STATUS_PROBE_TIMEOUT_MS),
        sidecar_status.and_then(|status| status.last_error),
    )
}

pub fn reverse_proxy_url_for_host(host: &str) -> Option<String> {
    let status = reverse_proxy_status();
    if !status.running {
        return None;
    }
    if status.bind_port == PROXY_PRIMARY_PORT {
        Some(format!("http://{host}"))
    } else {
        Some(format!("http://{host}:{}", status.bind_port))
    }
}

pub fn effective_reverse_proxy_url_for_host(config: &LoopboxConfig, host: &str) -> Option<String> {
    let status = effective_reverse_proxy_status(config);
    if !status.running {
        return None;
    }
    if status.bind_port == PROXY_PRIMARY_PORT {
        Some(format!("http://{host}"))
    } else {
        Some(format!("http://{host}:{}", status.bind_port))
    }
}

pub fn record_reverse_proxy_sidecar_status(
    status: &ReverseProxyStatus,
    last_error: Option<String>,
) -> Result<(), String> {
    let path = reverse_proxy_sidecar_status_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let mut status = status.clone();
    status.source = "sidecar".to_string();
    status.last_error = last_error.clone();
    let payload = ReverseProxySidecarStatusFile {
        pid: std::process::id(),
        updated_at_unix_ms: unix_timestamp_ms(SystemTime::now()),
        status,
        last_error,
    };
    let serialized = serde_json::to_string_pretty(&payload)
        .map_err(|err| format!("Failed to serialize reverse proxy sidecar status: {err}"))?;
    fs::write(&path, serialized).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

pub fn clear_reverse_proxy_sidecar_status() -> Result<(), String> {
    let path = reverse_proxy_sidecar_status_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Failed to remove {}: {err}", path.display())),
    }
}

fn resolve_effective_reverse_proxy_status_with_probe<F>(
    mut local: ReverseProxyStatus,
    hosts: &[String],
    probe: F,
    last_error: Option<String>,
) -> ReverseProxyStatus
where
    F: Fn(&str, u16) -> bool,
{
    if local.running {
        if local.source.trim().is_empty() || local.source == "none" {
            local.source = "in_process".to_string();
        }
        local.last_error = None;
        return local;
    }

    for host in hosts {
        if probe(host, PROXY_PRIMARY_PORT) {
            return ReverseProxyStatus {
                running: true,
                bind_port: PROXY_PRIMARY_PORT,
                using_fallback_port: false,
                note: None,
                listener_count: 1,
                endpoint_listener_count: 0,
                source: "external_probe".to_string(),
                last_error: None,
            };
        }
    }
    for host in hosts {
        if probe(host, PROXY_FALLBACK_PORT) {
            return ReverseProxyStatus {
                running: true,
                bind_port: PROXY_FALLBACK_PORT,
                using_fallback_port: true,
                note: Some(format!(
                    "Reverse proxy responded on fallback port {PROXY_FALLBACK_PORT}."
                )),
                listener_count: 1,
                endpoint_listener_count: 0,
                source: "external_probe".to_string(),
                last_error: None,
            };
        }
    }

    ReverseProxyStatus {
        last_error,
        ..ReverseProxyStatus::default()
    }
}

fn proxy_probe_hosts(config: &LoopboxConfig) -> Vec<String> {
    let mut hosts = build_proxy_routes(config)
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn probe_reverse_proxy_port(host: &str, port: u16, timeout_ms: u64) -> bool {
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let timeout = Duration::from_millis(timeout_ms);
    addrs
        .filter(|addr| addr.ip().is_loopback())
        .any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

fn reverse_proxy_sidecar_status_from_disk() -> Option<ReverseProxyStatus> {
    let path = reverse_proxy_sidecar_status_path();
    let contents = fs::read_to_string(path).ok()?;
    let record: ReverseProxySidecarStatusFile = serde_json::from_str(&contents).ok()?;
    let age_ms = unix_timestamp_ms(SystemTime::now()).saturating_sub(record.updated_at_unix_ms);
    if age_ms > PROXY_SIDECAR_STATUS_TTL_MS {
        return None;
    }
    if !crate::platform::process::pid_exists(record.pid) {
        return None;
    }
    let mut status = record.status;
    status.source = "sidecar".to_string();
    if status.last_error.is_none() {
        status.last_error = record.last_error;
    }
    Some(status)
}

fn reverse_proxy_sidecar_status_path() -> std::path::PathBuf {
    config_path()
        .parent()
        .map(|parent| parent.join(PROXY_SIDECAR_STATUS_FILE_NAME))
        .unwrap_or_else(|| std::path::PathBuf::from(PROXY_SIDECAR_STATUS_FILE_NAME))
}

fn unix_timestamp_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn ensure_proxy_traffic_writer_running(config: &LoopboxConfig) -> Result<(), String> {
    let queue_size =
        sanitize_proxy_writer_queue_size(config.global.proxy_traffic.writer_queue_size);
    let retention_days = sanitize_proxy_retention_days(config.global.proxy_traffic.retention_days);
    let max_storage_mb = sanitize_proxy_max_storage_mb(config.global.proxy_traffic.max_storage_mb);
    super::features::ensure_proxy_traffic_writer_running(queue_size, retention_days, max_storage_mb)
}

#[cfg(test)]
fn parse_day_from_traffic_filename(name: &str) -> Option<i64> {
    super::features::parse_day_from_traffic_filename_for_test(name)
}

#[cfg(test)]
fn parse_day_key(day_key: &str) -> Option<i64> {
    super::features::parse_day_key_for_test(day_key)
}

#[cfg(test)]
fn proxy_event_to_har_entry(event: &ProxyTrafficEvent) -> serde_json::Value {
    super::features::proxy_event_to_har_entry_for_test(event)
}

fn ensure_proxy_endpoint_listeners_running(state: &mut ReverseProxyState) -> Vec<String> {
    let routes_snapshot = match state.endpoint_routes.read() {
        Ok(routes) => routes.clone(),
        Err(_) => {
            return vec!["Proxy endpoint route lock poisoned.".to_string()];
        }
    };

    let mut notes = Vec::new();
    for (key, route_set) in routes_snapshot {
        if route_set.is_empty() {
            continue;
        }
        let first_protocol = route_set[0].protocol.clone();
        if route_set
            .iter()
            .any(|route| route.protocol != first_protocol)
        {
            notes.push(format!(
                "Failed to start proxy endpoint listener on {}:{} (mixed protocols are not supported on the same listener).",
                key.listen_host, key.listen_port
            ));
            continue;
        }
        if state.started_endpoint_listeners.contains(&key) {
            continue;
        }
        match TcpListener::bind((key.listen_host.as_str(), key.listen_port)) {
            Ok(listener) => {
                let routes = state.endpoint_routes.clone();
                let listener_key = key.clone();
                thread::spawn(move || run_endpoint_proxy_listener(listener, listener_key, routes));
                state.started_endpoint_listeners.insert(key);
            }
            Err(err) => {
                let display_name = route_set
                    .first()
                    .map(|route| route.name.as_str())
                    .unwrap_or("endpoint");
                notes.push(format!(
                    "Failed to bind proxy endpoint '{}' on {}:{} ({err}).",
                    display_name, key.listen_host, key.listen_port
                ));
            }
        }
    }

    notes
}

fn bind_proxy_listener() -> Result<(u16, bool, Option<String>, TcpListener), String> {
    match TcpListener::bind(("0.0.0.0", PROXY_PRIMARY_PORT)) {
        Ok(listener) => Ok((PROXY_PRIMARY_PORT, false, None, listener)),
        Err(primary_err) => match TcpListener::bind(("0.0.0.0", PROXY_FALLBACK_PORT)) {
            Ok(listener) => Ok((
                PROXY_FALLBACK_PORT,
                true,
                Some(format!(
                    "Proxy could not bind :{PROXY_PRIMARY_PORT} ({primary_err}). Using fallback :{PROXY_FALLBACK_PORT}."
                )),
                listener,
            )),
            Err(fallback_err) => Err(format!(
                "Failed to start reverse proxy. Could not bind :{PROXY_PRIMARY_PORT} ({primary_err}) or :{PROXY_FALLBACK_PORT} ({fallback_err})."
            )),
        },
    }
}

fn run_proxy_listener(listener: TcpListener, routes: Arc<RwLock<HashMap<String, ProxyRoute>>>) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        let routes = routes.clone();
        thread::spawn(move || {
            let _ = handle_proxy_connection(stream, routes);
        });
    }
}

fn run_endpoint_proxy_listener(
    listener: TcpListener,
    listener_key: ProxyEndpointKey,
    endpoint_routes: Arc<RwLock<HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>>>,
) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        let listener_key = listener_key.clone();
        let endpoint_routes = endpoint_routes.clone();
        thread::spawn(move || {
            let _ = handle_endpoint_proxy_connection(stream, &listener_key, endpoint_routes);
        });
    }
}

#[derive(Debug, Clone)]
struct PreviewCapture {
    max_bytes: usize,
    text_only: bool,
    bytes: Vec<u8>,
    truncated: bool,
}

impl PreviewCapture {
    fn new(max_bytes: usize, text_only: bool) -> Self {
        Self {
            max_bytes,
            text_only,
            bytes: Vec::new(),
            truncated: false,
        }
    }

    fn ingest(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        if self.max_bytes == 0 {
            if !chunk.is_empty() {
                self.truncated = true;
            }
            return;
        }
        if self.bytes.len() >= self.max_bytes {
            self.truncated = true;
            return;
        }
        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if chunk.len() > remaining {
            self.bytes.extend_from_slice(&chunk[..remaining]);
            self.truncated = true;
        } else {
            self.bytes.extend_from_slice(chunk);
        }
    }

    fn finish(self) -> BodyPreviewResult {
        if self.bytes.is_empty() {
            return BodyPreviewResult {
                preview: None,
                truncated: self.truncated,
                binary: false,
            };
        }
        if self.text_only && !looks_like_text_bytes(&self.bytes) {
            return BodyPreviewResult {
                preview: None,
                truncated: self.truncated,
                binary: true,
            };
        }
        BodyPreviewResult {
            preview: Some(String::from_utf8_lossy(&self.bytes).to_string()),
            truncated: self.truncated,
            binary: false,
        }
    }
}

fn finalize_optional_preview(preview: Option<PreviewCapture>) -> BodyPreviewResult {
    preview.map(PreviewCapture::finish).unwrap_or_default()
}

fn finalize_grpc_optional_preview(
    preview: Option<PreviewCapture>,
    proto_paths: &[String],
    grpc_service: Option<&str>,
    grpc_method: Option<&str>,
    is_request: bool,
) -> BodyPreviewResult {
    let Some(preview) = preview else {
        return BodyPreviewResult::default();
    };

    let PreviewCapture {
        max_bytes: _,
        text_only,
        bytes,
        truncated,
    } = preview;

    if bytes.is_empty() {
        return BodyPreviewResult {
            preview: None,
            truncated,
            binary: false,
        };
    }

    if let Some(rendered) =
        render_grpc_preview(&bytes, proto_paths, grpc_service, grpc_method, is_request)
    {
        return BodyPreviewResult {
            preview: Some(rendered),
            truncated,
            binary: false,
        };
    }

    if text_only && !looks_like_text_bytes(&bytes) {
        return BodyPreviewResult {
            preview: None,
            truncated,
            binary: true,
        };
    }

    BodyPreviewResult {
        preview: Some(String::from_utf8_lossy(&bytes).to_string()),
        truncated,
        binary: false,
    }
}

fn current_project_grpc_proto_paths(project_name: &str, fallback: &[String]) -> Vec<String> {
    let trimmed = project_name.trim();
    if trimmed.is_empty() {
        return fallback.to_vec();
    }
    if let Ok(config) = load_config() {
        if let Some(project) = config.projects.get(trimmed) {
            if !project.grpc_proto_paths.is_empty() {
                return project.grpc_proto_paths.clone();
            }
        }
    }
    fallback.to_vec()
}

fn render_grpc_preview(
    bytes: &[u8],
    proto_paths: &[String],
    grpc_service: Option<&str>,
    grpc_method: Option<&str>,
    is_request: bool,
) -> Option<String> {
    super::features::render_grpc_preview(bytes, proto_paths, grpc_service, grpc_method, is_request)
}

#[cfg(test)]
fn split_grpc_frames(
    bytes: &[u8],
) -> (Vec<(super::features::GrpcFrameMetaForTest, Vec<u8>)>, bool) {
    super::features::split_grpc_frames_for_test(bytes)
}

#[cfg(test)]
fn beautify_protoc_text_output(raw: &str) -> String {
    super::features::beautify_protoc_text_output_for_test(raw)
}

#[cfg(test)]
mod tests;
