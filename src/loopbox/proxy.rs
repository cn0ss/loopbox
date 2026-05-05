use super::{service_ports, LoopboxConfig, ProjectConfig, ProxyCaptureMode, ProxyEndpointProtocol};
use axum::http::{Request as HttpRequest, Response as HttpResponse, Version};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

mod connections;
mod io;
mod meta;
mod net;

use connections::*;
use io::*;
use meta::*;
use net::*;

const PROXY_PRIMARY_PORT: u16 = 80;
const PROXY_FALLBACK_PORT: u16 = 18_080;
const MAX_REQUEST_HEADER_BYTES: usize = 256 * 1024;
const MAX_RESPONSE_HEADER_BYTES: usize = 256 * 1024;
const MAX_PROXY_TRAFFIC_MAX_EVENTS: usize = 100_000;

fn default_proxy_event_protocol() -> String {
    "http1".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverseProxyStatus {
    pub running: bool,
    pub bind_port: u16,
    pub using_fallback_port: bool,
    pub note: Option<String>,
    pub listener_count: usize,
    pub endpoint_listener_count: usize,
    pub source: String,
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
            source: "none".to_string(),
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyTrafficEvent {
    pub id: u64,
    pub started_at_utc: String,
    pub project_name: String,
    pub service_name: String,
    #[serde(default = "default_proxy_event_protocol")]
    pub protocol: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status_code: Option<u16>,
    #[serde(default)]
    pub stream_id: Option<u32>,
    #[serde(default)]
    pub grpc_service: Option<String>,
    #[serde(default)]
    pub grpc_method: Option<String>,
    #[serde(default)]
    pub grpc_status: Option<i32>,
    #[serde(default)]
    pub grpc_message: Option<String>,
    pub duration_ms: u64,
    pub request_bytes: u64,
    pub response_bytes: u64,
    #[serde(default)]
    pub request_header_bytes: u64,
    #[serde(default)]
    pub request_body_bytes: u64,
    #[serde(default)]
    pub response_header_bytes: u64,
    #[serde(default)]
    pub response_body_bytes: u64,
    pub request_headers: Vec<ProxyTrafficHeader>,
    pub response_headers: Vec<ProxyTrafficHeader>,
    pub request_body_preview: Option<String>,
    pub response_body_preview: Option<String>,
    pub request_body_truncated: bool,
    pub response_body_truncated: bool,
    pub request_body_binary: bool,
    pub response_body_binary: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyTrafficHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyTrafficDiskStats {
    pub dropped_events: u64,
    pub total_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyRoute {
    target_ip: String,
    target_port: u16,
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
}

#[derive(Debug)]
struct ReverseProxyState {
    http_listener_started: bool,
    http_routes: Arc<RwLock<HashMap<String, ProxyRoute>>>,
    endpoint_routes: Arc<RwLock<HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>>>,
    started_endpoint_listeners: HashSet<ProxyEndpointKey>,
    base_note: Option<String>,
    status: ReverseProxyStatus,
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
        }
    }
}

static REVERSE_PROXY_STATE: OnceLock<Mutex<ReverseProxyState>> = OnceLock::new();
static PROXY_ASYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn map_bridge_status(status: super::proxy_bridge::ReverseProxyStatus) -> ReverseProxyStatus {
    ReverseProxyStatus {
        running: status.running,
        bind_port: status.bind_port,
        using_fallback_port: status.using_fallback_port,
        note: status.note,
        listener_count: status.listener_count,
        endpoint_listener_count: status.endpoint_listener_count,
        source: status.source,
        last_error: status.last_error,
    }
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
    if super::proxy_bridge::override_enabled() {
        let status = super::proxy_bridge::sync_reverse_proxy(config)?;
        return Ok(map_bridge_status(status));
    }

    let http_routes = build_proxy_routes(config);
    let endpoint_routes = build_proxy_endpoint_routes(config);

    let mut state = reverse_proxy_state()
        .lock()
        .map_err(|_| "Reverse proxy state lock poisoned.".to_string())?;

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
    if super::proxy_bridge::override_enabled() {
        return map_bridge_status(super::proxy_bridge::reverse_proxy_status());
    }

    reverse_proxy_state()
        .lock()
        .map(|state| state.status.clone())
        .unwrap_or_default()
}

pub fn effective_reverse_proxy_status(config: &LoopboxConfig) -> ReverseProxyStatus {
    if super::proxy_bridge::override_enabled() {
        return map_bridge_status(super::proxy_bridge::effective_reverse_proxy_status(config));
    }

    let local = reverse_proxy_status();
    if local.running {
        return local;
    }
    let hosts = build_proxy_routes(config)
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    resolve_effective_reverse_proxy_status_with_probe(local, &hosts, |host, port| {
        probe_reverse_proxy_port(host, port, 120)
    })
}

pub fn reverse_proxy_url_for_host(host: &str) -> Option<String> {
    if super::proxy_bridge::override_enabled() {
        return super::proxy_bridge::reverse_proxy_url_for_host(host);
    }

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
    if super::proxy_bridge::override_enabled() {
        return super::proxy_bridge::effective_reverse_proxy_url_for_host(config, host);
    }

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
    if super::proxy_bridge::override_enabled() {
        return super::proxy_bridge::record_reverse_proxy_sidecar_status(status, last_error);
    }
    let _ = status;
    let _ = last_error;
    Ok(())
}

pub fn clear_reverse_proxy_sidecar_status() -> Result<(), String> {
    if super::proxy_bridge::override_enabled() {
        return super::proxy_bridge::clear_reverse_proxy_sidecar_status();
    }
    Ok(())
}

pub fn reverse_proxy_fallback_port() -> u16 {
    PROXY_FALLBACK_PORT
}

pub fn project_proxy_traffic_enabled(config: &LoopboxConfig, project_name: &str) -> bool {
    super::features::project_proxy_traffic_enabled(config, project_name)
}

pub fn project_proxy_traffic_capture_mode(
    config: &LoopboxConfig,
    project_name: &str,
) -> ProxyCaptureMode {
    super::features::project_proxy_traffic_capture_mode(config, project_name)
}

pub fn proxy_traffic_events_for_project_with_persisted(
    project_name: &str,
    service_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ProxyTrafficEvent>, String> {
    let effective_limit = limit.clamp(1, MAX_PROXY_TRAFFIC_MAX_EVENTS);
    super::features::proxy_traffic_events_for_project_with_persisted(
        project_name,
        service_filter,
        effective_limit,
    )
}

pub fn clear_proxy_traffic_events_for_project(project_name: &str) -> Result<usize, String> {
    super::features::clear_proxy_traffic_events_for_project(project_name)
}

pub fn proxy_traffic_disk_stats() -> ProxyTrafficDiskStats {
    super::features::proxy_traffic_disk_stats()
}

pub fn export_proxy_traffic_har_for_project(
    project_name: &str,
    service_filter: Option<&str>,
    output_path: &std::path::Path,
) -> Result<usize, String> {
    super::features::export_proxy_traffic_har_for_project(project_name, service_filter, output_path)
}

fn resolve_effective_reverse_proxy_status_with_probe<F>(
    mut local: ReverseProxyStatus,
    hosts: &[String],
    probe: F,
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

    ReverseProxyStatus::default()
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

fn build_proxy_routes(config: &LoopboxConfig) -> HashMap<String, ProxyRoute> {
    let mut routes = HashMap::new();
    let suffix = config.global.domain_suffix.trim().trim_start_matches('.');

    for (project_name, project) in &config.projects {
        let project_clean = project_name.trim().to_lowercase();
        for service in &project.services {
            let http_port = service_ports(service)
                .into_iter()
                .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
                .map(|entry| entry.port);
            let Some(port) = http_port else {
                continue;
            };

            let service_clean = service.name.trim().to_lowercase();
            let host = format!("{service_clean}.{project_clean}.{suffix}").to_lowercase();
            routes.insert(
                host,
                ProxyRoute {
                    target_ip: project.ip.trim().to_string(),
                    target_port: port,
                },
            );
        }
    }

    routes
}

fn build_proxy_endpoint_routes(
    config: &LoopboxConfig,
) -> HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>> {
    let mut routes = HashMap::new();
    let suffix = config
        .global
        .domain_suffix
        .trim()
        .trim_start_matches('.')
        .to_lowercase();

    for (project_name, project) in &config.projects {
        for endpoint in &project.proxy_endpoints {
            if endpoint.listen_port == 0 || endpoint.upstream_port == 0 {
                continue;
            }
            let listen_host = endpoint.listen_host.trim();
            let upstream_host = endpoint.upstream_host.trim();
            if upstream_host.is_empty() {
                continue;
            }

            let key = ProxyEndpointKey {
                listen_host: if listen_host.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    listen_host.to_string()
                },
                listen_port: endpoint.listen_port,
            };
            let service_name = endpoint
                .service_name
                .as_ref()
                .map(|value| value.trim().to_lowercase())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    find_service_for_endpoint_in_project(
                        project,
                        upstream_host,
                        endpoint.upstream_port,
                    )
                })
                .unwrap_or_else(|| endpoint.name.trim().to_string());

            insert_proxy_endpoint_route(
                &mut routes,
                key,
                ProxyEndpointRoute {
                    name: endpoint.name.trim().to_string(),
                    protocol: endpoint.protocol.clone(),
                    upstream_host: upstream_host.to_string(),
                    upstream_port: endpoint.upstream_port,
                    authority: endpoint
                        .authority
                        .as_ref()
                        .map(|value| value.trim().to_lowercase()),
                    project_name: project_name.to_string(),
                    service_name,
                },
            );
        }
    }

    // Backward compatibility for legacy global endpoint routes.
    for endpoint in &config.global.proxy_endpoints {
        if endpoint.listen_port == 0 || endpoint.upstream_port == 0 {
            continue;
        }
        let listen_host = endpoint.listen_host.trim();
        let upstream_host = endpoint.upstream_host.trim();
        if upstream_host.is_empty() {
            continue;
        }

        let key = ProxyEndpointKey {
            listen_host: if listen_host.is_empty() {
                "127.0.0.1".to_string()
            } else {
                listen_host.to_string()
            },
            listen_port: endpoint.listen_port,
        };

        let mut project_name = endpoint
            .project_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let mut service_name = endpoint
            .service_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        if project_name.is_none() || service_name.is_none() {
            if let Some((matched_project, matched_service)) =
                find_project_service_for_endpoint(config, upstream_host, endpoint.upstream_port)
            {
                project_name = Some(matched_project);
                service_name = Some(matched_service);
            }
        }

        let effective_project_name = project_name.unwrap_or_else(|| "legacy-global".to_string());
        let effective_service_name = service_name
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| endpoint.name.trim().to_string());

        insert_proxy_endpoint_route(
            &mut routes,
            key,
            ProxyEndpointRoute {
                name: endpoint.name.trim().to_string(),
                protocol: endpoint.protocol.clone(),
                upstream_host: upstream_host.to_string(),
                upstream_port: endpoint.upstream_port,
                authority: endpoint
                    .authority
                    .as_ref()
                    .map(|value| value.trim().to_lowercase()),
                project_name: effective_project_name,
                service_name: effective_service_name,
            },
        );
    }

    // Auto routes: derive gRPC and TCP listeners from sandbox service ports.
    for (project_name, project) in &config.projects {
        for service in &project.services {
            for port_entry in service_ports(service) {
                match port_entry.protocol {
                    ProxyEndpointProtocol::GrpcH2c => {
                        let key = ProxyEndpointKey {
                            listen_host: "127.0.0.1".to_string(),
                            listen_port: port_entry.port,
                        };
                        let authority = if suffix.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "{}.{}.{}",
                                service.name.trim().to_lowercase(),
                                project_name.trim().to_lowercase(),
                                suffix
                            ))
                        };
                        let route_name = format!(
                            "auto-{}-{}-grpc-{}",
                            project_name.trim().to_lowercase(),
                            service.name.trim().to_lowercase(),
                            port_entry.port
                        );
                        insert_proxy_endpoint_route(
                            &mut routes,
                            key,
                            ProxyEndpointRoute {
                                name: route_name,
                                protocol: ProxyEndpointProtocol::GrpcH2c,
                                upstream_host: project.ip.trim().to_string(),
                                upstream_port: port_entry.port,
                                authority,
                                project_name: project_name.to_string(),
                                service_name: service.name.clone(),
                            },
                        );
                    }
                    ProxyEndpointProtocol::TcpPassthrough => {
                        let key = ProxyEndpointKey {
                            listen_host: "127.0.0.1".to_string(),
                            listen_port: port_entry.port,
                        };
                        let route_name = format!(
                            "auto-{}-{}-tcp-{}",
                            project_name.trim().to_lowercase(),
                            service.name.trim().to_lowercase(),
                            port_entry.port
                        );
                        insert_proxy_endpoint_route(
                            &mut routes,
                            key,
                            ProxyEndpointRoute {
                                name: route_name,
                                protocol: ProxyEndpointProtocol::TcpPassthrough,
                                upstream_host: project.ip.trim().to_string(),
                                upstream_port: port_entry.port,
                                authority: None,
                                project_name: project_name.to_string(),
                                service_name: service.name.clone(),
                            },
                        );
                    }
                    ProxyEndpointProtocol::Http1 => {}
                }
            }
        }
    }

    routes
}

fn insert_proxy_endpoint_route(
    routes: &mut HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>,
    key: ProxyEndpointKey,
    route: ProxyEndpointRoute,
) {
    let route_protocol = route.protocol.clone();
    let route_authority = route.authority.clone().unwrap_or_default();
    let route_authority = route_authority.trim().to_lowercase();

    let entry = routes.entry(key).or_default();
    if entry
        .iter()
        .any(|existing| existing.protocol != route_protocol)
    {
        return;
    }

    match route_protocol {
        ProxyEndpointProtocol::GrpcH2c => {
            let duplicate = entry.iter().any(|existing| {
                existing
                    .authority
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    == route_authority
            });
            if !duplicate {
                entry.push(route);
            }
        }
        ProxyEndpointProtocol::Http1 | ProxyEndpointProtocol::TcpPassthrough => {
            if entry.is_empty() {
                entry.push(route);
            }
        }
    }
}

fn find_service_for_endpoint_in_project(
    project: &ProjectConfig,
    upstream_host: &str,
    upstream_port: u16,
) -> Option<String> {
    if !project.ip.trim().eq_ignore_ascii_case(upstream_host.trim()) {
        return None;
    }
    project
        .services
        .iter()
        .find(|service| {
            service_ports(service)
                .iter()
                .any(|entry| entry.port == upstream_port)
        })
        .map(|service| service.name.clone())
}

fn find_project_service_for_endpoint(
    config: &LoopboxConfig,
    upstream_host: &str,
    upstream_port: u16,
) -> Option<(String, String)> {
    for (project_name, project) in &config.projects {
        if !project.ip.trim().eq_ignore_ascii_case(upstream_host.trim()) {
            continue;
        }
        for service in &project.services {
            if service_ports(service)
                .iter()
                .any(|entry| entry.port == upstream_port)
            {
                return Some((project_name.clone(), service.name.clone()));
            }
        }
    }
    None
}

fn grpc_request_authority(request: &HttpRequest<h2::RecvStream>) -> Option<String> {
    request
        .uri()
        .authority()
        .map(|value| value.as_str())
        .or_else(|| {
            request
                .headers()
                .get("host")
                .and_then(|value| value.to_str().ok())
        })
        .and_then(normalize_authority)
}

fn select_grpc_route_for_authority<'a>(
    routes: &'a [ProxyEndpointRoute],
    authority: Option<&str>,
) -> Option<&'a ProxyEndpointRoute> {
    if routes.is_empty() {
        return None;
    }
    if routes.len() == 1 {
        return routes.first();
    }

    if let Some(authority) = authority {
        for route in routes {
            if let Some(pattern) = route.authority.as_deref() {
                if authority_matches(pattern, authority) {
                    return Some(route);
                }
            }
        }
    }

    routes.iter().find(|route| route.authority.is_none())
}

fn normalize_authority(value: &str) -> Option<String> {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split("://")
        .last()
        .unwrap_or(trimmed.as_str())
        .trim();
    let authority_only = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .trim();
    if authority_only.is_empty() {
        return None;
    }
    Some(authority_only.to_string())
}

fn authority_matches(pattern: &str, incoming: &str) -> bool {
    let normalized_pattern = match normalize_authority(pattern) {
        Some(value) => value,
        None => return false,
    };
    let normalized_incoming = match normalize_authority(incoming) {
        Some(value) => value,
        None => return false,
    };

    if normalized_pattern == normalized_incoming {
        return true;
    }
    if authority_has_explicit_port(&normalized_pattern) {
        return false;
    }
    strip_host_port(&normalized_pattern)
        .eq_ignore_ascii_case(&strip_host_port(&normalized_incoming))
}

fn authority_has_explicit_port(authority: &str) -> bool {
    if authority.starts_with('[') {
        return authority.contains("]:");
    }
    authority.matches(':').count() == 1
}

fn ensure_proxy_endpoint_listeners_running(state: &mut ReverseProxyState) -> Vec<String> {
    let routes_snapshot = match state.endpoint_routes.read() {
        Ok(routes) => routes.clone(),
        Err(_) => return vec!["Proxy endpoint route lock poisoned.".to_string()],
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

#[cfg(test)]
mod tests;
