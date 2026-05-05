use super::{
    AgentApiAuditEvent, DoctorIssue, LoopboxConfig, ProjectConfig, ProxyCaptureMode,
    ProxyTrafficDiskStats, ProxyTrafficEvent, ServiceConfig,
};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

fn map_internal_issue(issue: super::internal::features::DoctorIssue) -> DoctorIssue {
    DoctorIssue {
        level: issue.level,
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
    super::internal::features::doctor_service_extra_issues(config, project_name, project, service)
        .into_iter()
        .map(map_internal_issue)
        .collect()
}

pub(crate) fn doctor_requires_start_command(service: &ServiceConfig) -> bool {
    super::internal::features::doctor_requires_start_command(service)
}

pub(crate) fn doctor_global_extra_issues(config: &LoopboxConfig) -> Vec<DoctorIssue> {
    super::internal::features::doctor_global_extra_issues(config)
        .into_iter()
        .map(map_internal_issue)
        .collect()
}

pub(crate) fn project_proxy_traffic_enabled(config: &LoopboxConfig, project_name: &str) -> bool {
    super::internal::features::project_proxy_traffic_enabled(config, project_name)
}

pub(crate) fn project_proxy_traffic_capture_mode(
    config: &LoopboxConfig,
    project_name: &str,
) -> ProxyCaptureMode {
    super::internal::features::project_proxy_traffic_capture_mode(config, project_name)
}

pub(crate) fn proxy_traffic_events_for_project_with_persisted(
    project_name: &str,
    service_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ProxyTrafficEvent>, String> {
    super::internal::features::proxy_traffic_events_for_project_with_persisted(
        project_name,
        service_filter,
        limit,
    )
}

pub(crate) fn clear_proxy_traffic_events_for_project(project_name: &str) -> Result<usize, String> {
    super::internal::features::clear_proxy_traffic_events_for_project(project_name)
}

pub(crate) fn proxy_traffic_disk_stats() -> ProxyTrafficDiskStats {
    super::internal::features::proxy_traffic_disk_stats()
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

pub(crate) async fn run_agent_api_audit_middleware(
    auth_enabled: bool,
    request: Request,
    next: Next,
) -> Response {
    super::internal::features::run_agent_api_audit_middleware(auth_enabled, request, next).await
}

pub(crate) fn agent_api_audit_events(limit: usize) -> Result<Vec<AgentApiAuditEvent>, String> {
    super::internal::features::agent_api_audit_events(limit)
}

pub(crate) fn clear_agent_api_audit_events() -> Result<usize, String> {
    super::internal::features::clear_agent_api_audit_events()
}
