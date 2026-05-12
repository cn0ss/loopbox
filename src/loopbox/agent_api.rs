use super::{
    add_project, app_version_label, apply_system_setup, clear_reverse_proxy_sidecar_status,
    config_path, doctor_report, effective_reverse_proxy_status, incident_timeline_for_project,
    load_config, project_primary_host, project_proxy_traffic_capture_mode,
    project_proxy_traffic_enabled, proxy_traffic_events_for_project_with_persisted,
    record_reverse_proxy_sidecar_status, resource_metrics_latest_for_config,
    resource_metrics_series_for_project, restart_service, save_config, send_service_input,
    service_input_attached, service_log_attached, service_logs_tail, service_ports,
    service_runtime_status, service_terminal_attached, start_project_all, start_service,
    stop_project_all, stop_service, sync_resource_metrics_sampler, sync_reverse_proxy,
    update_project, AddProjectInput, AgentApiAuditEvent, AgentApiSettings, ContainerServiceConfig,
    DoctorFixAction, DoctorIssue, DoctorLevel, IncidentTimelineEvent, LoopboxConfig, OpenTarget,
    ProjectConfig, ProxyCaptureMode, ProxyEndpointProtocol, ReverseProxyStatus, ServiceConfig,
    ServiceEntry, ServicePortEntry, ServiceResourceSample, ServiceRuntimeKind,
    ServiceRuntimeSnapshot, ServiceRuntimeState, UpdateProjectInput,
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
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_AGENT_API_PORT: u16 = 39_393;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 2_000;
const DEFAULT_REQUEST_LIMIT: usize = 200;
const MAX_REQUEST_LIMIT: usize = 2_000;
const DEFAULT_RESOURCE_METRICS_LIMIT: usize = 1_000;
const MAX_RESOURCE_METRICS_LIMIT: usize = 20_000;
const DEFAULT_INCIDENT_LIMIT: usize = 100;
const MAX_INCIDENT_LIMIT: usize = 500;
const TOKEN_FILE_NAME: &str = "agent-api-token";
const DISCOVERY_FILE_NAME: &str = "agent-api.json";
const AGENT_API_VERSION: &str = "v1";
const AGENT_GUIDANCE_START_MARKER: &str = "<!-- loopbox-agent-api:start -->";
const AGENT_GUIDANCE_END_MARKER: &str = "<!-- loopbox-agent-api:end -->";
const PROXY_SIDECAR_PID_FILE_NAME: &str = "reverse-proxy-sidecar.pid";
const PROXY_SIDECAR_HEARTBEAT_FILE_NAME: &str = "reverse-proxy-sidecar-heartbeat";
const PROXY_SIDECAR_HEARTBEAT_TTL_SECS: u64 = 20;
const PROXY_SIDECAR_LOOP_SECS: u64 = 1;

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
    source: String,
    last_error: Option<String>,
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
struct DoctorResponse {
    issues: Vec<DoctorIssueDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DoctorIssueDto {
    level: &'static str,
    project: Option<String>,
    message: String,
    fix_label: Option<String>,
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
    input_attached: bool,
    terminal_attached: bool,
    resources: Option<ServiceResourceSampleDto>,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceResourceSampleDto {
    project: String,
    service: String,
    sampled_at_unix_ms: u64,
    sampled_at_utc: String,
    runtime: &'static str,
    state: RuntimeStateDto,
    pid: Option<u32>,
    cpu_percent: Option<f64>,
    memory_bytes: Option<u64>,
    process_count: Option<usize>,
    container_name: Option<String>,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Deserialize)]
struct ResourcesQuery {
    service: Option<String>,
    window: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct IncidentsQuery {
    service: Option<String>,
    window: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
struct ServiceInputRequest {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceInputResponse {
    project: String,
    service: String,
    bytes: usize,
    input_attached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceInputTarget {
    project: String,
    service: String,
    text: String,
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
struct ProjectResourcesResponse {
    project: String,
    service: Option<String>,
    window: String,
    limit: usize,
    latest: Vec<ServiceResourceSampleDto>,
    samples: Vec<ServiceResourceSampleDto>,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectIncidentsResponse {
    project: String,
    service: Option<String>,
    window: String,
    limit: usize,
    events: Vec<IncidentTimelineEvent>,
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
    reverse_proxy_synced: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentApiHeadlessArgs {
    run: bool,
    sync_reverse_proxy: bool,
    proxy_keepalive: bool,
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
3) GET /v1/doctor\n\
4) GET /v1/projects\n\
5) GET /v1/projects/{{project}}\n\
6) GET /v1/projects/{{project}}/runtime\n\
7) For debugging: GET /v1/projects/{{project}}/incidents?window=1h&limit=100 before pulling broader evidence\n\
8) Only when needed: GET /v1/projects/{{project}}/logs?service={{svc}}&limit=50\n\
9) Only when needed: GET /v1/projects/{{project}}/requests?service={{svc}}&limit=50\n\
10) Config mutations: POST /v1/projects | PUT /v1/projects/{{project}}\n\
11) Runtime control: POST /v1/projects/{{project}}/start | /stop | /restart\n\
12) Service control: POST /v1/projects/{{project}}/services/{{service}}/start | /stop | /restart | /input\n\n\
Agent behavior:\n\
- Prefer Loopbox-managed hostnames and Loopbox config over guessing ports.\n\
- Prefer project/runtime and incident timeline inspection before reading raw logs, requests, or resources.\n\
- Use runtime input only when a service reports input_attached=true; terminal_attached is UI-only in v1 and terminal frames are not exposed over this API.\n\
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
- the core workflow: health -> meta -> doctor -> projects -> project detail -> runtime -> incidents -> logs/requests/input only when needed"
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

pub fn run_agent_api_subcommand_from_args(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some("__agent_api_server") {
        return None;
    }

    let parsed = match parse_agent_api_headless_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("Loopbox Agent API headless argument error: {err}");
            return Some(2);
        }
    };

    Some(run_agent_api_headless(parsed))
}

fn parse_agent_api_headless_args(args: &[String]) -> Result<AgentApiHeadlessArgs, String> {
    if args.first().map(String::as_str) != Some("__agent_api_server") {
        return Err("Missing __agent_api_server subcommand.".to_string());
    }

    let mut sync_reverse_proxy = false;
    let mut proxy_keepalive = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--sync-reverse-proxy" => sync_reverse_proxy = true,
            "--no-reverse-proxy" => sync_reverse_proxy = false,
            "--proxy-keepalive" => {
                proxy_keepalive = true;
                sync_reverse_proxy = true;
            }
            unknown => {
                return Err(format!("Unknown Agent API headless argument '{unknown}'."));
            }
        }
    }

    Ok(AgentApiHeadlessArgs {
        run: true,
        sync_reverse_proxy,
        proxy_keepalive,
    })
}

fn run_agent_api_headless(args: AgentApiHeadlessArgs) -> i32 {
    if args.proxy_keepalive {
        return run_reverse_proxy_keepalive_loop();
    }

    let config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Loopbox config load warning: {err}");
            LoopboxConfig::default()
        }
    };

    if args.sync_reverse_proxy {
        if let Err(err) = sync_reverse_proxy(&config) {
            eprintln!("Loopbox reverse proxy startup warning: {err}");
        }
    }
    if let Err(err) = sync_resource_metrics_sampler(&config) {
        eprintln!("Loopbox resource metrics startup warning: {err}");
    }

    match sync_agent_api_server(&config) {
        Ok(info) if info.running => {
            let url = info
                .base_url
                .unwrap_or_else(|| format!("http://127.0.0.1:{}", info.bind_port));
            eprintln!("Loopbox agent API headless server listening at {url}.");
            loop {
                std::thread::park_timeout(Duration::from_secs(3600));
            }
        }
        Ok(_) => {
            eprintln!("Loopbox Agent API is disabled in config.");
            1
        }
        Err(err) => {
            eprintln!("Loopbox agent API startup error: {err}");
            1
        }
    }
}

pub fn sync_reverse_proxy_sidecar(config: &LoopboxConfig) -> Result<bool, String> {
    if !config_requires_reverse_proxy_sync(config) {
        return Ok(false);
    }

    write_reverse_proxy_sidecar_heartbeat()?;
    if !should_spawn_reverse_proxy_sidecar(reverse_proxy_sidecar_pid(), |pid| {
        crate::platform::process::pid_exists(pid)
    }) {
        return Ok(true);
    }

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current Loopbox executable: {err}"))?;
    let child = Command::new(current_exe)
        .arg("__agent_api_server")
        .arg("--sync-reverse-proxy")
        .arg("--proxy-keepalive")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Failed to start reverse proxy sidecar: {err}"))?;
    write_reverse_proxy_sidecar_pid(child.id())?;
    Ok(true)
}

fn run_reverse_proxy_keepalive_loop() -> i32 {
    if let Err(err) = write_reverse_proxy_sidecar_pid(std::process::id()) {
        eprintln!("Loopbox reverse proxy sidecar pid warning: {err}");
    }

    loop {
        let status_result = load_config()
            .map_err(|err| format!("Failed to load config: {err}"))
            .and_then(|config| sync_reverse_proxy(&config));
        match status_result {
            Ok(status) => {
                if let Err(err) = record_reverse_proxy_sidecar_status(&status, None) {
                    eprintln!("Loopbox reverse proxy sidecar status warning: {err}");
                }
            }
            Err(err) => {
                let status = ReverseProxyStatus::default();
                if let Err(write_err) =
                    record_reverse_proxy_sidecar_status(&status, Some(err.clone()))
                {
                    eprintln!("Loopbox reverse proxy sidecar status warning: {write_err}");
                }
                eprintln!("Loopbox reverse proxy sidecar sync warning: {err}");
            }
        }

        if !reverse_proxy_sidecar_heartbeat_is_fresh() {
            break;
        }
        thread::sleep(Duration::from_secs(PROXY_SIDECAR_LOOP_SECS));
    }

    let current_pid = std::process::id();
    if reverse_proxy_sidecar_pid() == Some(current_pid) {
        let _ = clear_reverse_proxy_sidecar_pid();
        let _ = clear_reverse_proxy_sidecar_status();
    }

    0
}

fn reverse_proxy_sidecar_pid_path() -> PathBuf {
    config_path()
        .parent()
        .map(|parent| parent.join(PROXY_SIDECAR_PID_FILE_NAME))
        .unwrap_or_else(|| PathBuf::from(PROXY_SIDECAR_PID_FILE_NAME))
}

fn reverse_proxy_sidecar_heartbeat_path() -> PathBuf {
    config_path()
        .parent()
        .map(|parent| parent.join(PROXY_SIDECAR_HEARTBEAT_FILE_NAME))
        .unwrap_or_else(|| PathBuf::from(PROXY_SIDECAR_HEARTBEAT_FILE_NAME))
}

fn reverse_proxy_sidecar_pid() -> Option<u32> {
    fs::read_to_string(reverse_proxy_sidecar_pid_path())
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok())
}

fn write_reverse_proxy_sidecar_pid(pid: u32) -> Result<(), String> {
    let path = reverse_proxy_sidecar_pid_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&path, pid.to_string())
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn clear_reverse_proxy_sidecar_pid() -> Result<(), String> {
    let path = reverse_proxy_sidecar_pid_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Failed to remove {}: {err}", path.display())),
    }
}

fn write_reverse_proxy_sidecar_heartbeat() -> Result<(), String> {
    let path = reverse_proxy_sidecar_heartbeat_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&path, unix_timestamp(SystemTime::now()).to_string())
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn reverse_proxy_sidecar_heartbeat_is_fresh() -> bool {
    fs::read_to_string(reverse_proxy_sidecar_heartbeat_path())
        .ok()
        .and_then(|contents| contents.trim().parse::<u64>().ok())
        .is_some_and(|timestamp| {
            reverse_proxy_sidecar_heartbeat_is_fresh_at(
                timestamp,
                unix_timestamp(SystemTime::now()),
            )
        })
}

fn should_spawn_reverse_proxy_sidecar<F>(pid: Option<u32>, pid_exists: F) -> bool
where
    F: Fn(u32) -> bool,
{
    !pid.is_some_and(pid_exists)
}

fn reverse_proxy_sidecar_heartbeat_is_fresh_at(recorded_at: u64, now: u64) -> bool {
    now >= recorded_at && now.saturating_sub(recorded_at) <= PROXY_SIDECAR_HEARTBEAT_TTL_SECS
}

fn unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
    let reverse_proxy_synced = if config_requires_reverse_proxy_sync(config) {
        sync_reverse_proxy_sidecar(config).map_err(|err| {
            ApiError::conflict(format!("Failed to sync reverse proxy sidecar: {err}"))
        })?;
        true
    } else {
        false
    };
    if let Err(err) = sync_resource_metrics_sampler(config) {
        eprintln!("Loopbox resource metrics sync warning: {err}");
    }

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
        reverse_proxy_synced,
        system_setup_message,
    })
}

fn config_requires_reverse_proxy_sync(config: &LoopboxConfig) -> bool {
    config.projects.values().any(|project| {
        project
            .services
            .iter()
            .any(|service| !service_ports(service).is_empty())
            || !project.proxy_endpoints.is_empty()
    }) || !config.global.proxy_endpoints.is_empty()
}

fn capture_mode_label(mode: ProxyCaptureMode) -> &'static str {
    match mode {
        ProxyCaptureMode::Metadata => "metadata",
        ProxyCaptureMode::Headers => "headers",
        ProxyCaptureMode::BodyPreview => "body_preview",
    }
}

fn doctor_issue_dtos(issues: Vec<DoctorIssue>) -> Vec<DoctorIssueDto> {
    issues
        .into_iter()
        .map(|issue| DoctorIssueDto {
            level: doctor_level_label(&issue.level),
            project: issue.project,
            message: issue.message,
            fix_label: issue
                .fix
                .as_ref()
                .map(DoctorFixAction::label)
                .map(str::to_string),
        })
        .collect()
}

fn doctor_level_label(level: &DoctorLevel) -> &'static str {
    match level {
        DoctorLevel::Error => "error",
        DoctorLevel::Warning => "warning",
        DoctorLevel::Info => "info",
    }
}

fn service_runtime_dto_from_snapshot(
    snapshot: ServiceRuntimeSnapshot,
    log_attached: bool,
    input_attached: bool,
    terminal_attached: bool,
    resources: Option<ServiceResourceSample>,
) -> ServiceRuntimeDto {
    ServiceRuntimeDto {
        service: snapshot.service,
        state: RuntimeStateDto::from(snapshot.state),
        pid: snapshot.pid,
        started_at: snapshot.started_at,
        exit_code: snapshot.exit_code,
        last_error: snapshot.last_error,
        log_attached,
        input_attached,
        terminal_attached,
        resources: resources.map(ServiceResourceSampleDto::from),
    }
}

impl From<ServiceResourceSample> for ServiceResourceSampleDto {
    fn from(value: ServiceResourceSample) -> Self {
        Self {
            project: value.project_name,
            service: value.service_name,
            sampled_at_unix_ms: value.sampled_at_unix_ms,
            sampled_at_utc: value.sampled_at_utc,
            runtime: service_runtime_kind_label(value.runtime),
            state: RuntimeStateDto::from(value.state),
            pid: value.pid,
            cpu_percent: value.cpu_percent,
            memory_bytes: value.memory_bytes,
            process_count: value.process_count,
            container_name: value.container_name,
            unavailable_reason: value.unavailable_reason,
        }
    }
}

fn resolve_service_input_target(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
    request: &ServiceInputRequest,
) -> Result<ServiceInputTarget, ApiError> {
    if request.text.trim().is_empty() {
        return Err(ApiError::bad_request("Input text cannot be empty."));
    }

    let project = get_project(config, project_name)?;
    let service = get_service(project, service_name)?;
    if service.runtime != ServiceRuntimeKind::Process {
        return Err(ApiError::bad_request(format!(
            "Service '{}' uses runtime '{}' and does not support runtime input.",
            service.name,
            service_runtime_kind_label(service.runtime)
        )));
    }

    Ok(ServiceInputTarget {
        project: project_name.to_string(),
        service: service.name.clone(),
        text: request.text.clone(),
    })
}

fn snapshots_to_dtos(
    project_name: &str,
    project_services: &[ServiceConfig],
    snapshots: Vec<ServiceRuntimeSnapshot>,
) -> Result<Vec<ServiceRuntimeDto>, ApiError> {
    let mut out = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let attached = service_log_attached(project_name, &snapshot.service).unwrap_or(false);
        let input_attached =
            service_input_attached(project_name, &snapshot.service).unwrap_or(false);
        let terminal_attached =
            service_terminal_attached(project_name, &snapshot.service).unwrap_or(false);
        out.push(service_runtime_dto_from_snapshot(
            snapshot,
            attached,
            input_attached,
            terminal_attached,
            None,
        ));
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
    let latest_resources = resource_metrics_latest_for_config(config).unwrap_or_default();
    for service in services {
        let snapshot = service_runtime_status(config, project_name, &service.name)
            .map_err(|err| ApiError::internal(format!("Failed to read runtime status: {err}")))?;
        let attached = service_log_attached(project_name, &service.name).unwrap_or(false);
        let input_attached = service_input_attached(project_name, &service.name).unwrap_or(false);
        let terminal_attached =
            service_terminal_attached(project_name, &service.name).unwrap_or(false);
        let resource_key = format!("{project_name}::{}", service.name);
        let resources = latest_resources.get(&resource_key).cloned();
        snapshots.push(service_runtime_dto_from_snapshot(
            snapshot,
            attached,
            input_attached,
            terminal_attached,
            resources,
        ));
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

fn openapi_json_response(schema_name: &str) -> serde_json::Value {
    json!({
        "description": schema_name,
        "content": {
            "application/json": {
                "schema": {
                    "$ref": format!("#/components/schemas/{schema_name}")
                }
            }
        }
    })
}

fn openapi_component_schemas() -> serde_json::Value {
    json!({
        "ApiErrorEnvelope": {
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": { "$ref": "#/components/schemas/ApiErrorBody" }
            }
        },
        "ApiErrorBody": {
            "type": "object",
            "required": ["code", "message"],
            "properties": {
                "code": { "type": "string" },
                "message": { "type": "string" },
                "details": { "type": "object", "nullable": true }
            }
        },
        "HealthResponse": {
            "type": "object",
            "required": ["ok", "api_version", "app_version", "reverse_proxy", "agent_api"],
            "properties": {
                "ok": { "type": "boolean" },
                "api_version": { "type": "string" },
                "app_version": { "type": "string" },
                "reverse_proxy": { "$ref": "#/components/schemas/ReverseProxyInfo" },
                "agent_api": { "$ref": "#/components/schemas/AgentApiHealthInfo" }
            }
        },
        "ReverseProxyInfo": {
            "type": "object",
            "required": ["running", "bind_port", "using_fallback_port", "source"],
            "properties": {
                "running": { "type": "boolean" },
                "bind_port": { "type": "integer", "format": "uint16" },
                "using_fallback_port": { "type": "boolean" },
                "note": { "type": "string", "nullable": true },
                "source": { "type": "string", "enum": ["in_process", "sidecar", "external_probe", "none"] },
                "last_error": { "type": "string", "nullable": true }
            }
        },
        "AgentApiHealthInfo": {
            "type": "object",
            "required": ["auth_enabled", "bind_port"],
            "properties": {
                "auth_enabled": { "type": "boolean" },
                "bind_port": { "type": "integer", "format": "uint16" }
            }
        },
        "MetaResponse": {
            "type": "object",
            "required": ["api_version", "log_limit_default", "log_limit_max", "request_limit_default", "request_limit_max", "auth_enabled", "openapi_url"],
            "properties": {
                "api_version": { "type": "string" },
                "log_limit_default": { "type": "integer" },
                "log_limit_max": { "type": "integer" },
                "request_limit_default": { "type": "integer" },
                "request_limit_max": { "type": "integer" },
                "auth_enabled": { "type": "boolean" },
                "openapi_url": { "type": "string" }
            }
        },
        "DoctorResponse": {
            "type": "object",
            "required": ["issues"],
            "properties": {
                "issues": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/DoctorIssueDto" }
                }
            }
        },
        "DoctorIssueDto": {
            "type": "object",
            "required": ["level", "message"],
            "properties": {
                "level": { "type": "string", "enum": ["error", "warning", "info"] },
                "project": { "type": "string", "nullable": true },
                "message": { "type": "string" },
                "fix_label": { "type": "string", "nullable": true }
            }
        },
        "ProjectsResponse": {
            "type": "object",
            "required": ["projects"],
            "properties": {
                "projects": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ProjectSummary" }
                }
            }
        },
        "ProjectSummary": {
            "type": "object",
            "required": ["name", "dir", "ip", "primary_host", "service_count", "status"],
            "properties": {
                "name": { "type": "string" },
                "dir": { "type": "string" },
                "ip": { "type": "string" },
                "primary_host": { "type": "string" },
                "service_count": { "type": "integer" },
                "status": { "$ref": "#/components/schemas/RuntimeCounts" }
            }
        },
        "RuntimeCounts": {
            "type": "object",
            "required": ["running", "starting", "unhealthy", "crashed", "stopped", "unknown"],
            "properties": {
                "running": { "type": "integer" },
                "starting": { "type": "integer" },
                "unhealthy": { "type": "integer" },
                "crashed": { "type": "integer" },
                "stopped": { "type": "integer" },
                "unknown": { "type": "integer" }
            }
        },
        "ProjectDetailResponse": {
            "type": "object",
            "required": ["name", "primary_host", "config", "services", "capture_enabled", "capture_mode"],
            "properties": {
                "name": { "type": "string" },
                "primary_host": { "type": "string" },
                "config": { "type": "object" },
                "services": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ProjectServiceDetail" }
                },
                "capture_enabled": { "type": "boolean" },
                "capture_mode": { "type": "string", "enum": ["metadata", "headers", "body_preview"] }
            }
        },
        "ProjectServiceDetail": {
            "type": "object",
            "required": ["name", "host", "url", "command", "workdir"],
            "properties": {
                "name": { "type": "string" },
                "port": { "type": "integer", "format": "uint16", "nullable": true },
                "host": { "type": "string" },
                "url": { "type": "string" },
                "command": { "type": "string" },
                "workdir": { "type": "string" }
            }
        },
        "ProjectRuntimeResponse": {
            "type": "object",
            "required": ["project", "services"],
            "properties": {
                "project": { "type": "string" },
                "services": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ServiceRuntimeDto" }
                }
            }
        },
        "ServiceRuntimeDto": {
            "type": "object",
            "required": ["service", "state", "log_attached", "input_attached", "terminal_attached"],
            "properties": {
                "service": { "type": "string" },
                "state": { "$ref": "#/components/schemas/RuntimeStateDto" },
                "pid": { "type": "integer", "format": "uint32", "nullable": true },
                "started_at": { "type": "integer", "format": "uint64", "nullable": true },
                "exit_code": { "type": "integer", "nullable": true },
                "last_error": { "type": "string", "nullable": true },
                "log_attached": { "type": "boolean" },
                "input_attached": { "type": "boolean" },
                "terminal_attached": { "type": "boolean" },
                "resources": {
                    "nullable": true,
                    "allOf": [{ "$ref": "#/components/schemas/ServiceResourceSampleDto" }]
                }
            }
        },
        "ServiceResourceSampleDto": {
            "type": "object",
            "required": ["project", "service", "sampled_at_unix_ms", "sampled_at_utc", "runtime", "state"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string" },
                "sampled_at_unix_ms": { "type": "integer", "format": "uint64" },
                "sampled_at_utc": { "type": "string" },
                "runtime": { "type": "string", "enum": ["process", "container"] },
                "state": { "$ref": "#/components/schemas/RuntimeStateDto" },
                "pid": { "type": "integer", "format": "uint32", "nullable": true },
                "cpu_percent": { "type": "number", "format": "double", "nullable": true },
                "memory_bytes": { "type": "integer", "format": "uint64", "nullable": true },
                "process_count": { "type": "integer", "nullable": true },
                "container_name": { "type": "string", "nullable": true },
                "unavailable_reason": { "type": "string", "nullable": true }
            }
        },
        "ProjectResourcesResponse": {
            "type": "object",
            "required": ["project", "window", "limit", "latest", "samples"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string", "nullable": true },
                "window": { "type": "string", "enum": ["15m", "1h", "24h", "7d"] },
                "limit": { "type": "integer" },
                "latest": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ServiceResourceSampleDto" }
                },
                "samples": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ServiceResourceSampleDto" }
                }
            }
        },
        "RuntimeStateDto": {
            "type": "string",
            "enum": ["stopped", "starting", "running", "unhealthy", "crashed"]
        },
        "LogsResponse": {
            "type": "object",
            "required": ["project", "service", "limit", "log_attached", "lines"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string" },
                "limit": { "type": "integer" },
                "log_attached": { "type": "boolean" },
                "lines": { "type": "array", "items": { "type": "string" } }
            }
        },
        "RequestsResponse": {
            "type": "object",
            "required": ["project", "limit", "capture_enabled", "capture_mode", "events"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string", "nullable": true },
                "limit": { "type": "integer" },
                "capture_enabled": { "type": "boolean" },
                "capture_mode": { "type": "string", "enum": ["metadata", "headers", "body_preview"] },
                "events": { "type": "array", "items": { "type": "object" } }
            }
        },
        "ProjectIncidentsResponse": {
            "type": "object",
            "required": ["project", "window", "limit", "events"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string", "nullable": true },
                "window": { "type": "string", "enum": ["15m", "1h", "24h", "7d"] },
                "limit": { "type": "integer" },
                "events": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/IncidentTimelineEvent" }
                }
            }
        },
        "IncidentTimelineEvent": {
            "type": "object",
            "required": ["id", "occurred_at_unix_ms", "occurred_at_utc", "project_name", "severity", "kind", "summary", "evidence", "source"],
            "properties": {
                "id": { "type": "string" },
                "occurred_at_unix_ms": { "type": "integer", "format": "uint64" },
                "occurred_at_utc": { "type": "string" },
                "project_name": { "type": "string" },
                "service_name": { "type": "string", "nullable": true },
                "severity": { "type": "string", "enum": ["info", "warning", "critical"] },
                "kind": { "type": "string", "enum": ["runtime_transition", "traffic_failure", "slow_request", "resource_pressure", "resource_unavailable"] },
                "summary": { "type": "string" },
                "detail": { "type": "string", "nullable": true },
                "evidence": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/IncidentEvidence" }
                },
                "source": { "type": "string" }
            }
        },
        "IncidentEvidence": {
            "type": "object",
            "required": ["type"],
            "properties": {
                "type": { "type": "string", "enum": ["runtime_snapshot", "request_summary", "resource_sample_summary", "log_excerpt"] }
            },
            "additionalProperties": true
        },
        "MutationResponse": {
            "type": "object",
            "required": ["project", "action", "results"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string", "nullable": true },
                "action": { "type": "string", "enum": ["start", "stop", "restart"] },
                "results": { "type": "array", "items": { "$ref": "#/components/schemas/ServiceRuntimeDto" } }
            }
        },
        "ProjectConfigMutationResponse": {
            "type": "object",
            "required": ["project", "action", "saved_config_path", "reverse_proxy_synced", "system_setup_applied", "detail"],
            "properties": {
                "project": { "type": "string" },
                "action": { "type": "string", "enum": ["create", "update"] },
                "saved_config_path": { "type": "string" },
                "reverse_proxy_synced": { "type": "boolean" },
                "system_setup_applied": { "type": "boolean" },
                "system_setup_message": { "type": "string", "nullable": true },
                "detail": { "$ref": "#/components/schemas/ProjectDetailResponse" }
            }
        },
        "ProjectCreateRequest": {
            "type": "object",
            "required": ["name", "dir"],
            "properties": {
                "name": { "type": "string" },
                "dir": { "type": "string" },
                "ip": { "type": "string" },
                "services": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ProjectServiceRequest" }
                }
            }
        },
        "ProjectUpdateRequest": {
            "type": "object",
            "required": ["dir"],
            "properties": {
                "dir": { "type": "string" },
                "ip": { "type": "string" },
                "services": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ProjectServiceRequest" }
                }
            }
        },
        "ProjectServiceRequest": {
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "ports": { "type": "array", "items": { "$ref": "#/components/schemas/ProjectServicePortRequest" } },
                "port": { "type": "integer", "format": "uint16", "nullable": true },
                "protocol": { "$ref": "#/components/schemas/ProxyEndpointProtocol" },
                "runtime": { "type": "string", "enum": ["process", "container"] },
                "command": { "type": "string" },
                "workdir": { "type": "string" },
                "env_files": { "type": "array", "items": { "type": "string" } },
                "depends_on": { "type": "array", "items": { "type": "string" } },
                "autostart": { "type": "boolean" },
                "health_path": { "type": "string", "nullable": true },
                "container": { "$ref": "#/components/schemas/ContainerServiceConfig" }
            }
        },
        "ProjectServicePortRequest": {
            "type": "object",
            "required": ["port"],
            "properties": {
                "port": { "type": "integer", "format": "uint16" },
                "protocol": { "$ref": "#/components/schemas/ProxyEndpointProtocol" },
                "health_path": { "type": "string", "nullable": true }
            }
        },
        "ProxyEndpointProtocol": {
            "type": "string",
            "enum": ["http1", "grpc_h2c", "tcp_passthrough"]
        },
        "ContainerServiceConfig": {
            "type": "object",
            "required": ["image"],
            "properties": {
                "image": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } },
                "env": { "type": "array", "items": { "type": "string" } },
                "volumes": { "type": "array", "items": { "type": "string" } },
                "auto_remove": { "type": "boolean" }
            }
        },
        "ServiceInputRequest": {
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string" }
            }
        },
        "ServiceInputResponse": {
            "type": "object",
            "required": ["project", "service", "bytes", "input_attached"],
            "properties": {
                "project": { "type": "string" },
                "service": { "type": "string" },
                "bytes": { "type": "integer" },
                "input_attached": { "type": "boolean" }
            }
        }
    })
}

fn openapi_spec_json(bind_port: u16, auth_enabled: bool) -> serde_json::Value {
    let mut get_security = Vec::new();
    if auth_enabled {
        get_security.push(json!({ "bearerAuth": [] }));
    }

    let mut project_get = json!({
        "summary": "List projects",
        "responses": { "200": openapi_json_response("ProjectsResponse") }
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
                    "responses": { "200": openapi_json_response("HealthResponse") }
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
                    "responses": { "200": openapi_json_response("MetaResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/doctor"): {
                "get": {
                    "summary": "Doctor issues",
                    "responses": { "200": openapi_json_response("DoctorResponse") },
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
                                "schema": { "$ref": "#/components/schemas/ProjectCreateRequest" }
                            }
                        }
                    },
                    "responses": { "200": openapi_json_response("ProjectConfigMutationResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}"): {
                "get": {
                    "summary": "Project details",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": openapi_json_response("ProjectDetailResponse") },
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
                                "schema": { "$ref": "#/components/schemas/ProjectUpdateRequest" }
                            }
                        }
                    },
                    "responses": { "200": openapi_json_response("ProjectConfigMutationResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime"): {
                "get": {
                    "summary": "Project runtime",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": openapi_json_response("ProjectRuntimeResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/resources"): {
                "get": {
                    "summary": "Project resource metrics",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "window", "in": "query", "required": false, "schema": { "type": "string", "enum": ["15m", "1h", "24h", "7d"], "default": "1h" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "minimum": 1, "maximum": 20000 } }
                    ],
                    "responses": { "200": openapi_json_response("ProjectResourcesResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/incidents"): {
                "get": {
                    "summary": "Project incident timeline",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "window", "in": "query", "required": false, "schema": { "type": "string", "enum": ["15m", "1h", "24h", "7d"], "default": "1h" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "minimum": 1, "maximum": 500 } }
                    ],
                    "responses": { "200": openapi_json_response("ProjectIncidentsResponse") },
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
                    "responses": { "200": openapi_json_response("LogsResponse") },
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
                    "responses": { "200": openapi_json_response("RequestsResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/start"): {
                "post": {
                    "summary": "Start all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": openapi_json_response("MutationResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/stop"): {
                "post": {
                    "summary": "Stop all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": openapi_json_response("MutationResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/restart"): {
                "post": {
                    "summary": "Restart all services",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": { "200": openapi_json_response("MutationResponse") },
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
                    "responses": { "200": openapi_json_response("MutationResponse") },
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
                    "responses": { "200": openapi_json_response("MutationResponse") },
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
                    "responses": { "200": openapi_json_response("MutationResponse") },
                    "security": get_security
                }
            },
            format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/input"): {
                "post": {
                    "summary": "Send input to one service",
                    "parameters": [
                        { "name": "project", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "service", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ServiceInputRequest" }
                            }
                        }
                    },
                    "responses": { "200": openapi_json_response("ServiceInputResponse") },
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
            },
            "schemas": openapi_component_schemas()
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
            (format!("/{AGENT_API_VERSION}/doctor"), vec!["get"]),
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
                format!("/{AGENT_API_VERSION}/projects/{{project}}/resources"),
                vec!["get"],
            ),
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/incidents"),
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
            (
                format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/input"),
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

        let schemas = spec["components"]["schemas"]
            .as_object()
            .expect("schemas object");
        assert!(schemas.contains_key("HealthResponse"));
        assert!(schemas.contains_key("DoctorResponse"));
        assert!(schemas.contains_key("ProjectRuntimeResponse"));
        assert!(schemas.contains_key("ProjectResourcesResponse"));
        assert!(schemas.contains_key("ProjectIncidentsResponse"));
        assert!(schemas.contains_key("IncidentTimelineEvent"));
        assert!(schemas.contains_key("ServiceResourceSampleDto"));
        assert!(schemas.contains_key("ProjectCreateRequest"));
        assert!(schemas.contains_key("ServiceInputRequest"));
        assert_eq!(
            schemas["ServiceRuntimeDto"]["properties"]["input_attached"]["type"],
            "boolean"
        );
        assert_eq!(
            schemas["ServiceRuntimeDto"]["properties"]["terminal_attached"]["type"],
            "boolean"
        );
        assert_eq!(
            schemas["ProjectServiceRequest"]["properties"]["runtime"]["enum"][1],
            "container"
        );
        assert!(schemas["ProjectServiceRequest"]["properties"]
            .as_object()
            .expect("service request properties")
            .contains_key("container"));
        assert_eq!(
            spec["paths"][format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime")]["get"]
                ["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ProjectRuntimeResponse"
        );
        assert_eq!(
            spec["paths"][format!("/{AGENT_API_VERSION}/projects/{{project}}/resources")]["get"]
                ["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ProjectResourcesResponse"
        );
        assert_eq!(
            spec["paths"][format!("/{AGENT_API_VERSION}/projects/{{project}}/incidents")]["get"]
                ["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ProjectIncidentsResponse"
        );
    }

    #[test]
    fn agent_api_headless_subcommand_is_detected_and_parsed() {
        let args = vec!["__agent_api_server".to_string()];
        let parsed = parse_agent_api_headless_args(&args).expect("headless args");

        assert!(parsed.run);
        assert!(!parsed.sync_reverse_proxy);
        assert!(!parsed.proxy_keepalive);
    }

    #[test]
    fn agent_api_headless_subcommand_parses_reverse_proxy_flags_and_rejects_unknown_flags() {
        let sync_args = vec![
            "__agent_api_server".to_string(),
            "--sync-reverse-proxy".to_string(),
        ];
        let parsed = parse_agent_api_headless_args(&sync_args).expect("sync args");
        assert!(parsed.run);
        assert!(parsed.sync_reverse_proxy);
        assert!(!parsed.proxy_keepalive);

        let no_proxy_args = vec![
            "__agent_api_server".to_string(),
            "--sync-reverse-proxy".to_string(),
            "--no-reverse-proxy".to_string(),
        ];
        let parsed = parse_agent_api_headless_args(&no_proxy_args).expect("no proxy args");
        assert!(parsed.run);
        assert!(!parsed.sync_reverse_proxy);
        assert!(!parsed.proxy_keepalive);

        let err = parse_agent_api_headless_args(&[
            "__agent_api_server".to_string(),
            "--unknown".to_string(),
        ])
        .expect_err("unknown flag should fail");
        assert!(err.contains("Unknown Agent API headless argument '--unknown'"));
    }

    #[test]
    fn agent_api_headless_subcommand_parses_proxy_keepalive_mode() {
        let args = vec![
            "__agent_api_server".to_string(),
            "--sync-reverse-proxy".to_string(),
            "--proxy-keepalive".to_string(),
        ];
        let parsed = parse_agent_api_headless_args(&args).expect("proxy keepalive args");

        assert!(parsed.run);
        assert!(parsed.sync_reverse_proxy);
        assert!(parsed.proxy_keepalive);
    }

    #[test]
    fn reverse_proxy_sidecar_spawn_decision_avoids_duplicate_live_pid() {
        assert!(!should_spawn_reverse_proxy_sidecar(Some(42), |pid| pid != 0));
        assert!(should_spawn_reverse_proxy_sidecar(Some(42), |_pid| false));
        assert!(should_spawn_reverse_proxy_sidecar(None, |_pid| true));
    }

    #[test]
    fn reverse_proxy_sidecar_heartbeat_freshness_expires_after_ttl() {
        assert!(reverse_proxy_sidecar_heartbeat_is_fresh_at(100, 119));
        assert!(reverse_proxy_sidecar_heartbeat_is_fresh_at(100, 120));
        assert!(!reverse_proxy_sidecar_heartbeat_is_fresh_at(100, 121));
        assert!(!reverse_proxy_sidecar_heartbeat_is_fresh_at(121, 100));
    }

    #[test]
    fn openapi_auth_security_matches_runtime_auth_mode() {
        let protected_path = format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime");
        let auth_spec = openapi_spec_json(39_393, true);
        assert_eq!(
            auth_spec["paths"][&protected_path]["get"]["security"][0]["bearerAuth"],
            json!([])
        );

        let no_auth_spec = openapi_spec_json(39_393, false);
        assert_eq!(
            no_auth_spec["paths"][&protected_path]["get"]["security"],
            json!([])
        );
        assert!(
            no_auth_spec["paths"][format!("/{AGENT_API_VERSION}/health")]["get"]
                .get("security")
                .is_none()
        );
    }

    #[test]
    fn config_requires_reverse_proxy_sync_only_for_routable_services_or_endpoints() {
        let mut config = LoopboxConfig::default();
        config.projects.insert(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.2".to_string(),
                services: vec![ServiceConfig {
                    name: "worker".to_string(),
                    runtime: ServiceRuntimeKind::Process,
                    container: None,
                    ports: Vec::new(),
                    port: None,
                    protocol: ProxyEndpointProtocol::Http1,
                    command: "cargo run".to_string(),
                    workdir: "/tmp/demo".to_string(),
                    env_files: Vec::new(),
                    depends_on: Vec::new(),
                    autostart: false,
                    health_path: None,
                }],
                default_open_service: Some("worker".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: Vec::new(),
                proxy_endpoints: Vec::new(),
            },
        );

        assert!(!config_requires_reverse_proxy_sync(&config));

        config.projects.get_mut("demo").unwrap().services[0].ports =
            vec![crate::loopbox::ServicePortConfig {
                port: 8080,
                protocol: ProxyEndpointProtocol::Http1,
                health_path: None,
            }];
        assert!(config_requires_reverse_proxy_sync(&config));

        config.projects.get_mut("demo").unwrap().services[0].ports = Vec::new();
        config
            .global
            .proxy_endpoints
            .push(crate::loopbox::ProxyEndpointConfig {
                name: "grpc".to_string(),
                listen_host: "127.0.0.1".to_string(),
                listen_port: 50060,
                protocol: ProxyEndpointProtocol::GrpcH2c,
                upstream_host: "127.0.0.2".to_string(),
                upstream_port: 50051,
                authority: None,
                project_name: None,
                service_name: Some("worker".to_string()),
            });
        assert!(config_requires_reverse_proxy_sync(&config));
    }

    #[test]
    fn doctor_issue_dtos_include_level_project_message_and_fix_label() {
        let issues = doctor_issue_dtos(vec![
            DoctorIssue::warning_with_fix(
                Some("demo".to_string()),
                "Run setup.",
                DoctorFixAction::ApplySystemSetup,
            ),
            DoctorIssue::info("Everything else looks good."),
        ]);

        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].level, "warning");
        assert_eq!(issues[0].project.as_deref(), Some("demo"));
        assert_eq!(issues[0].message, "Run setup.");
        assert_eq!(issues[0].fix_label.as_deref(), Some("Setup System"));
        assert_eq!(issues[1].level, "info");
        assert_eq!(issues[1].project, None);
        assert_eq!(issues[1].message, "Everything else looks good.");
        assert_eq!(issues[1].fix_label, None);
    }

    #[test]
    fn runtime_dto_reports_log_input_and_resource_attachment() {
        let snapshot = ServiceRuntimeSnapshot {
            project: "demo".to_string(),
            service: "web".to_string(),
            state: ServiceRuntimeState::Running,
            pid: Some(123),
            started_at: Some(456),
            exit_code: None,
            last_error: None,
        };
        let sample = ServiceResourceSample {
            project_name: "demo".to_string(),
            service_name: "web".to_string(),
            sampled_at_unix_ms: 1_776_000_000_000,
            sampled_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
            runtime: ServiceRuntimeKind::Process,
            state: ServiceRuntimeState::Running,
            pid: Some(123),
            cpu_percent: Some(13.25),
            memory_bytes: Some(128 * 1024 * 1024),
            process_count: Some(4),
            container_name: None,
            unavailable_reason: None,
        };

        let dto = service_runtime_dto_from_snapshot(snapshot, true, true, true, Some(sample));

        assert_eq!(dto.service, "web");
        assert_eq!(dto.state, RuntimeStateDto::Running);
        assert_eq!(dto.pid, Some(123));
        assert!(dto.log_attached);
        assert!(dto.input_attached);
        assert!(dto.terminal_attached);
        assert_eq!(
            dto.resources.as_ref().and_then(|sample| sample.cpu_percent),
            Some(13.25)
        );
    }

    #[test]
    fn service_input_target_rejects_empty_text_unknown_project_and_unknown_service() {
        let mut config = LoopboxConfig::default();
        config.projects.insert(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.2".to_string(),
                services: vec![ServiceConfig {
                    name: "web".to_string(),
                    runtime: ServiceRuntimeKind::Process,
                    container: None,
                    ports: Vec::new(),
                    port: None,
                    protocol: ProxyEndpointProtocol::Http1,
                    command: "cat".to_string(),
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

        assert_eq!(
            resolve_service_input_target(
                &config,
                "demo",
                "web",
                &ServiceInputRequest {
                    text: "   ".to_string()
                },
            )
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            resolve_service_input_target(
                &config,
                "missing",
                "web",
                &ServiceInputRequest {
                    text: "x".to_string()
                },
            )
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            resolve_service_input_target(
                &config,
                "demo",
                "missing",
                &ServiceInputRequest {
                    text: "x".to_string()
                },
            )
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn service_input_target_rejects_container_services() {
        let mut config = LoopboxConfig::default();
        config.projects.insert(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.2".to_string(),
                services: vec![ServiceConfig {
                    name: "db".to_string(),
                    runtime: ServiceRuntimeKind::Container,
                    container: Some(ContainerServiceConfig {
                        image: "postgres:16".to_string(),
                        args: Vec::new(),
                        env: Vec::new(),
                        volumes: Vec::new(),
                        auto_remove: true,
                    }),
                    ports: Vec::new(),
                    port: None,
                    protocol: ProxyEndpointProtocol::Http1,
                    command: String::new(),
                    workdir: "/tmp/demo".to_string(),
                    env_files: Vec::new(),
                    depends_on: Vec::new(),
                    autostart: false,
                    health_path: None,
                }],
                default_open_service: Some("db".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: Vec::new(),
                proxy_endpoints: Vec::new(),
            },
        );

        let err = resolve_service_input_target(
            &config,
            "demo",
            "db",
            &ServiceInputRequest {
                text: "x".to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("does not support runtime input"));
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
