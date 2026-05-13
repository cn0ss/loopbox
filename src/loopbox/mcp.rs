use super::{
    add_project, doctor_report, incident_timeline_for_project, load_config, open_url_for,
    preview_add_project, project_primary_host, proxy_traffic_events_for_project_with_persisted,
    resource_metrics_series_for_project, restart_service, save_config, send_service_input,
    service_logs_tail, service_ports, service_runtime_status, start_project_all, start_service,
    stop_project_all, stop_service, update_project, AddProjectInput, DoctorLevel, LoopboxConfig,
    OpenTarget, ServiceEntry, ServicePortEntry, UpdateProjectInput,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::service::{ElicitationError, RequestContext, RoleServer};
use rmcp::{tool, tool_router, transport::stdio, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
struct LoopboxMcpServer;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RuntimeArgs {
    #[schemars(description = "Optional Loopbox sandbox/project name")]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LogsArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Service name inside the sandbox")]
    service: String,
    #[schemars(description = "Maximum log lines to return. Loopbox clamps this to 1..500.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RequestsArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Optional service name filter")]
    service: Option<String>,
    #[schemars(description = "Maximum request records to return. Loopbox clamps this to 1..200.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ResourcesArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Optional service name filter")]
    service: Option<String>,
    #[schemars(description = "Metrics window: 15m, 1h, 24h, or 7d")]
    window: Option<String>,
    #[schemars(description = "Maximum samples to return. Loopbox clamps this to 1..200.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct IncidentsArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Optional service name filter")]
    service: Option<String>,
    #[schemars(description = "Incident window: 15m, 1h, 24h, or 7d")]
    window: Option<String>,
    #[schemars(description = "Maximum incident records to return. Loopbox clamps this to 1..500.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ValidateProjectArgs {
    project: McpProjectInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateProjectArgs {
    project: McpProjectInput,
    #[schemars(description = "Optional non-interactive approval override for tests.")]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateProjectArgs {
    #[schemars(description = "Existing Loopbox sandbox/project name")]
    project: String,
    config: McpProjectUpdateInput,
    #[schemars(description = "Optional non-interactive approval override for tests.")]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectMutationArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Optional non-interactive approval override for tests.")]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ServiceMutationArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Service name inside the sandbox")]
    service: String,
    #[schemars(description = "Optional non-interactive approval override for tests.")]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SendInputArgs {
    #[schemars(description = "Loopbox sandbox/project name")]
    project: String,
    #[schemars(description = "Service name inside the sandbox")]
    service: String,
    #[schemars(description = "Text to send to the service stdin/terminal")]
    input: String,
    #[schemars(description = "Optional non-interactive approval override for tests.")]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpProjectInput {
    name: String,
    dir: String,
    ip: Option<String>,
    health_check_interval_secs: Option<u64>,
    services: Vec<McpServiceInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpProjectUpdateInput {
    dir: String,
    ip: Option<String>,
    health_check_interval_secs: Option<u64>,
    services: Vec<McpServiceInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpServiceInput {
    name: String,
    command: String,
    #[schemars(description = "Working directory relative to the project dir, or absolute path")]
    workdir: Option<String>,
    ports: Option<Vec<McpPortInput>>,
    runtime: Option<String>,
    env_files: Option<Vec<String>>,
    depends_on: Option<Vec<String>>,
    autostart: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpPortInput {
    port: u16,
    protocol: Option<String>,
    health_path: Option<String>,
    health_check_interval_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct MutationConfirmation {
    #[schemars(description = "Set true to approve the Loopbox mutation")]
    confirmed: bool,
}

rmcp::elicit_safe!(MutationConfirmation);

#[tool_router(server_handler)]
impl LoopboxMcpServer {
    #[tool(
        description = "Return a Loopbox overview with projects, primary hostnames, runtime counts, and doctor status."
    )]
    fn loopbox_overview(&self, _args: Parameters<EmptyArgs>) -> CallToolResult {
        tool_value(loopbox_overview_value())
    }

    #[tool(description = "Run Loopbox doctor checks and return structured issues.")]
    fn loopbox_doctor(&self, _args: Parameters<EmptyArgs>) -> CallToolResult {
        tool_value(loopbox_doctor_value())
    }

    #[tool(
        description = "List Loopbox sandboxes/projects with service counts and primary hostnames."
    )]
    fn loopbox_list_projects(&self, _args: Parameters<EmptyArgs>) -> CallToolResult {
        tool_value(load_config().map(|config| {
            json!({
                "projects": config.projects.iter().map(|(name, project)| {
                    project_summary(&config, name, project)
                }).collect::<Vec<_>>()
            })
        }))
    }

    #[tool(description = "Read one Loopbox sandbox/project config and computed service URLs.")]
    fn loopbox_read_project(&self, args: Parameters<ProjectArgs>) -> CallToolResult {
        tool_value(load_config().and_then(|config| {
            let project = config
                .projects
                .get(&args.0.project)
                .ok_or_else(|| format!("Project '{}' not found.", args.0.project))?;
            Ok(json!({
                "project": project_summary(&config, &args.0.project, project),
                "config": project,
                "services": project.services.iter().map(|service| {
                    json!({
                        "name": service.name,
                        "url": open_url_for(&config, &args.0.project, OpenTarget::Service(service.name.clone())).ok(),
                        "ports": service_ports(service),
                    })
                }).collect::<Vec<_>>()
            }))
        }))
    }

    #[tool(description = "Return runtime status for all services, or one project when provided.")]
    fn loopbox_runtime(&self, args: Parameters<RuntimeArgs>) -> CallToolResult {
        tool_value(load_config().map(|config| runtime_value(&config, args.0.project.as_deref())))
    }

    #[tool(description = "Read a clamped tail of Loopbox service logs.")]
    fn loopbox_logs(&self, args: Parameters<LogsArgs>) -> CallToolResult {
        let args = args.0;
        let limit = args.limit.unwrap_or(120).clamp(1, 500);
        tool_value(
            service_logs_tail(&args.project, &args.service, limit).map(|lines| {
                json!({
                    "project": args.project,
                    "service": args.service,
                    "limit": limit,
                    "lines": lines,
                })
            }),
        )
    }

    #[tool(description = "Read recent Loopbox proxy traffic/request records for a project.")]
    fn loopbox_requests(&self, args: Parameters<RequestsArgs>) -> CallToolResult {
        let args = args.0;
        let limit = args.limit.unwrap_or(50).clamp(1, 200);
        tool_value(
            proxy_traffic_events_for_project_with_persisted(
                &args.project,
                args.service.as_deref(),
                limit,
            )
            .map(|events| {
                json!({
                    "project": args.project,
                    "service": args.service,
                    "limit": limit,
                    "events": events,
                })
            }),
        )
    }

    #[tool(description = "Read Loopbox resource metrics for a project or service.")]
    fn loopbox_resources(&self, args: Parameters<ResourcesArgs>) -> CallToolResult {
        let args = args.0;
        let limit = args.limit.unwrap_or(100).clamp(1, 200);
        let window = args.window.unwrap_or_else(|| "1h".to_string());
        tool_value(
            resource_metrics_series_for_project(
                &args.project,
                args.service.as_deref(),
                &window,
                limit,
            )
            .map(|samples| {
                json!({
                    "project": args.project,
                    "service": args.service,
                    "window": window,
                    "limit": limit,
                    "samples": samples,
                })
            }),
        )
    }

    #[tool(description = "Read the Loopbox incident timeline for a project or service.")]
    fn loopbox_incidents(&self, args: Parameters<IncidentsArgs>) -> CallToolResult {
        let args = args.0;
        let limit = args.limit.unwrap_or(100).clamp(1, 500);
        let window = args.window.unwrap_or_else(|| "1h".to_string());
        tool_value(load_config().and_then(|config| {
            incident_timeline_for_project(
                &config,
                &args.project,
                args.service.as_deref(),
                &window,
                limit,
            )
            .map(|events| {
                json!({
                    "project": args.project,
                    "service": args.service,
                    "window": window,
                    "limit": limit,
                    "events": events,
                })
            })
        }))
    }

    #[tool(description = "Validate a proposed Loopbox sandbox/project config without saving it.")]
    fn loopbox_validate_project_config(
        &self,
        args: Parameters<ValidateProjectArgs>,
    ) -> CallToolResult {
        tool_value(load_config().and_then(|config| {
            let input = add_project_input(args.0.project)?;
            let (name, project) = preview_add_project(&config, &input)?;
            Ok(json!({ "name": name, "project": project }))
        }))
    }

    #[tool(
        description = "Create a Loopbox sandbox/project after user approval through MCP elicitation."
    )]
    async fn loopbox_create_project(
        &self,
        args: Parameters<CreateProjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let input = match add_project_input(args.project) {
            Ok(input) => input,
            Err(err) => return tool_error(err),
        };
        let message = format!(
            "Create Loopbox sandbox '{}' with {} service(s)?",
            input.name,
            input.services.len()
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(mutate_config(|config| {
            let name = add_project(config, &input)?;
            Ok(json!({ "created": name }))
        }))
    }

    #[tool(
        description = "Update a Loopbox sandbox/project after user approval through MCP elicitation."
    )]
    async fn loopbox_update_project(
        &self,
        args: Parameters<UpdateProjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let update = match update_project_input(args.config) {
            Ok(input) => input,
            Err(err) => return tool_error(err),
        };
        let message = format!("Update Loopbox sandbox '{}'?", args.project);
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(mutate_config(|config| {
            update_project(config, &args.project, &update)?;
            Ok(json!({ "updated": args.project }))
        }))
    }

    #[tool(description = "Start every service in a Loopbox project after user approval.")]
    async fn loopbox_start_project(
        &self,
        args: Parameters<ProjectMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!("Start all services in Loopbox sandbox '{}'?", args.project);
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(load_config().and_then(|config| {
            start_project_all(&config, &args.project).map(|snapshots| {
                json!({
                    "project": args.project,
                    "snapshots": snapshots.into_iter().map(snapshot_value).collect::<Vec<_>>()
                })
            })
        }))
    }

    #[tool(description = "Stop every service in a Loopbox project after user approval.")]
    async fn loopbox_stop_project(
        &self,
        args: Parameters<ProjectMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!("Stop all services in Loopbox sandbox '{}'?", args.project);
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(load_config().and_then(|config| {
            stop_project_all(&config, &args.project).map(|snapshots| {
                json!({
                    "project": args.project,
                    "snapshots": snapshots.into_iter().map(snapshot_value).collect::<Vec<_>>()
                })
            })
        }))
    }

    #[tool(description = "Restart every service in a Loopbox project after user approval.")]
    async fn loopbox_restart_project(
        &self,
        args: Parameters<ProjectMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!(
            "Restart all services in Loopbox sandbox '{}'?",
            args.project
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(load_config().and_then(|config| {
            stop_project_all(&config, &args.project)?;
            start_project_all(&config, &args.project).map(|snapshots| {
                json!({
                    "project": args.project,
                    "snapshots": snapshots.into_iter().map(snapshot_value).collect::<Vec<_>>()
                })
            })
        }))
    }

    #[tool(description = "Start one Loopbox service after user approval.")]
    async fn loopbox_start_service(
        &self,
        args: Parameters<ServiceMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!(
            "Start Loopbox service '{} / {}'?",
            args.project, args.service
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(load_config().and_then(|config| {
            start_service(&config, &args.project, &args.service)
                .map(|snapshot| json!({ "snapshot": snapshot_value(snapshot) }))
        }))
    }

    #[tool(description = "Stop one Loopbox service after user approval.")]
    async fn loopbox_stop_service(
        &self,
        args: Parameters<ServiceMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!(
            "Stop Loopbox service '{} / {}'?",
            args.project, args.service
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(
            stop_service(&args.project, &args.service)
                .map(|snapshot| json!({ "snapshot": snapshot_value(snapshot) })),
        )
    }

    #[tool(description = "Restart one Loopbox service after user approval.")]
    async fn loopbox_restart_service(
        &self,
        args: Parameters<ServiceMutationArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!(
            "Restart Loopbox service '{} / {}'?",
            args.project, args.service
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(load_config().and_then(|config| {
            restart_service(&config, &args.project, &args.service)
                .map(|snapshot| json!({ "snapshot": snapshot_value(snapshot) }))
        }))
    }

    #[tool(
        description = "Send stdin/terminal input to a running Loopbox service after user approval."
    )]
    async fn loopbox_send_service_input(
        &self,
        args: Parameters<SendInputArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args = args.0;
        let message = format!(
            "Send {} byte(s) of input to Loopbox service '{} / {}'?",
            args.input.len(),
            args.project,
            args.service
        );
        if let Err(err) = require_mutation_approval(&context, args.approved, &message).await {
            return tool_error(err);
        }
        tool_value(
            send_service_input(&args.project, &args.service, &args.input)
                .map(|_| json!({ "sent": true })),
        )
    }
}

pub fn run_loopbox_mcp_subcommand_from_args(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some("__loopbox_mcp_server") {
        return None;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("loopbox-mcp")
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Failed to start Loopbox MCP runtime: {err}");
            return Some(1);
        }
    };

    Some(match runtime.block_on(run_loopbox_mcp_server()) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Loopbox MCP server failed: {err}");
            1
        }
    })
}

async fn run_loopbox_mcp_server() -> Result<(), String> {
    let service = LoopboxMcpServer
        .serve(stdio())
        .await
        .map_err(|err| format!("Failed to serve Loopbox MCP stdio: {err}"))?;
    service
        .waiting()
        .await
        .map_err(|err| format!("Loopbox MCP stdio closed with error: {err}"))?;
    Ok(())
}

async fn require_mutation_approval(
    context: &RequestContext<RoleServer>,
    approved: Option<bool>,
    message: &str,
) -> Result<(), String> {
    if approved == Some(true) {
        return Ok(());
    }
    if approved == Some(false) {
        return Err("Mutation declined by explicit approval flag.".to_string());
    }
    if context.peer.supported_elicitation_modes().is_empty() {
        return Err(
            "Client does not advertise MCP elicitation support; mutation was not run.".to_string(),
        );
    }

    match context.peer.elicit::<MutationConfirmation>(message).await {
        Ok(Some(MutationConfirmation { confirmed: true })) => Ok(()),
        Ok(Some(_)) | Ok(None) => Err("Mutation was not approved.".to_string()),
        Err(ElicitationError::UserDeclined) => Err("Mutation declined by user.".to_string()),
        Err(ElicitationError::UserCancelled) => Err("Mutation cancelled by user.".to_string()),
        Err(err) => Err(format!("MCP elicitation failed: {err}")),
    }
}

fn loopbox_overview_value() -> Result<Value, String> {
    let config = load_config()?;
    let doctor = doctor_report(&config);
    let runtime = runtime_value(&config, None);
    Ok(json!({
        "projects": config.projects.iter().map(|(name, project)| {
            project_summary(&config, name, project)
        }).collect::<Vec<_>>(),
        "doctor": doctor_issues_value(doctor),
        "runtime": runtime,
    }))
}

fn loopbox_doctor_value() -> Result<Value, String> {
    let config = load_config()?;
    Ok(json!({ "issues": doctor_issues_value(doctor_report(&config)) }))
}

fn runtime_value(config: &LoopboxConfig, project_filter: Option<&str>) -> Value {
    let mut projects = BTreeMap::<String, Vec<Value>>::new();
    for (project_name, project) in &config.projects {
        if project_filter.is_some_and(|filter| filter != project_name) {
            continue;
        }
        let services = project
            .services
            .iter()
            .map(|service| {
                let status =
                    service_runtime_status(config, project_name, &service.name).ok().map(snapshot_value);
                json!({
                    "service": service.name,
                    "url": open_url_for(config, project_name, OpenTarget::Service(service.name.clone())).ok(),
                    "ports": service_ports(service),
                    "status": status,
                })
            })
            .collect::<Vec<_>>();
        projects.insert(project_name.clone(), services);
    }
    json!({ "projects": projects })
}

fn project_summary(config: &LoopboxConfig, name: &str, project: &super::ProjectConfig) -> Value {
    let running = project
        .services
        .iter()
        .filter(|service| {
            service_runtime_status(config, name, &service.name)
                .map(|snapshot| {
                    matches!(
                        snapshot.state,
                        super::ServiceRuntimeState::Running | super::ServiceRuntimeState::Starting
                    )
                })
                .unwrap_or(false)
        })
        .count();

    json!({
        "name": name,
        "dir": project.dir,
        "ip": project.ip,
        "primaryHost": project_primary_host(config, name),
        "serviceCount": project.services.len(),
        "runningCount": running,
        "services": project.services.iter().map(|service| {
            json!({
                "name": service.name,
                "runtime": service.runtime,
                "ports": service_ports(service),
                "url": open_url_for(config, name, OpenTarget::Service(service.name.clone())).ok(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn doctor_issues_value(issues: Vec<super::DoctorIssue>) -> Vec<Value> {
    issues
        .into_iter()
        .map(|issue| {
            json!({
                "level": match issue.level {
                    DoctorLevel::Error => "error",
                    DoctorLevel::Warning => "warning",
                    DoctorLevel::Info => "info",
                },
                "project": issue.project,
                "message": issue.message,
                "fix": issue.fix.map(|fix| fix.label().to_string()),
            })
        })
        .collect()
}

fn snapshot_value(snapshot: super::ServiceRuntimeSnapshot) -> Value {
    json!({
        "project": snapshot.project,
        "service": snapshot.service,
        "state": snapshot.state,
        "pid": snapshot.pid,
        "startedAt": snapshot.started_at,
        "exitCode": snapshot.exit_code,
        "lastError": snapshot.last_error,
    })
}

fn add_project_input(input: McpProjectInput) -> Result<AddProjectInput, String> {
    Ok(AddProjectInput {
        name: input.name,
        dir: input.dir,
        ip: input.ip.unwrap_or_default(),
        health_check_interval_secs: input
            .health_check_interval_secs
            .map(|value| value.to_string())
            .unwrap_or_default(),
        services: input
            .services
            .into_iter()
            .map(service_entry)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn update_project_input(input: McpProjectUpdateInput) -> Result<UpdateProjectInput, String> {
    Ok(UpdateProjectInput {
        dir: input.dir,
        ip: input.ip.unwrap_or_default(),
        health_check_interval_secs: input
            .health_check_interval_secs
            .map(|value| value.to_string())
            .unwrap_or_default(),
        services: input
            .services
            .into_iter()
            .map(service_entry)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn service_entry(input: McpServiceInput) -> Result<ServiceEntry, String> {
    if input.name.trim().is_empty() {
        return Err("Service name cannot be empty.".to_string());
    }
    if input.command.trim().is_empty() {
        return Err(format!("Service '{}' command cannot be empty.", input.name));
    }
    let ports = input
        .ports
        .unwrap_or_default()
        .into_iter()
        .map(|port| ServicePortEntry {
            port: port.port.to_string(),
            protocol: port.protocol.unwrap_or_else(|| "http1".to_string()),
            health_path: port.health_path.unwrap_or_default(),
            health_check_interval_secs: port
                .health_check_interval_secs
                .map(|value| value.to_string())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let primary = ports.first().cloned().unwrap_or(ServicePortEntry {
        port: String::new(),
        protocol: "http1".to_string(),
        health_path: String::new(),
        health_check_interval_secs: String::new(),
    });
    Ok(ServiceEntry {
        name: input.name,
        port: primary.port.clone(),
        protocol: primary.protocol.clone(),
        health_path: primary.health_path.clone(),
        ports,
        runtime: input.runtime.unwrap_or_else(|| "process".to_string()),
        command: input.command,
        workdir: input.workdir.unwrap_or_default(),
        env_files: input.env_files.unwrap_or_default().join("\n"),
        depends_on: input.depends_on.unwrap_or_default().join(", "),
        autostart: input.autostart.unwrap_or(false),
        container_image: String::new(),
        container_args: String::new(),
        container_env: String::new(),
        container_volumes: String::new(),
        container_auto_remove: false,
    })
}

fn mutate_config<F>(mutation: F) -> Result<Value, String>
where
    F: FnOnce(&mut LoopboxConfig) -> Result<Value, String>,
{
    let mut config = load_config()?;
    let result = mutation(&mut config)?;
    save_config(&config)?;
    Ok(result)
}

fn tool_value(result: Result<Value, String>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(value),
        Err(err) => tool_error(err),
    }
}

fn tool_error(err: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": err.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_mcp_project_input_to_add_project_input() {
        let input = McpProjectInput {
            name: "demo".to_string(),
            dir: "/tmp/demo".to_string(),
            ip: None,
            health_check_interval_secs: None,
            services: vec![McpServiceInput {
                name: "web".to_string(),
                command: "npm run dev".to_string(),
                workdir: None,
                ports: Some(vec![McpPortInput {
                    port: 5173,
                    protocol: None,
                    health_path: Some("/health".to_string()),
                    health_check_interval_secs: None,
                }]),
                runtime: None,
                env_files: Some(vec![".env".to_string()]),
                depends_on: Some(vec!["api".to_string()]),
                autostart: Some(true),
            }],
        };

        let converted = add_project_input(input).unwrap();
        assert_eq!(converted.name, "demo");
        assert_eq!(converted.services[0].ports[0].port, "5173");
        assert_eq!(converted.services[0].ports[0].protocol, "http1");
        assert_eq!(converted.services[0].env_files, ".env");
        assert_eq!(converted.services[0].depends_on, "api");
        assert!(converted.services[0].autostart);
    }

    #[test]
    fn explicit_decline_blocks_mutation_before_elicitation() {
        let err = explicit_approval_gate(Some(false)).unwrap_err();
        assert!(err.contains("declined"));
    }

    #[test]
    fn tool_argument_schemas_are_json_objects() {
        let empty_schema = schemars::schema_for!(EmptyArgs);
        assert_eq!(empty_schema.as_value().get("type"), Some(&json!("object")));
        assert_eq!(
            empty_schema.as_value().get("additionalProperties"),
            Some(&json!(false))
        );

        let runtime_schema = schemars::schema_for!(RuntimeArgs);
        assert_eq!(
            runtime_schema.as_value().get("type"),
            Some(&json!("object"))
        );
    }

    #[test]
    fn advertised_tool_schemas_are_json_objects() {
        for tool in LoopboxMcpServer::tool_router().list_all() {
            assert_eq!(
                tool.input_schema.get("type"),
                Some(&json!("object")),
                "{} advertised non-object input schema: {:?}",
                tool.name,
                tool.input_schema
            );
        }
    }

    #[test]
    fn advertised_tools_include_project_mutations() {
        let tool_names = LoopboxMcpServer::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();

        for expected in [
            "loopbox_validate_project_config",
            "loopbox_create_project",
            "loopbox_update_project",
            "loopbox_incidents",
        ] {
            assert!(
                tool_names.iter().any(|name| name == expected),
                "missing advertised MCP tool: {expected}"
            );
        }
    }

    fn explicit_approval_gate(approved: Option<bool>) -> Result<(), String> {
        match approved {
            Some(true) => Ok(()),
            Some(false) => Err("Mutation declined by explicit approval flag.".to_string()),
            None => Err("MCP elicitation required.".to_string()),
        }
    }
}
