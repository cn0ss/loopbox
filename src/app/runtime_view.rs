use crate::app::models::RuntimeFilter;
use crate::loopbox::{
    self, ProxyEndpointProtocol, ServiceConfig, ServicePortConfig, ServicePortConflict,
    ServiceResourceSample, ServiceRuntimeKind, ServiceRuntimeSnapshot, ServiceRuntimeState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeServiceActionFlags {
    pub(crate) is_active: bool,
    pub(crate) is_process: bool,
    pub(crate) can_start: bool,
    pub(crate) can_stop: bool,
    pub(crate) can_restart: bool,
    pub(crate) can_open: bool,
    pub(crate) can_terminal: bool,
    pub(crate) can_run: bool,
    pub(crate) can_attach: bool,
    pub(crate) can_send_input: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeServiceRow {
    pub(crate) project_name: String,
    pub(crate) project_ip: String,
    pub(crate) service_name: String,
    pub(crate) service: ServiceConfig,
    pub(crate) snapshot: ServiceRuntimeSnapshot,
    pub(crate) ports: Vec<ServicePortConfig>,
    pub(crate) runtime_kind: ServiceRuntimeKind,
    pub(crate) runtime_label: &'static str,
    pub(crate) execution_label: String,
    pub(crate) port_label: String,
    pub(crate) status_label: String,
    pub(crate) status_class: &'static str,
    pub(crate) log_attached: bool,
    pub(crate) input_attached: bool,
    pub(crate) terminal_attached: bool,
    pub(crate) resources: Option<ServiceResourceSample>,
    pub(crate) port_conflicts: Vec<ServicePortConflict>,
    pub(crate) can_start: bool,
    pub(crate) can_stop: bool,
    pub(crate) can_restart: bool,
    pub(crate) can_open: bool,
    pub(crate) can_terminal: bool,
    pub(crate) can_run: bool,
    pub(crate) can_attach: bool,
    pub(crate) can_send_input: bool,
    pub(crate) can_kill_port_blocker: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RuntimeServiceAttachments {
    pub(crate) log_attached: bool,
    pub(crate) input_attached: bool,
    pub(crate) terminal_attached: bool,
    pub(crate) resources: Option<ServiceResourceSample>,
    pub(crate) port_conflicts: Vec<ServicePortConflict>,
}

pub(crate) fn build_runtime_service_row(
    project_name: &str,
    project_ip: &str,
    service: &ServiceConfig,
    snapshot: ServiceRuntimeSnapshot,
    attachments: RuntimeServiceAttachments,
) -> RuntimeServiceRow {
    let ports = loopbox::service_ports(service);
    let runtime_kind = service.runtime;
    let action_flags = runtime_service_action_flags(
        service,
        snapshot.state,
        attachments.input_attached,
        attachments.terminal_attached,
    );
    let port_conflicts = if action_flags.is_active {
        Vec::new()
    } else {
        attachments.port_conflicts
    };
    let can_kill_port_blocker = port_conflicts
        .iter()
        .any(|conflict| conflict.owner.is_some());

    RuntimeServiceRow {
        project_name: project_name.to_string(),
        project_ip: project_ip.to_string(),
        service_name: service.name.clone(),
        service: service.clone(),
        ports: ports.clone(),
        runtime_kind,
        runtime_label: runtime_kind_label(runtime_kind),
        execution_label: runtime_execution_label(service),
        port_label: runtime_port_label(project_ip, service, &ports),
        status_label: runtime_status_summary(&snapshot),
        status_class: runtime_status_class(snapshot.state),
        snapshot,
        log_attached: attachments.log_attached,
        input_attached: attachments.input_attached,
        terminal_attached: attachments.terminal_attached,
        resources: attachments.resources,
        port_conflicts,
        can_start: action_flags.can_start,
        can_stop: action_flags.can_stop,
        can_restart: action_flags.can_restart,
        can_open: action_flags.can_open,
        can_terminal: action_flags.can_terminal,
        can_run: action_flags.can_run,
        can_attach: action_flags.can_attach,
        can_send_input: action_flags.can_send_input,
        can_kill_port_blocker,
    }
}

pub(crate) fn runtime_service_action_flags(
    service: &ServiceConfig,
    state: ServiceRuntimeState,
    input_attached: bool,
    terminal_attached: bool,
) -> RuntimeServiceActionFlags {
    let is_active = runtime_state_is_active(state);
    let is_process = service.runtime == ServiceRuntimeKind::Process;
    let has_http_port = loopbox::service_ports(service)
        .iter()
        .any(|entry| entry.protocol == ProxyEndpointProtocol::Http1);

    RuntimeServiceActionFlags {
        is_active,
        is_process,
        can_start: !is_active,
        can_stop: is_active,
        can_restart: is_active,
        can_open: has_http_port,
        can_terminal: is_process && (!is_active || terminal_attached),
        can_run: is_process && !is_active,
        can_attach: is_process && is_active && input_attached && !terminal_attached,
        can_send_input: is_process && is_active && input_attached && !terminal_attached,
    }
}

pub(crate) fn runtime_row_matches(
    row: &RuntimeServiceRow,
    filter: RuntimeFilter,
    search: &str,
) -> bool {
    let filter_match = match filter {
        RuntimeFilter::All => true,
        RuntimeFilter::Running => matches!(
            row.snapshot.state,
            ServiceRuntimeState::Running | ServiceRuntimeState::Starting
        ),
        RuntimeFilter::Stopped => row.snapshot.state == ServiceRuntimeState::Stopped,
        RuntimeFilter::Unhealthy => matches!(
            row.snapshot.state,
            ServiceRuntimeState::Unhealthy | ServiceRuntimeState::Crashed
        ),
        RuntimeFilter::Crashed => row.snapshot.state == ServiceRuntimeState::Crashed,
        RuntimeFilter::Containers => row.runtime_kind == ServiceRuntimeKind::Container,
        RuntimeFilter::Processes => row.runtime_kind == ServiceRuntimeKind::Process,
    };
    if !filter_match {
        return false;
    }

    let needle = search.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return true;
    }

    [
        row.project_name.as_str(),
        row.service_name.as_str(),
        row.runtime_label,
        row.execution_label.as_str(),
        row.service.workdir.as_str(),
        row.port_label.as_str(),
        row.status_label.as_str(),
    ]
    .iter()
    .any(|value| value.to_ascii_lowercase().contains(&needle))
}

pub(crate) fn runtime_status_summary(status: &ServiceRuntimeSnapshot) -> String {
    let state = match status.state {
        ServiceRuntimeState::Stopped => "stopped",
        ServiceRuntimeState::Starting => "starting",
        ServiceRuntimeState::Running => "running",
        ServiceRuntimeState::Unhealthy => "unhealthy",
        ServiceRuntimeState::Crashed => "crashed",
    };

    if let Some(pid) = status.pid {
        format!("{state} (pid {pid})")
    } else if let Some(code) = status.exit_code {
        format!("{state} (exit {code})")
    } else if let Some(err) = &status.last_error {
        format!("{state} ({err})")
    } else {
        state.to_string()
    }
}

pub(crate) fn runtime_status_class(state: ServiceRuntimeState) -> &'static str {
    match state {
        ServiceRuntimeState::Running => "status-running",
        ServiceRuntimeState::Starting => "status-starting",
        ServiceRuntimeState::Unhealthy | ServiceRuntimeState::Crashed => "status-danger",
        ServiceRuntimeState::Stopped => "status-stopped",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceMetricKind {
    Cpu,
    Memory,
}

pub(crate) fn format_cpu_percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.1}%"),
        None => "n/a".to_string(),
    }
}

pub(crate) fn format_memory_bytes(value: Option<u64>) -> String {
    let Some(bytes) = value else {
        return "n/a".to_string();
    };
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kib = bytes as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{kib:.1} KB");
    }
    let mib = kib / 1024.0;
    if mib < 1024.0 {
        return format!("{mib:.1} MB");
    }
    let gib = mib / 1024.0;
    format!("{gib:.1} GB")
}

pub(crate) fn format_sample_age(
    now_unix_ms: u64,
    sample: Option<&ServiceResourceSample>,
) -> String {
    let Some(sample) = sample else {
        return "no samples".to_string();
    };
    let age_ms = now_unix_ms.saturating_sub(sample.sampled_at_unix_ms);
    let age_seconds = age_ms / 1000;
    if age_seconds < 60 {
        return format!("{age_seconds}s ago");
    }
    let age_minutes = age_seconds / 60;
    if age_minutes < 60 {
        return format!("{age_minutes}m ago");
    }
    let age_hours = age_minutes / 60;
    if age_hours < 48 {
        return format!("{age_hours}h ago");
    }
    format!("{}d ago", age_hours / 24)
}

pub(crate) fn resource_sparkline_points(
    samples: &[crate::loopbox::ServiceResourceSample],
    kind: ResourceMetricKind,
    width: f64,
    height: f64,
) -> String {
    let values = samples
        .iter()
        .filter_map(|sample| match kind {
            ResourceMetricKind::Cpu => sample.cpu_percent,
            ResourceMetricKind::Memory => sample.memory_bytes.map(|value| value as f64),
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return String::new();
    }

    let max_value = values.iter().copied().fold(0.0_f64, f64::max).max(1.0);
    let last_index = values.len().saturating_sub(1).max(1) as f64;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = if values.len() == 1 {
                width
            } else {
                width * index as f64 / last_index
            };
            let normalized = (value / max_value).clamp(0.0, 1.0);
            let y = height - normalized * height;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn runtime_state_is_active(state: ServiceRuntimeState) -> bool {
    matches!(
        state,
        ServiceRuntimeState::Running
            | ServiceRuntimeState::Starting
            | ServiceRuntimeState::Unhealthy
    )
}

fn runtime_kind_label(kind: ServiceRuntimeKind) -> &'static str {
    match kind {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    }
}

fn runtime_execution_label(service: &ServiceConfig) -> String {
    match service.runtime {
        ServiceRuntimeKind::Process => format!("$ {}", service.command),
        ServiceRuntimeKind::Container => {
            let image = service
                .container
                .as_ref()
                .map(|container| container.image.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("<missing image>");
            format!("image {image}")
        }
    }
}

fn runtime_port_label(
    project_ip: &str,
    service: &ServiceConfig,
    ports: &[ServicePortConfig],
) -> String {
    if ports.is_empty() {
        return "no port".to_string();
    }

    if service.runtime == ServiceRuntimeKind::Container {
        return ports
            .iter()
            .map(|entry| format!("{project_ip}:{}->{}", entry.port, entry.port))
            .collect::<Vec<_>>()
            .join(", ");
    }

    ports
        .iter()
        .map(|entry| format!(":{}/{}", entry.port, protocol_label(&entry.protocol)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn protocol_label(protocol: &ProxyEndpointProtocol) -> &'static str {
    match protocol {
        ProxyEndpointProtocol::Http1 => "http1",
        ProxyEndpointProtocol::GrpcH2c => "grpc_h2c",
        ProxyEndpointProtocol::TcpPassthrough => "tcp_passthrough",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_metric_formatters_are_compact_and_stable() {
        assert_eq!(format_cpu_percent(Some(0.42)), "0.4%");
        assert_eq!(format_cpu_percent(Some(142.5)), "142.5%");
        assert_eq!(format_cpu_percent(None), "n/a");

        assert_eq!(format_memory_bytes(Some(512)), "512 B");
        assert_eq!(format_memory_bytes(Some(1536)), "1.5 KB");
        assert_eq!(format_memory_bytes(Some(42 * 1024 * 1024)), "42.0 MB");
        assert_eq!(format_memory_bytes(None), "n/a");
    }

    #[test]
    fn resource_trend_points_normalize_cpu_values_for_sparkline() {
        let samples = vec![
            crate::loopbox::ServiceResourceSample {
                project_name: "demo".to_string(),
                service_name: "web".to_string(),
                sampled_at_unix_ms: 1,
                sampled_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
                runtime: ServiceRuntimeKind::Process,
                state: ServiceRuntimeState::Running,
                pid: Some(123),
                cpu_percent: Some(0.0),
                memory_bytes: Some(1024),
                process_count: Some(1),
                container_name: None,
                unavailable_reason: None,
            },
            crate::loopbox::ServiceResourceSample {
                project_name: "demo".to_string(),
                service_name: "web".to_string(),
                sampled_at_unix_ms: 2,
                sampled_at_utc: "2026-05-05 12:00:05 UTC".to_string(),
                runtime: ServiceRuntimeKind::Process,
                state: ServiceRuntimeState::Running,
                pid: Some(123),
                cpu_percent: Some(50.0),
                memory_bytes: Some(2048),
                process_count: Some(1),
                container_name: None,
                unavailable_reason: None,
            },
            crate::loopbox::ServiceResourceSample {
                project_name: "demo".to_string(),
                service_name: "web".to_string(),
                sampled_at_unix_ms: 3,
                sampled_at_utc: "2026-05-05 12:00:10 UTC".to_string(),
                runtime: ServiceRuntimeKind::Process,
                state: ServiceRuntimeState::Running,
                pid: Some(123),
                cpu_percent: Some(100.0),
                memory_bytes: Some(4096),
                process_count: Some(1),
                container_name: None,
                unavailable_reason: None,
            },
        ];

        let points = resource_sparkline_points(&samples, ResourceMetricKind::Cpu, 120.0, 32.0);

        assert!(points.starts_with("0.0,32.0 "));
        assert!(points.contains("60.0,16.0"));
        assert!(points.ends_with("120.0,0.0"));
    }

    fn process_service() -> ServiceConfig {
        ServiceConfig {
            name: "web".to_string(),
            runtime: ServiceRuntimeKind::Process,
            container: None,
            ports: vec![ServicePortConfig {
                port: 5173,
                protocol: ProxyEndpointProtocol::Http1,
                health_path: None,
            }],
            port: Some(5173),
            protocol: ProxyEndpointProtocol::Http1,
            command: "pnpm dev".to_string(),
            workdir: "/tmp/app".to_string(),
            env_files: Vec::new(),
            depends_on: Vec::new(),
            autostart: false,
            health_path: None,
        }
    }

    fn snapshot(state: ServiceRuntimeState) -> ServiceRuntimeSnapshot {
        ServiceRuntimeSnapshot {
            project: "demo".to_string(),
            service: "web".to_string(),
            state,
            pid: None,
            started_at: None,
            exit_code: None,
            last_error: None,
        }
    }

    fn port_conflict() -> crate::loopbox::ServicePortConflict {
        crate::loopbox::ServicePortConflict {
            bind_ip: "127.0.0.30".to_string(),
            port: 5173,
            owner: Some(crate::loopbox::ServicePortOwner {
                pid: 6257,
                command: "server".to_string(),
            }),
        }
    }

    #[test]
    fn stopped_process_rows_expose_port_blocker_actions() {
        let row = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &process_service(),
            snapshot(ServiceRuntimeState::Stopped),
            RuntimeServiceAttachments {
                port_conflicts: vec![port_conflict()],
                ..RuntimeServiceAttachments::default()
            },
        );

        assert_eq!(row.port_conflicts.len(), 1);
        assert!(row.can_kill_port_blocker);
    }

    #[test]
    fn active_process_rows_hide_port_blocker_actions() {
        let mut running = snapshot(ServiceRuntimeState::Running);
        running.pid = Some(6257);

        let row = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &process_service(),
            running,
            RuntimeServiceAttachments {
                port_conflicts: vec![port_conflict()],
                ..RuntimeServiceAttachments::default()
            },
        );

        assert!(row.port_conflicts.is_empty());
        assert!(!row.can_kill_port_blocker);
    }
}
