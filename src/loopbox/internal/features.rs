use std::env;
use std::path::PathBuf;

mod inner;

pub use crate::loopbox::{
    AgentApiAuditBodyEncoding, AgentApiAuditEvent, AgentApiAuditHeader, DoctorLevel, LoopboxConfig,
    ProjectConfig, ProxyCaptureMode, ProxyTrafficDiskStats, ProxyTrafficEvent, ProxyTrafficHeader,
    ServiceConfig, ServiceRuntimeKind,
};
pub use inner::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorIssue {
    pub level: DoctorLevel,
    pub project: Option<String>,
    pub message: String,
    pub fix: Option<String>,
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

pub fn enforce_traffic_capture_mode(selected: ProxyCaptureMode) -> ProxyCaptureMode {
    selected
}
