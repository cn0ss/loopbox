use crate::loopbox::{ProjectConfig, ServiceEntry};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

// ── Navigation ──

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Page {
    Sandboxes,
    NewSandbox,
    Agents,
    Runtime,
    Diagnostics,
    AgentApiAudit,
    System,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(super) enum DetailTab {
    Services,
    Topology,
    Timeline,
    Resources,
    Logs,
    Traffic,
    Environment,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeFilter {
    All,
    Running,
    Stopped,
    Unhealthy,
    Crashed,
    Containers,
    Processes,
}

// ── Notifications ──

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NoticeKind {
    Success,
    Error,
    Info,
}

impl NoticeKind {
    pub(crate) fn class_name(&self) -> &'static str {
        match self {
            Self::Success => "notice-success",
            Self::Error => "notice-error",
            Self::Info => "notice-info",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notice {
    id: u64,
    pub(crate) kind: NoticeKind,
    pub(crate) message: String,
}

impl Notice {
    pub(crate) fn success(message: impl Into<String>) -> Self {
        Self {
            id: next_notice_id(),
            kind: NoticeKind::Success,
            message: message.into(),
        }
    }

    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self {
            id: next_notice_id(),
            kind: NoticeKind::Error,
            message: message.into(),
        }
    }

    pub(crate) fn info(message: impl Into<String>) -> Self {
        Self {
            id: next_notice_id(),
            kind: NoticeKind::Info,
            message: message.into(),
        }
    }

    pub(crate) fn dismiss_after(&self) -> Duration {
        match self.kind {
            NoticeKind::Success | NoticeKind::Info => Duration::from_millis(3_500),
            NoticeKind::Error => Duration::from_millis(8_000),
        }
    }
}

static NEXT_NOTICE_ID: AtomicU64 = AtomicU64::new(1);

fn next_notice_id() -> u64 {
    NEXT_NOTICE_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod notice_tests {
    use super::Notice;
    use std::time::Duration;

    #[test]
    fn success_and_info_notices_dismiss_quickly() {
        assert_eq!(
            Notice::success("Saved.").dismiss_after(),
            Duration::from_millis(3_500)
        );
        assert_eq!(
            Notice::info("Opened.").dismiss_after(),
            Duration::from_millis(3_500)
        );
    }

    #[test]
    fn error_notices_stay_visible_longer() {
        assert_eq!(
            Notice::error("Failed.").dismiss_after(),
            Duration::from_millis(8_000)
        );
    }

    #[test]
    fn notices_with_same_kind_and_message_have_unique_identity() {
        let first = Notice::info("Saved.");
        let second = Notice::info("Saved.");

        assert_ne!(first, second);
    }
}

// ── Setup Status ──

#[derive(Debug, Clone, PartialEq)]
pub(super) enum SetupStatusKind {
    Success,
    Error,
}

impl SetupStatusKind {
    pub(super) fn class_name(&self) -> &'static str {
        match self {
            Self::Success => "setup-status-success",
            Self::Error => "setup-status-error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SetupStatus {
    pub(super) kind: SetupStatusKind,
    pub(super) action: String,
    pub(super) message: String,
    pub(super) timestamp: String,
}

impl SetupStatus {
    pub(super) fn success(action: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: SetupStatusKind::Success,
            action: action.into(),
            message: message.into(),
            timestamp: format_timestamp_utc(),
        }
    }

    pub(super) fn error(action: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: SetupStatusKind::Error,
            action: action.into(),
            message: message.into(),
            timestamp: format_timestamp_utc(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct ProjectEditForm {
    pub(super) dir: String,
    pub(super) ip: String,
    pub(super) services: Vec<ServiceEntry>,
}

impl ProjectEditForm {
    pub(super) fn from_project(project: &ProjectConfig) -> Self {
        let services = project
            .services
            .iter()
            .map(|service| ServiceEntry {
                name: service.name.clone(),
                ports: {
                    let effective_ports = crate::loopbox::service_ports(service);
                    if effective_ports.is_empty() {
                        vec![crate::loopbox::ServicePortEntry {
                            port: String::new(),
                            protocol: "http1".to_string(),
                            health_path: String::new(),
                        }]
                    } else {
                        effective_ports
                            .iter()
                            .map(|entry| crate::loopbox::ServicePortEntry {
                                port: entry.port.to_string(),
                                protocol: match entry.protocol {
                                    crate::loopbox::ProxyEndpointProtocol::Http1 => {
                                        "http1".to_string()
                                    }
                                    crate::loopbox::ProxyEndpointProtocol::GrpcH2c => {
                                        "grpc_h2c".to_string()
                                    }
                                    crate::loopbox::ProxyEndpointProtocol::TcpPassthrough => {
                                        "tcp_passthrough".to_string()
                                    }
                                },
                                health_path: entry.health_path.clone().unwrap_or_default(),
                            })
                            .collect()
                    }
                },
                port: crate::loopbox::service_ports(service)
                    .first()
                    .map(|entry| entry.port.to_string())
                    .unwrap_or_default(),
                protocol: crate::loopbox::service_ports(service)
                    .first()
                    .map(|entry| match entry.protocol {
                        crate::loopbox::ProxyEndpointProtocol::Http1 => "http1".to_string(),
                        crate::loopbox::ProxyEndpointProtocol::GrpcH2c => "grpc_h2c".to_string(),
                        crate::loopbox::ProxyEndpointProtocol::TcpPassthrough => {
                            "tcp_passthrough".to_string()
                        }
                    })
                    .unwrap_or_else(|| "http1".to_string()),
                runtime: match service.runtime {
                    crate::loopbox::ServiceRuntimeKind::Process => "process".to_string(),
                    crate::loopbox::ServiceRuntimeKind::Container => "container".to_string(),
                },
                command: service.command.clone(),
                workdir: service.workdir.clone(),
                env_files: service.env_files.join(", "),
                depends_on: service.depends_on.join(", "),
                autostart: service.autostart,
                health_path: crate::loopbox::service_ports(service)
                    .first()
                    .and_then(|entry| entry.health_path.clone())
                    .unwrap_or_default(),
                container_image: service
                    .container
                    .as_ref()
                    .map(|container| container.image.clone())
                    .unwrap_or_default(),
                container_args: service
                    .container
                    .as_ref()
                    .map(|container| container.args.join(", "))
                    .unwrap_or_default(),
                container_env: service
                    .container
                    .as_ref()
                    .map(|container| container.env.join("\n"))
                    .unwrap_or_default(),
                container_volumes: service
                    .container
                    .as_ref()
                    .map(|container| container.volumes.join("\n"))
                    .unwrap_or_default(),
                container_auto_remove: service
                    .container
                    .as_ref()
                    .map(|container| container.auto_remove)
                    .unwrap_or(false),
            })
            .collect();

        Self {
            dir: project.dir.clone(),
            ip: project.ip.clone(),
            services,
        }
    }
}

fn format_timestamp_utc() -> String {
    let now_secs_u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let now_secs = i64::try_from(now_secs_u64).unwrap_or(i64::MAX);
    format_unix_utc(now_secs)
}

fn format_unix_utc(epoch_seconds: i64) -> String {
    let day_seconds: i64 = 86_400;
    let days = epoch_seconds.div_euclid(day_seconds);
    let day_remainder = epoch_seconds.rem_euclid(day_seconds);

    let hour = day_remainder / 3_600;
    let minute = (day_remainder % 3_600) / 60;
    let second = day_remainder % 60;

    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
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
