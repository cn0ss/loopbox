use super::{
    add_project, app_version_label, apply_system_setup, config_path, load_config,
    project_primary_host, project_proxy_traffic_capture_mode, project_proxy_traffic_enabled,
    proxy_traffic_events_for_project_with_persisted, restart_service, reverse_proxy_status,
    save_config, service_log_attached, service_logs_tail, service_ports, service_runtime_status,
    start_project_all, start_service, stop_project_all, stop_service, sync_reverse_proxy,
    update_project, AddProjectInput, AgentApiAuditEvent, AgentApiSettings, ContainerServiceConfig,
    LoopboxConfig, OpenTarget, ProjectConfig, ProxyCaptureMode, ProxyEndpointProtocol,
    ServiceConfig, ServiceEntry, ServicePortEntry, ServiceRuntimeKind, ServiceRuntimeSnapshot,
    ServiceRuntimeState, UpdateProjectInput,
};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_AGENT_API_PORT: u16 = 39_393;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 2_000;
const DEFAULT_REQUEST_LIMIT: usize = 200;
const MAX_REQUEST_LIMIT: usize = 2_000;
const TOKEN_FILE_NAME: &str = "agent-api-token";
const DISCOVERY_FILE_NAME: &str = "agent-api.json";
const AGENT_API_VERSION: &str = "v1";
const AGENT_GUIDANCE_START_MARKER: &str = "<!-- loopbox-agent-api:start -->";
const AGENT_GUIDANCE_END_MARKER: &str = "<!-- loopbox-agent-api:end -->";

static AGENT_API_RUNTIME: OnceLock<Mutex<AgentApiRuntime>> = OnceLock::new();

mod routes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentApiServerInfo {
    pub enabled: bool,
    pub running: bool,
    pub auth_enabled: bool,
    pub bind_port: u16,
    pub base_url: Option<String>,
    pub openapi_url: Option<String>,
    pub token_path: Option<String>,
    pub discovery_path: String,
}

impl AgentApiServerInfo {
    fn disabled(port: u16, auth_enabled: bool) -> Self {
        Self {
            enabled: false,
            running: false,
            auth_enabled,
            bind_port: port,
            base_url: None,
            openapi_url: None,
            token_path: None,
            discovery_path: agent_api_discovery_path().display().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentApiRuntimeConfig {
    enabled: bool,
    auth_enabled: bool,
    bind_port: u16,
}

impl AgentApiRuntimeConfig {
    fn from_settings(settings: &AgentApiSettings) -> Self {
        Self {
            enabled: settings.enabled,
            auth_enabled: settings.auth_enabled,
            bind_port: if settings.port == 0 {
                DEFAULT_AGENT_API_PORT
            } else {
                settings.port
            },
        }
    }
}

#[derive(Debug)]
struct RunningAgentApi {
    config: AgentApiRuntimeConfig,
    info: AgentApiServerInfo,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Debug)]
struct AgentApiRuntime {
    running: Option<RunningAgentApi>,
    mutation_lock: Arc<Mutex<()>>,
    last_discovery: Option<AgentApiServerInfo>,
}

impl Default for AgentApiRuntime {
    fn default() -> Self {
        Self {
            running: None,
            mutation_lock: Arc::new(Mutex::new(())),
            last_discovery: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AgentApiState {
    expected_bearer: Option<Arc<String>>,
    auth_enabled: bool,
    bind_port: u16,
    mutation_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Clone, Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
struct HealthResponse {
    ok: bool,
    api_version: &'static str,
    app_version: String,
    reverse_proxy: ReverseProxyInfo,
    agent_api: AgentApiHealthInfo,
}

#[derive(Debug, Clone, Serialize)]
struct ReverseProxyInfo {
    running: bool,
    bind_port: u16,
    using_fallback_port: bool,
    note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AgentApiHealthInfo {
    auth_enabled: bool,
    bind_port: u16,
}

#[derive(Debug, Clone, Serialize)]
struct MetaResponse {
    api_version: &'static str,
    log_limit_default: usize,
    log_limit_max: usize,
    request_limit_default: usize,
    request_limit_max: usize,
    auth_enabled: bool,
    openapi_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectsResponse {
    projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectSummary {
    name: String,
    dir: String,
    ip: String,
    primary_host: String,
    service_count: usize,
    status: RuntimeCounts,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RuntimeCounts {
    running: usize,
    starting: usize,
    unhealthy: usize,
    crashed: usize,
    stopped: usize,
    unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectDetailResponse {
    name: String,
    primary_host: String,
    config: ProjectConfig,
    services: Vec<ProjectServiceDetail>,
    capture_enabled: bool,
    capture_mode: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectServiceDetail {
    name: String,
    port: Option<u16>,
    host: String,
    url: String,
    command: String,
    workdir: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectRuntimeResponse {
    project: String,
    services: Vec<ServiceRuntimeDto>,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceRuntimeDto {
    service: String,
    state: RuntimeStateDto,
    pid: Option<u32>,
    started_at: Option<u64>,
    exit_code: Option<i32>,
    last_error: Option<String>,
    log_attached: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeStateDto {
    Stopped,
    Starting,
    Running,
    Unhealthy,
    Crashed,
}

#[derive(Debug, Clone, Deserialize)]
struct LogsQuery {
    service: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct LogsResponse {
    project: String,
    service: String,
    limit: usize,
    log_attached: bool,
    lines: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RequestsQuery {
    service: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ProjectMutationQuery {
    #[serde(default)]
    apply_system_setup: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectCreateRequest {
    name: String,
    dir: String,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    services: Vec<ProjectServiceRequest>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectUpdateRequest {
    dir: String,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    services: Vec<ProjectServiceRequest>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectServiceRequest {
    name: String,
    #[serde(default)]
    ports: Vec<ProjectServicePortRequest>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    protocol: Option<ProxyEndpointProtocol>,
    #[serde(default)]
    runtime: Option<ServiceRuntimeKind>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    workdir: String,
    #[serde(default)]
    env_files: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    autostart: bool,
    #[serde(default)]
    health_path: Option<String>,
    #[serde(default)]
    container: Option<ContainerServiceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectServicePortRequest {
    port: u16,
    #[serde(default)]
    protocol: Option<ProxyEndpointProtocol>,
    #[serde(default)]
    health_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RequestsResponse {
    project: String,
    service: Option<String>,
    limit: usize,
    capture_enabled: bool,
    capture_mode: &'static str,
    events: Vec<super::ProxyTrafficEvent>,
}

#[derive(Debug, Clone, Serialize)]
struct MutationResponse {
    project: String,
    service: Option<String>,
    action: &'static str,
    results: Vec<ServiceRuntimeDto>,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectConfigMutationResponse {
    project: String,
    action: &'static str,
    saved_config_path: String,
    reverse_proxy_synced: bool,
    system_setup_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_setup_message: Option<String>,
    detail: ProjectDetailResponse,
}

#[derive(Debug, Clone)]
struct ProjectConfigPersistOutcome {
    saved_config_path: String,
    system_setup_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DiscoveryPayload {
    schema: &'static str,
    enabled: bool,
    running: bool,
    auth_enabled: bool,
    bind_port: u16,
    base_url: Option<String>,
    openapi_url: Option<String>,
    token_path: Option<String>,
    api_version: &'static str,
    generated_at_unix: u64,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "operation_failed", message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let payload = ApiErrorEnvelope {
            error: ApiErrorBody {
                code: self.code,
                message: self.message,
                details: self.details,
            },
        };
        (self.status, Json(payload)).into_response()
    }
}

impl From<ServiceRuntimeState> for RuntimeStateDto {
    fn from(value: ServiceRuntimeState) -> Self {
        match value {
            ServiceRuntimeState::Stopped => Self::Stopped,
            ServiceRuntimeState::Starting => Self::Starting,
            ServiceRuntimeState::Running => Self::Running,
            ServiceRuntimeState::Unhealthy => Self::Unhealthy,
            ServiceRuntimeState::Crashed => Self::Crashed,
        }
    }
}

pub fn agent_api_token_path() -> PathBuf {
    agent_api_base_dir().join(TOKEN_FILE_NAME)
}

pub fn agent_api_discovery_path() -> PathBuf {
    agent_api_base_dir().join(DISCOVERY_FILE_NAME)
}

pub fn agent_api_bootstrap_prompt_for_values(
    base_url: &str,
    openapi_url: &str,
    discovery_path: &str,
    auth_enabled: bool,
    token_path: &str,
) -> String {
    let auth_block = if auth_enabled {
        format!(
            "auth_enabled: true\n\
token_path: {token_path}\n\
header: Authorization: Bearer <token>"
        )
    } else {
        "auth_enabled: false\nheader: none".to_string()
    };

    format!(
        "Loopbox context:\n\
Loopbox is a local sandbox control plane for development projects.\n\
Primary goal: eliminate local port conflicts by giving each project a dedicated loopback IP and stable hostnames.\n\
Treat Loopbox as the source of truth for project runtime, URLs, logs, and request inspection instead of assuming raw localhost ports.\n\n\
Discovery-first connection flow:\n\
1) Read discovery_file first on every new session or reconnect.\n\
2) Use the discovery file values as authoritative for base_url, openapi_url, auth mode, and token path.\n\
3) If the discovery file is unavailable, fall back to the values below.\n\
4) Fetch GET /v1/openapi.json before assuming request/response shapes.\n\n\
Connection fallback:\n\
base_url: {base_url}\n\
openapi_url: {openapi_url}\n\
discovery_file: {discovery_path}\n\
{auth_block}\n\n\
Recommended agent workflow:\n\
1) GET /v1/health\n\
2) GET /v1/meta\n\
3) GET /v1/projects\n\
4) GET /v1/projects/{{project}}\n\
5) GET /v1/projects/{{project}}/runtime\n\
6) Only when needed: GET /v1/projects/{{project}}/logs?service={{svc}}&limit=50\n\
7) Only when needed: GET /v1/projects/{{project}}/requests?service={{svc}}&limit=50\n\
8) Config mutations: POST /v1/projects | PUT /v1/projects/{{project}}\n\
9) Runtime control: POST /v1/projects/{{project}}/start | /stop | /restart\n\
10) Service control: POST /v1/projects/{{project}}/services/{{service}}/start | /stop | /restart\n\n\
Agent behavior:\n\
- Prefer Loopbox-managed hostnames and Loopbox config over guessing ports.\n\
- Prefer project/runtime inspection before reading logs.\n\
- Use the OpenAPI document for exact schemas and endpoint details.\n\
- Remember that request capture may be empty or disabled depending on local settings.\n\
- There is currently no delete project endpoint; avoid assuming one exists.\n\n\
Persistence for future agents:\n\
Add or update a short \"Loopbox Agent API\" section in AGENTS.md or CLAUDE.md with:\n\
- what Loopbox is: local sandbox control + zero-conflict loopback networking\n\
- read discovery_file first each session\n\
- base_url\n\
- openapi_url\n\
- auth mode and token path (if enabled)\n\
- the core workflow: health -> meta -> projects -> project detail -> runtime -> logs/requests only when needed"
    )
}

pub fn agent_api_bootstrap_prompt(config: &LoopboxConfig) -> String {
    let port = config.global.agent_api.port;
    let base_url = format!("http://127.0.0.1:{port}");
    let openapi_url = format!("{base_url}/{AGENT_API_VERSION}/openapi.json");
    let discovery_path = agent_api_discovery_path().display().to_string();
    let token_path = agent_api_token_path().display().to_string();
    agent_api_bootstrap_prompt_for_values(
        &base_url,
        &openapi_url,
        &discovery_path,
        config.global.agent_api.auth_enabled,
        &token_path,
    )
}

pub fn ensure_project_agent_guidance(
    config: &LoopboxConfig,
    project_name: &str,
) -> Result<Option<PathBuf>, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let project_dir = PathBuf::from(project.dir.trim());
    if !project_dir.is_dir() {
        return Ok(None);
    }

    let agents_path = project_dir.join("AGENTS.md");
    let claude_path = project_dir.join("CLAUDE.md");
    let target_path = if agents_path.exists() {
        agents_path
    } else if claude_path.exists() {
        claude_path
    } else {
        project_dir.join("AGENTS.md")
    };

    let existing = match fs::read_to_string(&target_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(format!(
                "Failed to read agent guidance file {}: {err}",
                target_path.display()
            ));
        }
    };
    let section = format!(
        "{AGENT_GUIDANCE_START_MARKER}\n## Loopbox Agent API\n\n{}\n{AGENT_GUIDANCE_END_MARKER}\n",
        agent_api_bootstrap_prompt(config)
    );
    let updated = upsert_agent_guidance_section(&existing, &section);
    fs::write(&target_path, updated).map_err(|err| {
        format!(
            "Failed to write agent guidance file {}: {err}",
            target_path.display()
        )
    })?;
    Ok(Some(target_path))
}

pub fn agent_api_audit_events(limit: usize) -> Result<Vec<AgentApiAuditEvent>, String> {
    super::features::agent_api_audit_events(limit)
}

pub fn clear_agent_api_audit_events() -> Result<usize, String> {
    super::features::clear_agent_api_audit_events()
}

pub(super) async fn run_agent_api_audit_middleware(
    auth_enabled: bool,
    request: Request,
    next: Next,
) -> Response {
    super::features::run_agent_api_audit_middleware(auth_enabled, request, next).await
}

#[allow(dead_code)]
pub fn start_agent_api_server() -> Result<AgentApiServerInfo, String> {
    let config = load_config().unwrap_or_default();
    sync_agent_api_server(&config)
}

pub fn sync_agent_api_server(config: &LoopboxConfig) -> Result<AgentApiServerInfo, String> {
    let desired = AgentApiRuntimeConfig::from_settings(&config.global.agent_api);
    let token_path = agent_api_token_path();
    let discovery_path = agent_api_discovery_path();

    if !desired.enabled {
        let mut runtime = agent_api_runtime()
            .lock()
            .map_err(|_| "Agent API runtime state lock poisoned.".to_string())?;
        stop_running_server(&mut runtime.running);
        let info = AgentApiServerInfo::disabled(desired.bind_port, desired.auth_enabled);
        if runtime.last_discovery.as_ref() != Some(&info) {
            write_discovery_file(&discovery_path, &info)?;
            runtime.last_discovery = Some(info.clone());
        }
        return Ok(info);
    }

    {
        let runtime = agent_api_runtime()
            .lock()
            .map_err(|_| "Agent API runtime state lock poisoned.".to_string())?;
        if let Some(running) = runtime.running.as_ref() {
            if running.config == desired {
                let info = running.info.clone();
                return Ok(info);
            }
        }
    }

    let token = if desired.auth_enabled {
        Some(load_or_create_api_token(&token_path)?)
    } else {
        None
    };

    let expected_bearer = token
        .as_ref()
        .map(|value| Arc::new(format!("Bearer {value}")));

    let mut runtime = agent_api_runtime()
        .lock()
        .map_err(|_| "Agent API runtime state lock poisoned.".to_string())?;
    stop_running_server(&mut runtime.running);

    let state = AgentApiState {
        expected_bearer,
        auth_enabled: desired.auth_enabled,
        bind_port: desired.bind_port,
        mutation_lock: runtime.mutation_lock.clone(),
    };
    let bind_addr = format!("127.0.0.1:{}", desired.bind_port);

    let (startup_tx, startup_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let thread_handle = thread::Builder::new()
        .name("loopbox-agent-api".to_string())
        .spawn(move || run_agent_api_server(bind_addr, state, shutdown_rx, startup_tx))
        .map_err(|err| format!("Failed to start agent API thread: {err}"))?;

    let base_url = startup_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Timed out while starting local agent API server.".to_string())??;
    let openapi_url = format!("{base_url}/{AGENT_API_VERSION}/openapi.json");

    let info = AgentApiServerInfo {
        enabled: true,
        running: true,
        auth_enabled: desired.auth_enabled,
        bind_port: desired.bind_port,
        base_url: Some(base_url),
        openapi_url: Some(openapi_url),
        token_path: token.map(|_| token_path.display().to_string()),
        discovery_path: discovery_path.display().to_string(),
    };

    write_discovery_file(&discovery_path, &info)?;
    runtime.last_discovery = Some(info.clone());
    runtime.running = Some(RunningAgentApi {
        config: desired,
        info: info.clone(),
        shutdown: Some(shutdown_tx),
        thread: Some(thread_handle),
    });

    Ok(info)
}

fn agent_api_runtime() -> &'static Mutex<AgentApiRuntime> {
    AGENT_API_RUNTIME.get_or_init(|| Mutex::new(AgentApiRuntime::default()))
}

fn stop_running_server(running: &mut Option<RunningAgentApi>) {
    let Some(mut running_server) = running.take() else {
        return;
    };

    if let Some(shutdown) = running_server.shutdown.take() {
        let _ = shutdown.send(());
    }
    if let Some(thread) = running_server.thread.take() {
        let _ = thread.join();
    }
}

fn run_agent_api_server(
    bind_addr: String,
    state: AgentApiState,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    startup_tx: std::sync::mpsc::Sender<Result<String, String>>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = startup_tx.send(Err(format!(
                "Failed to create Tokio runtime for agent API: {err}"
            )));
            return;
        }
    };

    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => listener,
            Err(err) => {
                let _ = startup_tx.send(Err(format!(
                    "Failed to bind local agent API server on {bind_addr}: {err}"
                )));
                return;
            }
        };

        let base_url = listener
            .local_addr()
            .map(|addr| format!("http://{addr}"))
            .unwrap_or_else(|_| format!("http://{bind_addr}"));
        let _ = startup_tx.send(Ok(base_url));

        let app = routes::build_router(state);
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });

        if let Err(err) = server.await {
            eprintln!("Loopbox agent API server stopped: {err}");
        }
    });
}

fn lock_mutation(state: &AgentApiState) -> Result<std::sync::MutexGuard<'_, ()>, ApiError> {
    state
        .mutation_lock
        .lock()
        .map_err(|_| ApiError::internal("Agent API mutation lock poisoned."))
}

fn load_config_api() -> Result<LoopboxConfig, ApiError> {
    load_config().map_err(|err| ApiError::internal(format!("Failed to load config: {err}")))
}

fn get_project<'a>(
    config: &'a LoopboxConfig,
    project_name: &str,
) -> Result<&'a ProjectConfig, ApiError> {
    config
        .projects
        .get(project_name)
        .ok_or_else(|| ApiError::not_found(format!("Project '{project_name}' not found.")))
}

fn get_service<'a>(
    project: &'a ProjectConfig,
    service_name: &str,
) -> Result<&'a ServiceConfig, ApiError> {
    let normalized = service_name.trim();
    project
        .services
        .iter()
        .find(|service| service.name == normalized)
        .ok_or_else(|| ApiError::not_found(format!("Service '{normalized}' not found.")))
}

fn clamp_limit(value: Option<usize>, default: usize, max: usize) -> usize {
    value.unwrap_or(default).clamp(1, max)
}

fn build_project_detail_response(
    config: &LoopboxConfig,
    project_name: &str,
) -> Result<ProjectDetailResponse, ApiError> {
    let project = get_project(config, project_name)?.clone();
    let services = service_details_for_project(config, project_name, project.services.as_slice());
    Ok(ProjectDetailResponse {
        name: project_name.to_string(),
        primary_host: project_primary_host(config, project_name),
        config: project.clone(),
        services,
        capture_enabled: project_proxy_traffic_enabled(config, project_name),
        capture_mode: capture_mode_label(project_proxy_traffic_capture_mode(config, project_name)),
    })
}

fn project_create_request_to_input(request: ProjectCreateRequest) -> AddProjectInput {
    AddProjectInput {
        name: request.name,
        dir: request.dir,
        ip: request.ip,
        services: request
            .services
            .into_iter()
            .map(project_service_request_to_entry)
            .collect(),
    }
}

fn project_update_request_to_input(request: ProjectUpdateRequest) -> UpdateProjectInput {
    UpdateProjectInput {
        dir: request.dir,
        ip: request.ip,
        services: request
            .services
            .into_iter()
            .map(project_service_request_to_entry)
            .collect(),
    }
}

fn project_service_request_to_entry(request: ProjectServiceRequest) -> ServiceEntry {
    let default_protocol = request.protocol.unwrap_or(ProxyEndpointProtocol::Http1);
    let mut ports = Vec::with_capacity(request.ports.len());
    for port in request.ports {
        let protocol = port.protocol.unwrap_or(default_protocol.clone());
        ports.push(ServicePortEntry {
            port: port.port.to_string(),
            protocol: proxy_endpoint_protocol_label(&protocol).to_string(),
            health_path: port.health_path.unwrap_or_default(),
        });
    }

    if ports.is_empty() {
        if let Some(port) = request.port {
            ports.push(ServicePortEntry {
                port: port.to_string(),
                protocol: proxy_endpoint_protocol_label(&default_protocol).to_string(),
                health_path: request.health_path.clone().unwrap_or_default(),
            });
        }
    }

    let (legacy_port, legacy_protocol, legacy_health_path) = if let Some(first) = ports.first() {
        (
            first.port.clone(),
            first.protocol.clone(),
            first.health_path.clone(),
        )
    } else {
        (
            String::new(),
            proxy_endpoint_protocol_label(&default_protocol).to_string(),
            request.health_path.clone().unwrap_or_default(),
        )
    };

    let runtime = request.runtime.unwrap_or(ServiceRuntimeKind::Process);
    let (container_image, container_args, container_env, container_volumes, container_auto_remove) =
        if let Some(container) = request.container {
            (
                container.image,
                container.args.join(", "),
                container.env.join(", "),
                container.volumes.join(", "),
                container.auto_remove,
            )
        } else {
            (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                false,
            )
        };

    ServiceEntry {
        name: request.name,
        ports,
        port: legacy_port,
        protocol: legacy_protocol,
        runtime: service_runtime_kind_label(runtime).to_string(),
        command: request.command,
        workdir: request.workdir,
        env_files: request.env_files.join(", "),
        depends_on: request.depends_on.join(", "),
        autostart: request.autostart,
        health_path: legacy_health_path,
        container_image,
        container_args,
        container_env,
        container_volumes,
        container_auto_remove,
    }
}

fn proxy_endpoint_protocol_label(protocol: &ProxyEndpointProtocol) -> &'static str {
    match protocol {
        ProxyEndpointProtocol::Http1 => "http1",
        ProxyEndpointProtocol::GrpcH2c => "grpc_h2c",
        ProxyEndpointProtocol::TcpPassthrough => "tcp_passthrough",
    }
}

fn service_runtime_kind_label(runtime: ServiceRuntimeKind) -> &'static str {
    match runtime {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    }
}

fn map_project_config_mutation_error(err: String) -> ApiError {
    let normalized = err.to_ascii_lowercase();
    if normalized.contains("already exists") || normalized.contains("already assigned") {
        return ApiError::conflict(err);
    }
    if normalized.contains("does not exist") || normalized.contains("not found") {
        return ApiError::not_found(err);
    }
    ApiError::bad_request(err)
}

fn persist_project_config_mutation(
    config: &LoopboxConfig,
    apply_system: bool,
) -> Result<ProjectConfigPersistOutcome, ApiError> {
    let path = save_config(config)
        .map_err(|err| ApiError::internal(format!("Failed to save config: {err}")))?;
    sync_reverse_proxy(config)
        .map_err(|err| ApiError::conflict(format!("Failed to sync reverse proxy: {err}")))?;

    let system_setup_message =
        if apply_system {
            Some(apply_system_setup(config).map_err(|err| {
                ApiError::conflict(format!("Failed to apply system setup: {err}"))
            })?)
        } else {
            None
        };

    Ok(ProjectConfigPersistOutcome {
        saved_config_path: path.display().to_string(),
        system_setup_message,
    })
}

fn capture_mode_label(mode: ProxyCaptureMode) -> &'static str {
    match mode {
        ProxyCaptureMode::Metadata => "metadata",
        ProxyCaptureMode::Headers => "headers",
        ProxyCaptureMode::BodyPreview => "body_preview",
    }
}

fn snapshots_to_dtos(
    project_name: &str,
    project_services: &[ServiceConfig],
    snapshots: Vec<ServiceRuntimeSnapshot>,
) -> Result<Vec<ServiceRuntimeDto>, ApiError> {
    let mut out = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let attached = service_log_attached(project_name, &snapshot.service).unwrap_or(false);
        out.push(ServiceRuntimeDto {
            service: snapshot.service,
            state: RuntimeStateDto::from(snapshot.state),
            pid: snapshot.pid,
            started_at: snapshot.started_at,
            exit_code: snapshot.exit_code,
            last_error: snapshot.last_error,
            log_attached: attached,
        });
    }
    if out.is_empty() && !project_services.is_empty() {
        return Err(ApiError::internal(
            "Runtime action returned no service snapshots unexpectedly.",
        ));
    }
    Ok(out)
}

fn runtime_snapshot_dtos(
    config: &LoopboxConfig,
    project_name: &str,
    services: &[ServiceConfig],
) -> Result<Vec<ServiceRuntimeDto>, ApiError> {
    let mut snapshots = Vec::with_capacity(services.len());
    for service in services {
        let snapshot = service_runtime_status(config, project_name, &service.name)
            .map_err(|err| ApiError::internal(format!("Failed to read runtime status: {err}")))?;
        let attached = service_log_attached(project_name, &service.name).unwrap_or(false);
        snapshots.push(ServiceRuntimeDto {
            service: service.name.clone(),
            state: RuntimeStateDto::from(snapshot.state),
            pid: snapshot.pid,
            started_at: snapshot.started_at,
            exit_code: snapshot.exit_code,
            last_error: snapshot.last_error,
            log_attached: attached,
        });
    }
    Ok(snapshots)
}

fn project_runtime_counts(
    config: &LoopboxConfig,
    project_name: &str,
    project: &ProjectConfig,
) -> RuntimeCounts {
    let mut counts = RuntimeCounts::default();
    for service in &project.services {
        match service_runtime_status(config, project_name, &service.name) {
            Ok(snapshot) => match snapshot.state {
                ServiceRuntimeState::Running => counts.running += 1,
                ServiceRuntimeState::Starting => counts.starting += 1,
                ServiceRuntimeState::Unhealthy => counts.unhealthy += 1,
                ServiceRuntimeState::Crashed => counts.crashed += 1,
                ServiceRuntimeState::Stopped => counts.stopped += 1,
            },
            Err(_) => counts.unknown += 1,
        }
    }
    counts
}

fn service_details_for_project(
    config: &LoopboxConfig,
    project_name: &str,
    services: &[ServiceConfig],
) -> Vec<ProjectServiceDetail> {
    let suffix = config.global.domain_suffix.trim().trim_start_matches('.');
    let mut out = Vec::with_capacity(services.len());
    for service in services {
        let effective_ports = service_ports(service);
        let primary_port = effective_ports
            .iter()
            .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
            .map(|entry| entry.port)
            .or_else(|| effective_ports.first().map(|entry| entry.port));
        let host = format!(
            "{}.{}.{}",
            service.name.trim().to_lowercase(),
            project_name.trim().to_lowercase(),
            suffix
        );
        let url = super::open_url_for(
            config,
            project_name,
            OpenTarget::Service(service.name.clone()),
        )
        .unwrap_or_else(|_| {
            if let Some(port) = primary_port {
                format!("http://{host}:{port}")
            } else {
                format!("http://{host}")
            }
        });
        out.push(ProjectServiceDetail {
            name: service.name.clone(),
            port: primary_port,
            host,
            url,
            command: service.command.clone(),
            workdir: service.workdir.clone(),
        });
    }
    out
}

fn agent_api_base_dir() -> PathBuf {
    config_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".loopbox"))
}

fn upsert_agent_guidance_section(existing: &str, section: &str) -> String {
    let start = existing.find(AGENT_GUIDANCE_START_MARKER);
    let end = existing.find(AGENT_GUIDANCE_END_MARKER);
    if let (Some(start_idx), Some(end_idx)) = (start, end) {
        if start_idx < end_idx {
            let prefix = existing[..start_idx].trim_end();
            let suffix = existing[end_idx + AGENT_GUIDANCE_END_MARKER.len()..].trim_start();
            return match (prefix.is_empty(), suffix.is_empty()) {
                (true, true) => section.to_string(),
                (false, true) => format!("{prefix}\n\n{section}"),
                (true, false) => format!("{section}\n{suffix}"),
                (false, false) => format!("{prefix}\n\n{section}\n{suffix}"),
            };
        }
    }

    let trimmed = existing.trim_end();
    if trimmed.is_empty() {
        section.to_string()
    } else {
        format!("{trimmed}\n\n{section}")
    }
}

fn write_discovery_file(path: &FsPath, info: &AgentApiServerInfo) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let payload = DiscoveryPayload {
        schema: "loopbox_agent_api_discovery_v1",
        enabled: info.enabled,
        running: info.running,
        auth_enabled: info.auth_enabled,
        bind_port: info.bind_port,
        base_url: info.base_url.clone(),
        openapi_url: info.openapi_url.clone(),
        token_path: info.token_path.clone(),
        api_version: AGENT_API_VERSION,
        generated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let body = serde_json::to_string_pretty(&payload)
        .map_err(|err| format!("Failed to serialize agent API discovery payload: {err}"))?;
    fs::write(path, body).map_err(|err| {
        format!(
            "Failed to write agent API discovery file {}: {err}",
            path.display()
        )
    })
}

fn load_or_create_api_token(path: &FsPath) -> Result<String, String> {
    if path.exists() {
        let existing = fs::read_to_string(path)
            .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
        let token = existing.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let token = generate_api_token();
    fs::write(path, format!("{token}\n"))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    crate::platform::runtime::secure_file_permissions(path)?;
    Ok(token)
}

fn generate_api_token() -> String {
    let mut bytes = [0_u8; 32];
    if fill_token_from_os_rng(&mut bytes) {
        return hex_encode(&bytes);
    }

    let pid = std::process::id() as u64;
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    for (index, slot) in bytes.iter_mut().enumerate() {
        let shifted = now_nanos.rotate_left((index % 64) as u32);
        let mixed = shifted ^ pid.rotate_left((index % 32) as u32) ^ ((index as u64) * 131);
        *slot = (mixed & 0xFF) as u8;
    }
    hex_encode(&bytes)
}

fn fill_token_from_os_rng(out: &mut [u8]) -> bool {
    getrandom::fill(out).is_ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0F));
    }
    out
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '0',
    }
}

fn openapi_spec_json(bind_port: u16, auth_enabled: bool) -> serde_json::Value {
    let mut get_security = Vec::new();
    if auth_enabled {
        get_security.push(json!({ "bearerAuth": [] }));
    }

    let mut project_get = json!({
        "summary": "List projects",
        "responses": { "200": { "description": "Project list" } }
    });
    if auth_enabled {
        project_get["security"] = json!([{ "bearerAuth": [] }]);
    }

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Loopbox Agent API",
            "version": AGENT_API_VERSION,
            "description": "Local HTTP API for Loopbox project introspection and runtime control."
        },
        "servers": [
            {
                "url": format!("http://127.0.0.1:{bind_port}"),
                "description": "Localhost"
            }
        ],
        "paths": {
            format!("/{AGENT_API_VERSION}/health"): {
                "get": {
                    "summary": "Health check",
                    "responses": { "200": { "description": "OK" } }
                }
            },
            format!("/{AGENT_API_VERSION}/openapi.json"): {
                "get": {
                    "summary": "OpenAPI spec",
                    "responses": { "200": { "description": "OpenAPI JSON" } }
                }
            },
            format!("/{AGENT_API_VERSION}/meta"): {
                "get": {
                    "summary": "API metadata",
                    "responses": { "200": { "description": "Metadata" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects"): {
                "get": project_get,
                "post": {
                    "summary": "Create project config",
                    "parameters": [
                        { "name": "apply_system_setup", "in": "query", "required": false, "schema": { "type": "boolean" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    },
                    "responses": { "200": { "description": "Project mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}"): {
                "get": {
                    "summary": "Project details",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Project details" } },
                    "security": get_security
                },
                "put": {
                    "summary": "Update project config",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "apply_system_setup", "in": "query", "required": false, "schema": { "type": "boolean" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    },
                    "responses": { "200": { "description": "Project mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime"): {
                "get": {
                    "summary": "Project runtime",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Runtime" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/logs"): {
                "get": {
                    "summary": "Service logs",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "minimum": 1, "maximum": 2000 } }
                    ],
                    "responses": { "200": { "description": "Log lines" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/requests"): {
                "get": {
                    "summary": "Captured requests",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "minimum": 1, "maximum": 2000 } }
                    ],
                    "responses": { "200": { "description": "Traffic events" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/start"): {
                "post": {
                    "summary": "Start all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/stop"): {
                "post": {
                    "summary": "Stop all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/restart"): {
                "post": {
                    "summary": "Restart all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/start"): {
                "post": {
                    "summary": "Start one service",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/stop"): {
                "post": {
                    "summary": "Stop one service",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/restart"): {
                "post": {
                    "summary": "Restart one service",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "Mutation result" } },
                    "security": get_security
                }
            }
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer"
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_includes_documented_routes() {
        let spec = openapi_spec_json(39_393, true);
        let paths = spec["paths"].as_object().expect("paths object");

        let expected_paths = [
            (format!("/{AGENT_API_VERSION}/health"), vec!["get"]),
            (format!("/{AGENT_API_VERSION}/openapi.json"), vec!["get"]),
            (format!("/{AGENT_API_VERSION}/meta"), vec!["get"]),
            (
                format!("/{AGENT_API_VERSION}/projects"),
                vec!["get", "post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}"),
                vec!["get", "put"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime"),
                vec!["get"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/logs"),
                vec!["get"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/requests"),
                vec!["get"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/start"),
                vec!["post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/stop"),
                vec!["post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/restart"),
                vec!["post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/start"),
                vec!["post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/stop"),
                vec!["post"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/restart"),
                vec!["post"],
            ),
        ];

        for (path, methods) in expected_paths {
            let operations = paths
                .get(&path)
                .and_then(|value| value.as_object())
                .unwrap_or_else(|| panic!("missing OpenAPI path {path}"));
            for method in methods {
                assert!(
                    operations
                        .get(method)
                        .map(serde_json::Value::is_object)
                        .unwrap_or(false),
                    "missing {method} operation for {path}"
                );
            }
            assert!(
                !operations.contains_key("delete"),
                "Agent API must not document delete support for {path}"
            );
        }

        let projects_path = format!("/{AGENT_API_VERSION}/projects");
        let project_path = format!("/{AGENT_API_VERSION}/projects/{{project}}");

        assert_eq!(
            spec["paths"][&projects_path]["post"]["parameters"][0]["name"],
            "apply_system_setup"
        );
        assert_eq!(
            spec["paths"][&project_path]["put"]["parameters"][1]["name"],
            "apply_system_setup"
        );
        assert_eq!(
            spec["components"]["securitySchemes"]["bearerAuth"]["scheme"],
            "bearer"
        );
    }

    #[test]
    fn project_service_request_conversion_preserves_ports_and_runtime() {
        let request = ProjectServiceRequest {
            name: "api".to_string(),
            ports: vec![ProjectServicePortRequest {
                port: 50051,
                protocol: Some(ProxyEndpointProtocol::GrpcH2c),
                health_path: Some("/health".to_string()),
            }],
            port: None,
            protocol: Some(ProxyEndpointProtocol::Http1),
            runtime: Some(ServiceRuntimeKind::Container),
            command: String::new(),
            workdir: "/tmp/demo".to_string(),
            env_files: vec![".env".to_string(), ".env.local".to_string()],
            depends_on: vec!["db".to_string()],
            autostart: true,
            health_path: None,
            container: Some(ContainerServiceConfig {
                image: "ghcr.io/acme/api:latest".to_string(),
                args: vec!["--dev".to_string()],
                env: vec!["RUST_LOG=debug".to_string()],
                volumes: vec!["./data:/data".to_string()],
                auto_remove: true,
            }),
        };

        let entry = project_service_request_to_entry(request);
        assert_eq!(entry.runtime, "container");
        assert_eq!(entry.ports.len(), 1);
        assert_eq!(entry.ports[0].protocol, "grpc_h2c");
        assert_eq!(entry.port, "50051");
        assert_eq!(entry.protocol, "grpc_h2c");
        assert_eq!(entry.env_files, ".env, .env.local");
        assert_eq!(entry.depends_on, "db");
        assert_eq!(entry.container_image, "ghcr.io/acme/api:latest");
        assert_eq!(entry.container_args, "--dev");
        assert_eq!(entry.container_env, "RUST_LOG=debug");
        assert_eq!(entry.container_volumes, "./data:/data");
        assert!(entry.container_auto_remove);
    }

    #[test]
    fn project_service_request_conversion_preserves_legacy_container_runtime() {
        let request = ProjectServiceRequest {
            name: "worker".to_string(),
            ports: vec![],
            port: Some(8080),
            protocol: Some(ProxyEndpointProtocol::TcpPassthrough),
            runtime: Some(ServiceRuntimeKind::Container),
            command: String::new(),
            workdir: "/tmp/demo".to_string(),
            env_files: vec![],
            depends_on: vec![],
            autostart: false,
            health_path: Some("ready".to_string()),
            container: Some(ContainerServiceConfig {
                image: "ghcr.io/acme/worker:dev".to_string(),
                args: vec!["run".to_string(), "--queue=default".to_string()],
                env: vec!["RUST_LOG=info".to_string(), "WORKERS=2".to_string()],
                volumes: vec![
                    "./cache:/cache".to_string(),
                    "./config:/config:ro".to_string(),
                ],
                auto_remove: false,
            }),
        };

        let entry = project_service_request_to_entry(request);
        assert_eq!(entry.runtime, "container");
        assert_eq!(entry.port, "8080");
        assert_eq!(entry.protocol, "tcp_passthrough");
        assert_eq!(entry.health_path, "ready");
        assert_eq!(entry.ports.len(), 1);
        assert_eq!(entry.ports[0].port, "8080");
        assert_eq!(entry.ports[0].protocol, "tcp_passthrough");
        assert_eq!(entry.container_image, "ghcr.io/acme/worker:dev");
        assert_eq!(entry.container_args, "run, --queue=default");
        assert_eq!(entry.container_env, "RUST_LOG=info, WORKERS=2");
        assert_eq!(
            entry.container_volumes,
            "./cache:/cache, ./config:/config:ro"
        );
        assert!(!entry.container_auto_remove);
    }
}
