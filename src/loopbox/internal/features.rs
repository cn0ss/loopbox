use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

mod inner;

pub use inner::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorIssue {
    pub level: DoctorLevel,
    pub project: Option<String>,
    pub message: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LoopboxConfig {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub proxy_traffic: ProxyTrafficSettings,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            proxy_traffic: ProxyTrafficSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyTrafficSettings {
    #[serde(default)]
    pub capture_enabled_by_default: bool,
    #[serde(default = "default_proxy_capture_mode")]
    pub capture_mode_default: ProxyCaptureMode,
}

impl Default for ProxyTrafficSettings {
    fn default() -> Self {
        Self {
            capture_enabled_by_default: false,
            capture_mode_default: default_proxy_capture_mode(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_traffic_capture_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_traffic_capture_mode: Option<ProxyCaptureMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    #[serde(default = "default_service_runtime_kind")]
    pub runtime: ServiceRuntimeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerServiceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerServiceConfig {
    pub image: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRuntimeKind {
    Process,
    Container,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCaptureMode {
    Metadata,
    Headers,
    BodyPreview,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyTrafficDiskStats {
    pub dropped_events: u64,
    pub total_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentApiAuditBodyEncoding {
    Utf8,
    Hex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApiAuditHeader {
    pub name: String,
    pub value: String,
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

pub fn config_path() -> PathBuf {
    if let Some(xdg_config_home) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config_home)
            .join("loopbox")
            .join("config.toml");
    }

    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("loopbox")
            .join("config.toml");
    }

    PathBuf::from(".loopbox").join("config.toml")
}

pub fn supports_traffic_capture() -> bool {
    true
}

pub fn supports_traffic_har_export() -> bool {
    true
}

pub fn supports_grpc_proto_decode() -> bool {
    true
}

pub fn enforce_traffic_capture_mode(selected: ProxyCaptureMode) -> ProxyCaptureMode {
    selected
}

fn default_proxy_capture_mode() -> ProxyCaptureMode {
    ProxyCaptureMode::Metadata
}

fn default_service_runtime_kind() -> ServiceRuntimeKind {
    ServiceRuntimeKind::Process
}

fn default_proxy_event_protocol() -> String {
    "http1".to_string()
}
