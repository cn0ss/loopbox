use super::DoctorLevel;
use super::{
    AgentApiAuditEvent, DoctorIssue, LoopboxConfig, ProjectConfig, ProxyCaptureMode,
    ProxyTrafficDiskStats, ProxyTrafficEvent, ServiceConfig,
};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde::Serialize;

fn convert<T, U>(value: T) -> Result<U, String>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|err| format!("Failed to serialize EE bridge payload: {err}"))?;
    serde_json::from_value(json).map_err(|err| format!("Failed to decode EE bridge payload: {err}"))
}

fn map_private_issue(issue: super::internal::features::DoctorIssue) -> DoctorIssue {
    let level = match issue.level {
        super::internal::features::DoctorLevel::Error => DoctorLevel::Error,
        super::internal::features::DoctorLevel::Warning => DoctorLevel::Warning,
        super::internal::features::DoctorLevel::Info => DoctorLevel::Info,
    };
    DoctorIssue {
        level,
        project: issue.project,
        message: issue.message,
        fix: None,
    }
}

pub(crate) fn doctor_service_extra_issues(
    config: &LoopboxConfig,
    project_name: &str,
    project: &ProjectConfig,
    service: &ServiceConfig,
) -> Vec<DoctorIssue> {
    let cfg = match convert::<_, super::internal::features::LoopboxConfig>(config) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE doctor bridge warning: {err}");
            return Vec::new();
        }
    };
    let proj = match convert::<_, super::internal::features::ProjectConfig>(project) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE doctor bridge warning: {err}");
            return Vec::new();
        }
    };
    let svc = match convert::<_, super::internal::features::ServiceConfig>(service) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE doctor bridge warning: {err}");
            return Vec::new();
        }
    };
    super::internal::features::doctor_service_extra_issues(&cfg, project_name, &proj, &svc)
        .into_iter()
        .map(map_private_issue)
        .collect()
}

pub(crate) fn doctor_requires_start_command(service: &ServiceConfig) -> bool {
    let svc = match convert::<_, super::internal::features::ServiceConfig>(service) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE doctor bridge warning: {err}");
            return true;
        }
    };
    super::internal::features::doctor_requires_start_command(&svc)
}

pub(crate) fn doctor_global_extra_issues(config: &LoopboxConfig) -> Vec<DoctorIssue> {
    let cfg = match convert::<_, super::internal::features::LoopboxConfig>(config) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE doctor bridge warning: {err}");
            return Vec::new();
        }
    };
    super::internal::features::doctor_global_extra_issues(&cfg)
        .into_iter()
        .map(map_private_issue)
        .collect()
}

pub(crate) fn project_proxy_traffic_enabled(config: &LoopboxConfig, project_name: &str) -> bool {
    let cfg = match convert::<_, super::internal::features::LoopboxConfig>(config) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE traffic bridge warning: {err}");
            return false;
        }
    };
    super::internal::features::project_proxy_traffic_enabled(&cfg, project_name)
}

pub(crate) fn project_proxy_traffic_capture_mode(
    config: &LoopboxConfig,
    project_name: &str,
) -> ProxyCaptureMode {
    let cfg = match convert::<_, super::internal::features::LoopboxConfig>(config) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE traffic bridge warning: {err}");
            return ProxyCaptureMode::Metadata;
        }
    };
    let mode = super::internal::features::project_proxy_traffic_capture_mode(&cfg, project_name);
    convert::<_, ProxyCaptureMode>(mode).unwrap_or(ProxyCaptureMode::Metadata)
}

pub(crate) fn proxy_traffic_events_for_project(
    project_name: &str,
    limit: usize,
) -> Result<Vec<ProxyTrafficEvent>, String> {
    let events = super::internal::features::proxy_traffic_events_for_project(project_name, limit)?;
    convert::<_, Vec<ProxyTrafficEvent>>(events)
}

pub(crate) fn proxy_traffic_events_for_project_with_persisted(
    project_name: &str,
    service_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ProxyTrafficEvent>, String> {
    let events = super::internal::features::proxy_traffic_events_for_project_with_persisted(
        project_name,
        service_filter,
        limit,
    )?;
    convert::<_, Vec<ProxyTrafficEvent>>(events)
}

pub(crate) fn clear_proxy_traffic_events_for_project(project_name: &str) -> Result<usize, String> {
    super::internal::features::clear_proxy_traffic_events_for_project(project_name)
}

pub(crate) fn proxy_traffic_disk_stats() -> ProxyTrafficDiskStats {
    let stats = super::internal::features::proxy_traffic_disk_stats();
    ProxyTrafficDiskStats {
        dropped_events: stats.dropped_events,
        total_files: stats.total_files,
        total_bytes: stats.total_bytes,
    }
}

pub(crate) fn export_proxy_traffic_har_for_project(
    project_name: &str,
    service_filter: Option<&str>,
    output_path: &std::path::Path,
) -> Result<usize, String> {
    super::internal::features::export_proxy_traffic_har_for_project(
        project_name,
        service_filter,
        output_path,
    )
}

pub(crate) fn ensure_proxy_traffic_writer_running(
    queue_size: usize,
    retention_days: u16,
    max_storage_mb: usize,
) -> Result<(), String> {
    super::internal::features::ensure_proxy_traffic_writer_running(
        queue_size,
        retention_days,
        max_storage_mb,
    )
}

pub(crate) fn push_proxy_traffic_event(event: ProxyTrafficEvent, max_events: usize) {
    let private_event = match convert::<_, super::internal::features::ProxyTrafficEvent>(event) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Loopbox EE traffic bridge warning: {err}");
            return;
        }
    };
    super::internal::features::push_proxy_traffic_event(private_event, max_events);
}

pub(crate) async fn run_agent_api_audit_middleware(
    auth_enabled: bool,
    request: Request,
    next: Next,
) -> Response {
    super::internal::features::run_agent_api_audit_middleware(auth_enabled, request, next).await
}

pub(crate) fn agent_api_audit_events(limit: usize) -> Result<Vec<AgentApiAuditEvent>, String> {
    let events = super::internal::features::agent_api_audit_events(limit)?;
    convert::<_, Vec<AgentApiAuditEvent>>(events)
}

pub(crate) fn clear_agent_api_audit_events() -> Result<usize, String> {
    super::internal::features::clear_agent_api_audit_events()
}

pub(crate) fn render_grpc_preview(
    bytes: &[u8],
    proto_paths: &[String],
    grpc_service: Option<&str>,
    grpc_method: Option<&str>,
    is_request: bool,
) -> Option<String> {
    super::internal::features::render_grpc_preview(
        bytes,
        proto_paths,
        grpc_service,
        grpc_method,
        is_request,
    )
}
