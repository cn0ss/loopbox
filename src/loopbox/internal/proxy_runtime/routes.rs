use super::*;

pub(super) fn build_proxy_routes(config: &LoopboxConfig) -> HashMap<String, ProxyRoute> {
    let mut routes = HashMap::new();
    let suffix = config.global.domain_suffix.trim().trim_start_matches('.');
    let traffic_capture_supported = supports_traffic_capture();
    let redacted_header_names =
        sanitize_redaction_list(&config.global.proxy_traffic.redact_headers);
    let redacted_query_keys =
        sanitize_redaction_list(&config.global.proxy_traffic.redact_query_keys);

    for (project_name, project) in &config.projects {
        let project_clean = project_name.trim().to_lowercase();
        let capture_enabled = traffic_capture_supported
            && project
                .proxy_traffic_capture_enabled
                .unwrap_or(config.global.proxy_traffic.capture_enabled_by_default);
        let capture_mode = project
            .proxy_traffic_capture_mode
            .clone()
            .unwrap_or_else(|| config.global.proxy_traffic.capture_mode_default.clone());
        let capture_mode = enforce_traffic_capture_mode(capture_mode);
        let request_body_preview_max_bytes = sanitize_proxy_body_preview_limit(
            config.global.proxy_traffic.request_body_preview_max_bytes,
            DEFAULT_PROXY_REQUEST_BODY_PREVIEW_MAX_BYTES,
        );
        let response_body_preview_max_bytes = sanitize_proxy_body_preview_limit(
            config.global.proxy_traffic.response_body_preview_max_bytes,
            DEFAULT_PROXY_RESPONSE_BODY_PREVIEW_MAX_BYTES,
        );
        let capture_text_only = config.global.proxy_traffic.capture_text_only;
        for service in &project.services {
            let http_port = service_ports(service)
                .into_iter()
                .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
                .map(|entry| entry.port);
            let Some(port) = http_port else {
                continue;
            };
            let service_clean = service.name.trim().to_lowercase();
            let host = format!("{service_clean}.{project_clean}.{suffix}").to_lowercase();
            routes.insert(
                host,
                ProxyRoute {
                    project_name: project_name.to_string(),
                    service_name: service.name.clone(),
                    target_ip: project.ip.trim().to_string(),
                    target_port: port,
                    capture_enabled,
                    capture_mode: capture_mode.clone(),
                    capture_text_only,
                    redacted_header_names: redacted_header_names.clone(),
                    redacted_query_keys: redacted_query_keys.clone(),
                    request_body_preview_max_bytes,
                    response_body_preview_max_bytes,
                },
            );
        }
    }

    routes
}

pub(super) fn build_proxy_endpoint_routes(
    config: &LoopboxConfig,
) -> HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>> {
    let mut routes = HashMap::new();
    let traffic_capture_supported = supports_traffic_capture();
    let suffix = config
        .global
        .domain_suffix
        .trim()
        .trim_start_matches('.')
        .to_lowercase();
    let default_capture_enabled =
        traffic_capture_supported && config.global.proxy_traffic.capture_enabled_by_default;
    let default_capture_mode =
        enforce_traffic_capture_mode(config.global.proxy_traffic.capture_mode_default.clone());
    let capture_text_only = config.global.proxy_traffic.capture_text_only;
    let redacted_header_names =
        sanitize_redaction_list(&config.global.proxy_traffic.redact_headers);
    let redacted_query_keys =
        sanitize_redaction_list(&config.global.proxy_traffic.redact_query_keys);
    let request_body_preview_max_bytes = sanitize_proxy_body_preview_limit(
        config.global.proxy_traffic.request_body_preview_max_bytes,
        DEFAULT_PROXY_REQUEST_BODY_PREVIEW_MAX_BYTES,
    );
    let response_body_preview_max_bytes = sanitize_proxy_body_preview_limit(
        config.global.proxy_traffic.response_body_preview_max_bytes,
        DEFAULT_PROXY_RESPONSE_BODY_PREVIEW_MAX_BYTES,
    );

    for (project_name, project) in &config.projects {
        let capture_enabled = traffic_capture_supported
            && project
                .proxy_traffic_capture_enabled
                .unwrap_or(default_capture_enabled);
        let capture_mode = project
            .proxy_traffic_capture_mode
            .clone()
            .unwrap_or_else(|| default_capture_mode.clone());
        let capture_mode = enforce_traffic_capture_mode(capture_mode);

        for endpoint in &project.proxy_endpoints {
            if endpoint.listen_port == 0 || endpoint.upstream_port == 0 {
                continue;
            }
            let listen_host = endpoint.listen_host.trim();
            let upstream_host = endpoint.upstream_host.trim();
            if upstream_host.is_empty() {
                continue;
            }

            let key = ProxyEndpointKey {
                listen_host: if listen_host.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    listen_host.to_string()
                },
                listen_port: endpoint.listen_port,
            };
            let service_name = endpoint
                .service_name
                .as_ref()
                .map(|value| value.trim().to_lowercase())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    find_service_for_endpoint_in_project(
                        project,
                        upstream_host,
                        endpoint.upstream_port,
                    )
                })
                .unwrap_or_else(|| endpoint.name.trim().to_string());
            let grpc_proto_paths = project.grpc_proto_paths.clone();

            insert_proxy_endpoint_route(
                &mut routes,
                key,
                ProxyEndpointRoute {
                    name: endpoint.name.trim().to_string(),
                    protocol: endpoint.protocol.clone(),
                    upstream_host: upstream_host.to_string(),
                    upstream_port: endpoint.upstream_port,
                    authority: endpoint
                        .authority
                        .as_ref()
                        .map(|value| value.trim().to_lowercase()),
                    project_name: project_name.to_string(),
                    service_name,
                    grpc_proto_paths,
                    capture_enabled,
                    capture_mode: capture_mode.clone(),
                    capture_text_only,
                    redacted_header_names: redacted_header_names.clone(),
                    redacted_query_keys: redacted_query_keys.clone(),
                    request_body_preview_max_bytes,
                    response_body_preview_max_bytes,
                },
            );
        }
    }

    // Backward compatibility: keep honoring legacy global endpoint routes.
    for endpoint in &config.global.proxy_endpoints {
        if endpoint.listen_port == 0 || endpoint.upstream_port == 0 {
            continue;
        }
        let listen_host = endpoint.listen_host.trim();
        let upstream_host = endpoint.upstream_host.trim();
        if upstream_host.is_empty() {
            continue;
        }
        let key = ProxyEndpointKey {
            listen_host: if listen_host.is_empty() {
                "127.0.0.1".to_string()
            } else {
                listen_host.to_string()
            },
            listen_port: endpoint.listen_port,
        };

        let mut project_name = endpoint
            .project_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let mut service_name = endpoint
            .service_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        if project_name.is_none() || service_name.is_none() {
            if let Some((matched_project, matched_service)) =
                find_project_service_for_endpoint(config, upstream_host, endpoint.upstream_port)
            {
                project_name = Some(matched_project);
                service_name = Some(matched_service);
            }
        }

        let effective_project_name = project_name.unwrap_or_else(|| "legacy-global".to_string());
        let effective_service_name = service_name
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| endpoint.name.trim().to_string());
        let (capture_enabled, capture_mode) =
            if let Some(project) = config.projects.get(&effective_project_name) {
                (
                    traffic_capture_supported
                        && project
                            .proxy_traffic_capture_enabled
                            .unwrap_or(default_capture_enabled),
                    enforce_traffic_capture_mode(
                        project
                            .proxy_traffic_capture_mode
                            .clone()
                            .unwrap_or_else(|| default_capture_mode.clone()),
                    ),
                )
            } else {
                (default_capture_enabled, default_capture_mode.clone())
            };
        let grpc_proto_paths = config
            .projects
            .get(&effective_project_name)
            .map(|project| project.grpc_proto_paths.clone())
            .unwrap_or_default();

        insert_proxy_endpoint_route(
            &mut routes,
            key,
            ProxyEndpointRoute {
                name: endpoint.name.trim().to_string(),
                protocol: endpoint.protocol.clone(),
                upstream_host: upstream_host.to_string(),
                upstream_port: endpoint.upstream_port,
                authority: endpoint
                    .authority
                    .as_ref()
                    .map(|value| value.trim().to_lowercase()),
                project_name: effective_project_name,
                service_name: effective_service_name,
                grpc_proto_paths,
                capture_enabled,
                capture_mode,
                capture_text_only,
                redacted_header_names: redacted_header_names.clone(),
                redacted_query_keys: redacted_query_keys.clone(),
                request_body_preview_max_bytes,
                response_body_preview_max_bytes,
            },
        );
    }

    // Auto routes: derive gRPC listeners from sandbox services.
    // This avoids manual endpoint wiring for common service-to-service traffic.
    for (project_name, project) in &config.projects {
        let capture_enabled = traffic_capture_supported
            && project
                .proxy_traffic_capture_enabled
                .unwrap_or(default_capture_enabled);
        let capture_mode = project
            .proxy_traffic_capture_mode
            .clone()
            .unwrap_or_else(|| default_capture_mode.clone());
        let capture_mode = enforce_traffic_capture_mode(capture_mode);
        let grpc_proto_paths = project.grpc_proto_paths.clone();

        for service in &project.services {
            for port_entry in service_ports(service) {
                match port_entry.protocol {
                    ProxyEndpointProtocol::GrpcH2c => {
                        let key = ProxyEndpointKey {
                            listen_host: "127.0.0.1".to_string(),
                            listen_port: port_entry.port,
                        };
                        let authority = if suffix.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "{}.{}.{}",
                                service.name.trim().to_lowercase(),
                                project_name.trim().to_lowercase(),
                                suffix
                            ))
                        };
                        let route_name = format!(
                            "auto-{}-{}-grpc-{}",
                            project_name.trim().to_lowercase(),
                            service.name.trim().to_lowercase(),
                            port_entry.port
                        );
                        insert_proxy_endpoint_route(
                            &mut routes,
                            key,
                            ProxyEndpointRoute {
                                name: route_name,
                                protocol: ProxyEndpointProtocol::GrpcH2c,
                                upstream_host: project.ip.trim().to_string(),
                                upstream_port: port_entry.port,
                                authority,
                                project_name: project_name.to_string(),
                                service_name: service.name.clone(),
                                grpc_proto_paths: grpc_proto_paths.clone(),
                                capture_enabled,
                                capture_mode: capture_mode.clone(),
                                capture_text_only,
                                redacted_header_names: redacted_header_names.clone(),
                                redacted_query_keys: redacted_query_keys.clone(),
                                request_body_preview_max_bytes,
                                response_body_preview_max_bytes,
                            },
                        );
                    }
                    ProxyEndpointProtocol::TcpPassthrough => {
                        let key = ProxyEndpointKey {
                            listen_host: "127.0.0.1".to_string(),
                            listen_port: port_entry.port,
                        };
                        let route_name = format!(
                            "auto-{}-{}-tcp-{}",
                            project_name.trim().to_lowercase(),
                            service.name.trim().to_lowercase(),
                            port_entry.port
                        );
                        insert_proxy_endpoint_route(
                            &mut routes,
                            key,
                            ProxyEndpointRoute {
                                name: route_name,
                                protocol: ProxyEndpointProtocol::TcpPassthrough,
                                upstream_host: project.ip.trim().to_string(),
                                upstream_port: port_entry.port,
                                authority: None,
                                project_name: project_name.to_string(),
                                service_name: service.name.clone(),
                                grpc_proto_paths: grpc_proto_paths.clone(),
                                capture_enabled,
                                capture_mode: capture_mode.clone(),
                                capture_text_only,
                                redacted_header_names: redacted_header_names.clone(),
                                redacted_query_keys: redacted_query_keys.clone(),
                                request_body_preview_max_bytes,
                                response_body_preview_max_bytes,
                            },
                        );
                    }
                    ProxyEndpointProtocol::Http1 => {}
                }
            }
        }
    }
    routes
}

pub(super) fn insert_proxy_endpoint_route(
    routes: &mut HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>,
    key: ProxyEndpointKey,
    route: ProxyEndpointRoute,
) {
    let route_protocol = route.protocol.clone();
    let route_authority = route.authority.clone().unwrap_or_default();
    let route_authority = route_authority.trim().to_lowercase();

    let entry = routes.entry(key).or_insert_with(Vec::new);
    if entry
        .iter()
        .any(|existing| existing.protocol != route_protocol)
    {
        return;
    }

    match route_protocol {
        ProxyEndpointProtocol::GrpcH2c => {
            let duplicate = entry.iter().any(|existing| {
                existing
                    .authority
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    == route_authority
            });
            if !duplicate {
                entry.push(route);
            }
        }
        ProxyEndpointProtocol::Http1 | ProxyEndpointProtocol::TcpPassthrough => {
            if entry.is_empty() {
                entry.push(route);
            }
        }
    }
}

pub(super) fn find_service_for_endpoint_in_project(
    project: &ProjectConfig,
    upstream_host: &str,
    upstream_port: u16,
) -> Option<String> {
    if !project.ip.trim().eq_ignore_ascii_case(upstream_host.trim()) {
        return None;
    }
    project
        .services
        .iter()
        .find(|service| {
            service_ports(service)
                .iter()
                .any(|entry| entry.port == upstream_port)
        })
        .map(|service| service.name.clone())
}

pub(super) fn find_project_service_for_endpoint(
    config: &LoopboxConfig,
    upstream_host: &str,
    upstream_port: u16,
) -> Option<(String, String)> {
    for (project_name, project) in &config.projects {
        if !project.ip.trim().eq_ignore_ascii_case(upstream_host.trim()) {
            continue;
        }
        for service in &project.services {
            if service_ports(service)
                .iter()
                .any(|entry| entry.port == upstream_port)
            {
                return Some((project_name.clone(), service.name.clone()));
            }
        }
    }
    None
}

pub(super) fn grpc_request_authority(request: &HttpRequest<h2::RecvStream>) -> Option<String> {
    request
        .uri()
        .authority()
        .map(|value| value.as_str())
        .or_else(|| {
            request
                .headers()
                .get("host")
                .and_then(|value| value.to_str().ok())
        })
        .and_then(normalize_authority)
}

pub(super) fn select_grpc_route_for_authority<'a>(
    routes: &'a [ProxyEndpointRoute],
    authority: Option<&str>,
) -> Option<&'a ProxyEndpointRoute> {
    if routes.is_empty() {
        return None;
    }
    if routes.len() == 1 {
        return routes.first();
    }

    if let Some(authority) = authority {
        for route in routes {
            if let Some(pattern) = route.authority.as_deref() {
                if authority_matches(pattern, authority) {
                    return Some(route);
                }
            }
        }
    }

    routes.iter().find(|route| route.authority.is_none())
}

pub(super) fn normalize_authority(value: &str) -> Option<String> {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split("://")
        .last()
        .unwrap_or(trimmed.as_str())
        .trim();
    let authority_only = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .trim();
    if authority_only.is_empty() {
        return None;
    }
    Some(authority_only.to_string())
}

pub(super) fn authority_matches(pattern: &str, incoming: &str) -> bool {
    let normalized_pattern = match normalize_authority(pattern) {
        Some(value) => value,
        None => return false,
    };
    let normalized_incoming = match normalize_authority(incoming) {
        Some(value) => value,
        None => return false,
    };

    if normalized_pattern == normalized_incoming {
        return true;
    }
    if authority_has_explicit_port(&normalized_pattern) {
        return false;
    }
    strip_host_port(&normalized_pattern)
        .eq_ignore_ascii_case(&strip_host_port(&normalized_incoming))
}

pub(super) fn authority_has_explicit_port(authority: &str) -> bool {
    if authority.starts_with('[') {
        return authority.contains("]:");
    }
    authority.matches(':').count() == 1
}
