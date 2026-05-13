use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[macro_use]
mod internal;
mod agent_api;
mod codex_app;
mod codex_protocol;
mod config;
mod diagnostics;
mod discovery;
mod doctor;
mod env;
mod features;
mod incident;
mod install;
mod kubernetes;
mod mcp;
mod projects;
mod proxy;
mod proxy_bridge;
mod release;
mod resource_metrics;
mod runtime;
mod system;
mod updater;

#[allow(unused_imports)]
pub use agent_api::{
    agent_api_audit_events, agent_api_bootstrap_prompt, agent_api_bootstrap_prompt_for_values,
    agent_api_discovery_path, agent_api_token_path, clear_agent_api_audit_events,
    ensure_project_agent_guidance, run_agent_api_subcommand_from_args, start_agent_api_server,
    sync_agent_api_server, sync_reverse_proxy_sidecar, AgentApiServerInfo,
};
#[allow(unused_imports)]
pub use codex_app::{
    codex_agents_accept_request, codex_agents_decline_request, codex_agents_interrupt_turn,
    codex_agents_new_chat, codex_agents_prefill_diagnosis_prompt, codex_agents_prefill_prompt,
    codex_agents_reload_tools, codex_agents_resume_thread, codex_agents_send_diagnosis_message,
    codex_agents_send_message, codex_agents_snapshot, codex_agents_start, codex_agents_stop,
    CodexAgentAuthState, CodexAgentModel, CodexAgentPendingRequest, CodexAgentThreadSummary,
    CodexAgentsSnapshot,
};
pub use codex_protocol::{
    build_notification_line, build_request_line, build_response_line, item_to_transcript,
    parse_jsonrpc_line, CodexInbound, CodexRpcError, CodexTranscriptItem, CodexTranscriptState,
};
#[allow(unused_imports)]
pub use config::{
    config_path, load_config, reset_config_to_default, save_config, update_global_settings,
};
#[allow(unused_imports)]
pub use diagnostics::{
    complete_diagnosis_session, create_diagnosis_session, diagnosis_prompt_for_session,
    diagnosis_sessions, link_diagnosis_session_thread, read_diagnosis_session,
    resolve_diagnosis_session, update_diagnosis_session_status, CreateDiagnosisSessionInput,
    DiagnosisDoctorIssue, DiagnosisEvidenceSnapshot, DiagnosisLogTail, DiagnosisReport,
    DiagnosisRequestSummary, DiagnosisSession, DiagnosisSource, DiagnosisStatus,
};
#[allow(unused_imports)]
pub use discovery::{
    best_command_for_service, detect_project_blueprint, discover_compose_services,
    discover_project_commands, ComposeDiscovery, ComposePortSuggestion, ComposeServiceSuggestion,
    DiscoverySuggestion, ProjectBlueprintKind, ProjectBlueprintSuggestion,
};
pub use doctor::doctor_report;
pub use mcp::run_loopbox_mcp_subcommand_from_args;

pub fn enforce_traffic_capture_mode(mode: ProxyCaptureMode) -> ProxyCaptureMode {
    mode
}
#[allow(unused_imports)]
pub use env::{
    discover_env_files, merge_service_env, parse_env_file, read_env_file_content,
    write_env_file_content, EnvMergeResult, ParsedEnvFile,
};
#[allow(unused_imports)]
pub use incident::{
    incident_timeline_for_project, record_runtime_incident_transition, IncidentEvidence,
    IncidentKind, IncidentSeverity, IncidentTimelineEvent,
};
pub use install::ensure_installed_in_applications;
#[allow(unused_imports)]
pub use kubernetes::{
    cluster_snapshot, cluster_snapshot_for_namespace, cluster_summaries,
    discover_kubernetes_clusters, import_kubernetes_clusters, parse_endpoint_slice_snapshots,
    parse_event_snapshots, parse_ingress_snapshots, parse_namespace_names, parse_node_snapshots,
    parse_pod_snapshots, parse_service_snapshots, parse_workload_snapshots,
    start_cluster_wireguard, stop_cluster_wireguard, wireguard_active_from_show_output,
    KubernetesClusterDiscovery, KubernetesClusterImport, KubernetesClusterSnapshot,
    KubernetesConnectivityState, KubernetesEndpointSliceSnapshot, KubernetesEventSnapshot,
    KubernetesIngressSnapshot, KubernetesNamespaceSnapshot, KubernetesNodeSnapshot,
    KubernetesPodSnapshot, KubernetesServiceSnapshot, KubernetesTopologyEdge,
    KubernetesTopologyNode, KubernetesTopologySnapshot, KubernetesWorkloadSnapshot,
};
#[allow(unused_imports)]
pub use projects::{
    add_project, open_url_for, preview_add_project, project_env_exports, project_primary_host,
    remove_project, update_project,
};
#[allow(unused_imports)]
pub use proxy::{
    clear_proxy_traffic_events_for_project, clear_reverse_proxy_sidecar_status,
    effective_reverse_proxy_status, effective_reverse_proxy_url_for_host,
    export_proxy_traffic_har_for_project, project_proxy_traffic_capture_mode,
    project_proxy_traffic_enabled, proxy_traffic_disk_stats,
    proxy_traffic_events_for_project_with_persisted, record_reverse_proxy_sidecar_status,
    reverse_proxy_fallback_port, reverse_proxy_status, reverse_proxy_url_for_host,
    sync_reverse_proxy, ProxyTrafficDiskStats, ProxyTrafficEvent, ProxyTrafficHeader,
    ReverseProxyStatus,
};
#[allow(unused_imports)]
pub use release::{
    app_version_label, fetch_latest_github_release, is_newer_release_tag, latest_release_page_url,
    LatestReleaseInfo,
};
#[allow(unused_imports)]
pub use resource_metrics::{
    resource_metrics_disk_stats, resource_metrics_latest_for_config,
    resource_metrics_series_for_project, sync_resource_metrics_sampler, ResourceMetricsDiskStats,
    ResourceMetricsSettings, ServiceResourceSample,
};
#[allow(unused_imports)]
pub use runtime::{
    cleanup_stale_runtime_processes, clear_service_logs, kill_service_port_owner,
    open_terminal_attach_for_service, open_terminal_for_service, restart_service,
    run_runtime_subcommand_from_args, send_service_input, send_terminal_client_message,
    service_input_attached, service_log_attached, service_logs, service_logs_tail,
    service_port_conflicts, service_runtime_status, service_terminal_attached, start_project_all,
    start_service, stop_project_all, stop_service, terminal_session_snapshot, ServicePortConflict,
    ServicePortOwner, ServiceRuntimeSnapshot, ServiceRuntimeState, TerminalClientMessage,
    TerminalFrame, TerminalKeyAction, TerminalMods, TerminalMouseKind, TerminalServerMessage,
};
#[allow(unused_imports)]
pub use system::revert_script;
pub use system::{
    apply_script, apply_system_setup, has_changes_outside_managed_block, managed_hosts_block,
    proxy_redirect_configured, proxy_redirect_required, read_hosts_file, revert_system_setup,
    save_hosts_file,
};
#[allow(unused_imports)]
pub use updater::{
    can_check_for_updates, check_for_updates, init_updater, updater_automatic_checks_enabled,
    updater_feed_url, updater_last_checked_utc,
};

pub fn submit_support_ticket(email: &str, subject: &str, text: &str) -> Result<(), String> {
    internal::support::submit_support_ticket(email, subject, text, env!("CARGO_PKG_VERSION"))
}

pub fn docker_runtime_unavailable_message() -> Option<String> {
    internal::runtime_container::docker_runtime_unavailable_message(
        &internal::runtime_container::docker_runtime_status(),
    )
}

pub const HOSTS_BLOCK_BEGIN: &str = "# --- loopbox begin ---";
pub const HOSTS_BLOCK_END: &str = "# --- loopbox end ---";

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
    pub agent_api: AgentApiSettings,
    #[serde(default)]
    pub codex_agents: CodexAgentsSettings,
    #[serde(default)]
    pub proxy_traffic: ProxyTrafficSettings,
    #[serde(default)]
    pub resource_metrics: ResourceMetricsSettings,
    #[serde(default)]
    pub proxy_endpoints: Vec<ProxyEndpointConfig>,
    #[serde(default)]
    pub kubernetes: KubernetesSettings,
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            domain_suffix: default_domain_suffix(),
            ip_base: default_ip_base(),
            ip_range_start: default_ip_range_start(),
            ip_range_end: default_ip_range_end(),
            agent_api: AgentApiSettings::default(),
            codex_agents: CodexAgentsSettings::default(),
            proxy_traffic: ProxyTrafficSettings::default(),
            resource_metrics: ResourceMetricsSettings::default(),
            proxy_endpoints: Vec::new(),
            kubernetes: KubernetesSettings::default(),
            health_check_interval_secs: default_health_check_interval_secs(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KubernetesSettings {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<KubernetesClusterConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesClusterConfig {
    pub name: String,
    #[serde(default)]
    pub provider: KubernetesProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubeconfig_path: Option<String>,
    pub context: String,
    #[serde(default = "default_kubernetes_namespace")]
    pub default_namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wireguard: Option<WireGuardTunnelConfig>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KubernetesProvider {
    #[default]
    KubeconfigContext,
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireGuardTunnelConfig {
    pub name: String,
    #[serde(default)]
    pub mode: WireGuardMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireGuardMode {
    #[default]
    WgQuick,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexAgentsSettings {
    #[serde(default = "default_codex_agents_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_binary: Option<String>,
    #[serde(default = "default_codex_agents_model")]
    pub default_model: String,
    #[serde(default = "default_codex_agents_effort")]
    pub default_effort: String,
    #[serde(default = "default_codex_agents_sandbox")]
    pub default_sandbox: String,
}

impl Default for CodexAgentsSettings {
    fn default() -> Self {
        Self {
            enabled: default_codex_agents_enabled(),
            codex_binary: None,
            default_model: default_codex_agents_model(),
            default_effort: default_codex_agents_effort(),
            default_sandbox: default_codex_agents_sandbox(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApiSettings {
    #[serde(default = "default_agent_api_enabled")]
    pub enabled: bool,
    #[serde(default = "default_agent_api_port")]
    pub port: u16,
    #[serde(default = "default_agent_api_auth_enabled")]
    pub auth_enabled: bool,
}

impl Default for AgentApiSettings {
    fn default() -> Self {
        Self {
            enabled: default_agent_api_enabled(),
            port: default_agent_api_port(),
            auth_enabled: default_agent_api_auth_enabled(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApiAuditHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentApiAuditBodyEncoding {
    Utf8,
    Hex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApiAuditEvent {
    pub id: u64,
    pub started_at_unix_ms: u64,
    pub duration_ms: u64,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub matched_path: Option<String>,
    pub request_version: String,
    pub response_version: String,
    pub status_code: u16,
    pub auth_enabled: bool,
    pub authorization_header_present: bool,
    pub request_headers: Vec<AgentApiAuditHeader>,
    pub request_body: String,
    pub request_body_encoding: AgentApiAuditBodyEncoding,
    pub request_body_truncated: bool,
    pub request_body_bytes: usize,
    pub response_headers: Vec<AgentApiAuditHeader>,
    pub response_body: String,
    pub response_body_encoding: AgentApiAuditBodyEncoding,
    pub response_body_truncated: bool,
    pub response_body_bytes: usize,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCaptureMode {
    Metadata,
    Headers,
    BodyPreview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub dir: String,
    pub ip: String,
    pub services: Vec<ServiceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_interval_secs: Option<u64>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceEntry {
    pub name: String,
    pub ports: Vec<ServicePortEntry>,
    pub port: String,
    pub protocol: String,
    pub runtime: String,
    pub command: String,
    pub workdir: String,
    pub env_files: String,
    pub depends_on: String,
    pub autostart: bool,
    pub health_path: String,
    pub container_image: String,
    pub container_args: String,
    pub container_env: String,
    pub container_volumes: String,
    pub container_auto_remove: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServicePortEntry {
    pub port: String,
    pub protocol: String,
    pub health_path: String,
    pub health_check_interval_secs: String,
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
    pub command: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_interval_secs: Option<u64>,
}

pub fn service_ports(service: &ServiceConfig) -> Vec<ServicePortConfig> {
    let mut result = Vec::new();
    let mut seen_ports = std::collections::HashSet::new();

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
            health_check_interval_secs: entry.health_check_interval_secs,
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
                health_check_interval_secs: None,
            });
        }
    }

    result
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddProjectInput {
    pub name: String,
    pub dir: String,
    pub ip: String,
    pub health_check_interval_secs: String,
    pub services: Vec<ServiceEntry>,
}

impl Default for AddProjectInput {
    fn default() -> Self {
        Self {
            name: String::new(),
            dir: String::new(),
            ip: String::new(),
            health_check_interval_secs: String::new(),
            services: vec![
                ServiceEntry {
                    name: "backend".to_string(),
                    ports: vec![ServicePortEntry {
                        port: "8080".to_string(),
                        protocol: "http1".to_string(),
                        health_path: String::new(),
                        health_check_interval_secs: String::new(),
                    }],
                    port: "8080".to_string(),
                    protocol: "http1".to_string(),
                    runtime: "process".to_string(),
                    command: "npm run dev".to_string(),
                    workdir: String::new(),
                    env_files: String::new(),
                    depends_on: String::new(),
                    autostart: false,
                    health_path: String::new(),
                    container_image: String::new(),
                    container_args: String::new(),
                    container_env: String::new(),
                    container_volumes: String::new(),
                    container_auto_remove: false,
                },
                ServiceEntry {
                    name: "frontend".to_string(),
                    ports: vec![ServicePortEntry {
                        port: "5173".to_string(),
                        protocol: "http1".to_string(),
                        health_path: String::new(),
                        health_check_interval_secs: String::new(),
                    }],
                    port: "5173".to_string(),
                    protocol: "http1".to_string(),
                    runtime: "process".to_string(),
                    command: "npm run dev".to_string(),
                    workdir: String::new(),
                    env_files: String::new(),
                    depends_on: String::new(),
                    autostart: false,
                    health_path: String::new(),
                    container_image: String::new(),
                    container_args: String::new(),
                    container_env: String::new(),
                    container_volumes: String::new(),
                    container_auto_remove: false,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateProjectInput {
    pub dir: String,
    pub ip: String,
    pub health_check_interval_secs: String,
    pub services: Vec<ServiceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    Service(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorFixAction {
    ApplySystemSetup,
    CopyCommand { label: String, command: String },
}

impl DoctorFixAction {
    pub fn label(&self) -> &str {
        match self {
            Self::ApplySystemSetup => "Setup System",
            Self::CopyCommand { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorIssue {
    pub level: DoctorLevel,
    pub project: Option<String>,
    pub message: String,
    pub fix: Option<DoctorFixAction>,
}

impl DoctorIssue {
    fn error(project: Option<String>, message: impl Into<String>) -> Self {
        Self {
            level: DoctorLevel::Error,
            project,
            message: message.into(),
            fix: None,
        }
    }

    fn warning(project: Option<String>, message: impl Into<String>) -> Self {
        Self {
            level: DoctorLevel::Warning,
            project,
            message: message.into(),
            fix: None,
        }
    }

    fn warning_with_fix(
        project: Option<String>,
        message: impl Into<String>,
        fix: DoctorFixAction,
    ) -> Self {
        Self {
            level: DoctorLevel::Warning,
            project,
            message: message.into(),
            fix: Some(fix),
        }
    }

    fn info(message: impl Into<String>) -> Self {
        Self {
            level: DoctorLevel::Info,
            project: None,
            message: message.into(),
            fix: None,
        }
    }
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

fn default_health_check_interval_secs() -> u64 {
    10
}

fn default_proxy_capture_enabled_by_default() -> bool {
    false
}

fn default_agent_api_enabled() -> bool {
    true
}

fn default_agent_api_port() -> u16 {
    39_393
}

fn default_agent_api_auth_enabled() -> bool {
    true
}

fn default_codex_agents_enabled() -> bool {
    true
}

fn default_codex_agents_model() -> String {
    "gpt-5.4".to_string()
}

fn default_codex_agents_effort() -> String {
    "medium".to_string()
}

fn default_codex_agents_sandbox() -> String {
    "workspace-write".to_string()
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

fn default_kubernetes_namespace() -> String {
    "default".to_string()
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
