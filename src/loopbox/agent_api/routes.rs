use super::*;

pub(super) fn build_router(state: AgentApiState) -> Router {
    let protected = Router::new()
        .route(&format!("/{AGENT_API_VERSION}/meta"), get(meta_handler))
        .route(
            &format!("/{AGENT_API_VERSION}/projects"),
            get(list_projects_handler).post(project_create_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}"),
            get(project_detail_handler).put(project_update_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/runtime"),
            get(project_runtime_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/logs"),
            get(project_logs_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/requests"),
            get(project_requests_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/start"),
            post(project_start_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/stop"),
            post(project_stop_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/restart"),
            post(project_restart_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/start"),
            post(service_start_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/stop"),
            post(service_stop_handler),
        )
        .route(
            &format!("/{AGENT_API_VERSION}/projects/{{project}}/services/{{service}}/restart"),
            post(service_restart_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .route(&format!("/{AGENT_API_VERSION}/health"), get(health_handler))
        .route(
            &format!("/{AGENT_API_VERSION}/openapi.json"),
            get(openapi_handler),
        )
        .merge(protected)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_middleware,
        ))
        .with_state(state)
}

async fn auth_middleware(
    State(state): State<AgentApiState>,
    request: Request,
    next: Next,
) -> Response {
    if !state.auth_enabled {
        return next.run(request).await;
    }

    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let Some(expected) = state.expected_bearer.as_ref() else {
        return ApiError::unauthorized("Missing or invalid bearer token.").into_response();
    };

    if header != expected.as_str() {
        return ApiError::unauthorized("Missing or invalid bearer token.").into_response();
    }

    next.run(request).await
}

async fn audit_middleware(
    State(state): State<AgentApiState>,
    request: Request,
    next: Next,
) -> Response {
    run_agent_api_audit_middleware(state.auth_enabled, request, next).await
}

async fn openapi_handler(
    State(state): State<AgentApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(openapi_spec_json(state.bind_port, state.auth_enabled)))
}

async fn health_handler(
    State(state): State<AgentApiState>,
) -> Result<Json<HealthResponse>, ApiError> {
    let proxy = reverse_proxy_status();
    Ok(Json(HealthResponse {
        ok: true,
        api_version: AGENT_API_VERSION,
        app_version: app_version_label(),
        reverse_proxy: ReverseProxyInfo {
            running: proxy.running,
            bind_port: proxy.bind_port,
            using_fallback_port: proxy.using_fallback_port,
            note: proxy.note,
        },
        agent_api: AgentApiHealthInfo {
            auth_enabled: state.auth_enabled,
            bind_port: state.bind_port,
        },
    }))
}

async fn meta_handler(State(state): State<AgentApiState>) -> Result<Json<MetaResponse>, ApiError> {
    Ok(Json(MetaResponse {
        api_version: AGENT_API_VERSION,
        log_limit_default: DEFAULT_LOG_LIMIT,
        log_limit_max: MAX_LOG_LIMIT,
        request_limit_default: DEFAULT_REQUEST_LIMIT,
        request_limit_max: MAX_REQUEST_LIMIT,
        auth_enabled: state.auth_enabled,
        openapi_url: format!(
            "http://127.0.0.1:{}/{AGENT_API_VERSION}/openapi.json",
            state.bind_port
        ),
    }))
}

async fn list_projects_handler() -> Result<Json<ProjectsResponse>, ApiError> {
    let config = load_config_api()?;
    let mut projects = Vec::new();
    for (project_name, project_cfg) in &config.projects {
        let status = project_runtime_counts(&config, project_name, project_cfg);
        projects.push(ProjectSummary {
            name: project_name.clone(),
            dir: project_cfg.dir.clone(),
            ip: project_cfg.ip.clone(),
            primary_host: project_primary_host(&config, project_name),
            service_count: project_cfg.services.len(),
            status,
        });
    }
    Ok(Json(ProjectsResponse { projects }))
}

async fn project_detail_handler(
    Path(project_name): Path<String>,
) -> Result<Json<ProjectDetailResponse>, ApiError> {
    let config = load_config_api()?;
    Ok(Json(build_project_detail_response(&config, &project_name)?))
}

async fn project_create_handler(
    State(state): State<AgentApiState>,
    Query(query): Query<ProjectMutationQuery>,
    Json(request): Json<ProjectCreateRequest>,
) -> Result<Json<ProjectConfigMutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let mut config = load_config_api()?;
    let add_input = project_create_request_to_input(request);
    let project_name =
        add_project(&mut config, &add_input).map_err(map_project_config_mutation_error)?;
    let persist = persist_project_config_mutation(&config, query.apply_system_setup)?;
    let detail = build_project_detail_response(&config, &project_name)?;
    Ok(Json(ProjectConfigMutationResponse {
        project: project_name,
        action: "create",
        saved_config_path: persist.saved_config_path,
        reverse_proxy_synced: true,
        system_setup_applied: query.apply_system_setup,
        system_setup_message: persist.system_setup_message,
        detail,
    }))
}

async fn project_update_handler(
    State(state): State<AgentApiState>,
    Path(project_name): Path<String>,
    Query(query): Query<ProjectMutationQuery>,
    Json(request): Json<ProjectUpdateRequest>,
) -> Result<Json<ProjectConfigMutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let mut config = load_config_api()?;
    let update_input = project_update_request_to_input(request);
    update_project(&mut config, &project_name, &update_input)
        .map_err(map_project_config_mutation_error)?;
    let persist = persist_project_config_mutation(&config, query.apply_system_setup)?;
    let detail = build_project_detail_response(&config, &project_name)?;
    Ok(Json(ProjectConfigMutationResponse {
        project: project_name,
        action: "update",
        saved_config_path: persist.saved_config_path,
        reverse_proxy_synced: true,
        system_setup_applied: query.apply_system_setup,
        system_setup_message: persist.system_setup_message,
        detail,
    }))
}

async fn project_runtime_handler(
    Path(project_name): Path<String>,
) -> Result<Json<ProjectRuntimeResponse>, ApiError> {
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let services = runtime_snapshot_dtos(&config, &project_name, project.services.as_slice())?;
    Ok(Json(ProjectRuntimeResponse {
        project: project_name,
        services,
    }))
}

async fn project_logs_handler(
    Path(project_name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<LogsResponse>, ApiError> {
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let service_name = query
        .service
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("Missing required query parameter: service"))?;
    get_service(project, &service_name)?;

    let limit = clamp_limit(query.limit, DEFAULT_LOG_LIMIT, MAX_LOG_LIMIT);
    let lines = service_logs_tail(&project_name, &service_name, limit)
        .map_err(|err| ApiError::internal(format!("Failed to read logs: {err}")))?;
    let log_attached = service_log_attached(&project_name, &service_name)
        .map_err(|err| ApiError::internal(format!("Failed to inspect log attachment: {err}")))?;
    Ok(Json(LogsResponse {
        project: project_name,
        service: service_name,
        limit,
        log_attached,
        lines,
    }))
}

async fn project_requests_handler(
    Path(project_name): Path<String>,
    Query(query): Query<RequestsQuery>,
) -> Result<Json<RequestsResponse>, ApiError> {
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    if let Some(service_name) = query.service.as_ref() {
        get_service(project, service_name.trim())?;
    }

    let limit = clamp_limit(query.limit, DEFAULT_REQUEST_LIMIT, MAX_REQUEST_LIMIT);
    let normalized_service = query
        .service
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let events = proxy_traffic_events_for_project_with_persisted(
        &project_name,
        normalized_service.as_deref(),
        limit,
    )
    .map_err(|err| ApiError::internal(format!("Failed to read request history: {err}")))?;
    Ok(Json(RequestsResponse {
        project: project_name.clone(),
        service: normalized_service,
        limit,
        capture_enabled: project_proxy_traffic_enabled(&config, &project_name),
        capture_mode: capture_mode_label(project_proxy_traffic_capture_mode(
            &config,
            &project_name,
        )),
        events,
    }))
}

async fn project_start_handler(
    State(state): State<AgentApiState>,
    Path(project_name): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let results = start_project_all(&config, &project_name)
        .map_err(|err| ApiError::conflict(format!("Failed to start project: {err}")))?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: None,
        action: "start",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), results)?,
    }))
}

async fn project_stop_handler(
    State(state): State<AgentApiState>,
    Path(project_name): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let results = stop_project_all(&config, &project_name)
        .map_err(|err| ApiError::conflict(format!("Failed to stop project: {err}")))?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: None,
        action: "stop",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), results)?,
    }))
}

async fn project_restart_handler(
    State(state): State<AgentApiState>,
    Path(project_name): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    stop_project_all(&config, &project_name).map_err(|err| {
        ApiError::conflict(format!("Failed to restart project (stop failed): {err}"))
    })?;
    let results = start_project_all(&config, &project_name).map_err(|err| {
        ApiError::conflict(format!("Failed to restart project (start failed): {err}"))
    })?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: None,
        action: "restart",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), results)?,
    }))
}

async fn service_start_handler(
    State(state): State<AgentApiState>,
    Path((project_name, service_name)): Path<(String, String)>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let service = get_service(project, &service_name)?;
    let snapshot = start_service(&config, &project_name, &service.name)
        .map_err(|err| ApiError::conflict(format!("Failed to start service: {err}")))?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: Some(service.name.clone()),
        action: "start",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), vec![snapshot])?,
    }))
}

async fn service_stop_handler(
    State(state): State<AgentApiState>,
    Path((project_name, service_name)): Path<(String, String)>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let service = get_service(project, &service_name)?;
    let snapshot = stop_service(&project_name, &service.name)
        .map_err(|err| ApiError::conflict(format!("Failed to stop service: {err}")))?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: Some(service.name.clone()),
        action: "stop",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), vec![snapshot])?,
    }))
}

async fn service_restart_handler(
    State(state): State<AgentApiState>,
    Path((project_name, service_name)): Path<(String, String)>,
) -> Result<Json<MutationResponse>, ApiError> {
    let _guard = lock_mutation(&state)?;
    let config = load_config_api()?;
    let project = get_project(&config, &project_name)?;
    let service = get_service(project, &service_name)?;
    let snapshot = restart_service(&config, &project_name, &service.name)
        .map_err(|err| ApiError::conflict(format!("Failed to restart service: {err}")))?;
    Ok(Json(MutationResponse {
        project: project_name.clone(),
        service: Some(service.name.clone()),
        action: "restart",
        results: snapshots_to_dtos(&project_name, project.services.as_slice(), vec![snapshot])?,
    }))
}
