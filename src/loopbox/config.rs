use super::projects::normalize_domain_suffix;
use super::{
    default_agent_api_port, default_domain_suffix, default_health_check_interval_secs,
    default_ip_base, enforce_traffic_capture_mode,
    resource_metrics::sanitize_resource_metrics_settings, service_ports, LoopboxConfig,
    ProxyCaptureMode, ProxyEndpointConfig, ProxyEndpointProtocol, ServicePortConfig,
    ServiceRuntimeKind, WireGuardTunnelConfig,
};
use std::env;
use std::fs;
use std::path::PathBuf;

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

pub fn load_config() -> Result<LoopboxConfig, String> {
    let path = config_path();

    if !path.exists() {
        return Ok(LoopboxConfig::default());
    }

    let contents = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;

    let mut config: LoopboxConfig = toml::from_str(&contents).map_err(|err| {
        format!(
            "Invalid TOML in {}: {err}. This version expects the new service-based loopbox schema.",
            path.display()
        )
    })?;
    normalize_config(&mut config);
    Ok(config)
}

pub fn save_config(config: &LoopboxConfig) -> Result<PathBuf, String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let serialized = toml::to_string_pretty(config)
        .map_err(|err| format!("Failed to serialize config: {err}"))?;
    fs::write(&path, serialized)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    Ok(path)
}

#[allow(dead_code)]
pub fn reset_config_to_default() -> Result<PathBuf, String> {
    save_config(&LoopboxConfig::default())
}

pub fn update_global_settings(
    config: &mut LoopboxConfig,
    suffix: &str,
    range_start: &str,
    range_end: &str,
    health_check_interval_secs: &str,
) -> Result<(), String> {
    let parsed_start = range_start
        .trim()
        .parse::<u8>()
        .map_err(|_| "IP range start must be a number between 2 and 254.".to_string())?;
    let parsed_end = range_end
        .trim()
        .parse::<u8>()
        .map_err(|_| "IP range end must be a number between 2 and 254.".to_string())?;

    if parsed_start < 2 {
        return Err("IP range start must be >= 2.".to_string());
    }
    if parsed_end > 254 {
        return Err("IP range end must be <= 254.".to_string());
    }
    if parsed_start > parsed_end {
        return Err("IP range start must be <= IP range end.".to_string());
    }
    let parsed_health_interval = health_check_interval_secs
        .trim()
        .parse::<u64>()
        .map_err(|_| "Health check interval must be seconds between 2 and 300.".to_string())?;

    let cleaned_suffix = normalize_domain_suffix(suffix);
    config.global.domain_suffix = cleaned_suffix;
    config.global.ip_range_start = parsed_start;
    config.global.ip_range_end = parsed_end;
    config.global.health_check_interval_secs =
        sanitize_health_check_interval_secs(Some(parsed_health_interval))
            .unwrap_or_else(default_health_check_interval_secs);

    Ok(())
}

fn normalize_config(config: &mut LoopboxConfig) {
    if config.global.domain_suffix.trim().is_empty() {
        config.global.domain_suffix = default_domain_suffix();
    } else {
        config.global.domain_suffix = normalize_domain_suffix(&config.global.domain_suffix);
        if config.global.domain_suffix == "test" {
            config.global.domain_suffix = default_domain_suffix();
        }
    }

    if config.global.ip_base.trim().is_empty() {
        config.global.ip_base = default_ip_base();
    }

    if config.global.ip_range_start < 2 {
        config.global.ip_range_start = 2;
    }
    if config.global.ip_range_end > 254 {
        config.global.ip_range_end = 254;
    }
    if config.global.ip_range_start > config.global.ip_range_end {
        config.global.ip_range_start = 2;
        config.global.ip_range_end = 254;
    }

    if config.global.agent_api.port == 0 {
        config.global.agent_api.port = default_agent_api_port();
    }
    config.global.health_check_interval_secs =
        sanitize_health_check_interval_secs(Some(config.global.health_check_interval_secs))
            .unwrap_or_else(default_health_check_interval_secs);
    sanitize_resource_metrics_settings(&mut config.global.resource_metrics);

    // Legacy migration: bool preview flag -> capture mode.
    if config.global.proxy_traffic.capture_body_preview
        && config.global.proxy_traffic.capture_mode_default == ProxyCaptureMode::Metadata
    {
        config.global.proxy_traffic.capture_mode_default = ProxyCaptureMode::BodyPreview;
    }
    config.global.proxy_traffic.capture_mode_default =
        enforce_traffic_capture_mode(config.global.proxy_traffic.capture_mode_default.clone());
    config.global.proxy_traffic.capture_body_preview = matches!(
        config.global.proxy_traffic.capture_mode_default,
        ProxyCaptureMode::BodyPreview
    );

    if config.global.proxy_traffic.max_events == 0 {
        config.global.proxy_traffic.max_events = 2_000;
    } else {
        config.global.proxy_traffic.max_events =
            config.global.proxy_traffic.max_events.clamp(100, 100_000);
    }
    if config.global.proxy_traffic.request_body_preview_max_bytes == 0 {
        config.global.proxy_traffic.request_body_preview_max_bytes = 64 * 1024;
    } else {
        config.global.proxy_traffic.request_body_preview_max_bytes = config
            .global
            .proxy_traffic
            .request_body_preview_max_bytes
            .clamp(256, 1024 * 1024);
    }
    if config.global.proxy_traffic.response_body_preview_max_bytes == 0 {
        config.global.proxy_traffic.response_body_preview_max_bytes = 128 * 1024;
    } else {
        config.global.proxy_traffic.response_body_preview_max_bytes = config
            .global
            .proxy_traffic
            .response_body_preview_max_bytes
            .clamp(256, 1024 * 1024);
    }
    if config.global.proxy_traffic.retention_days == 0 {
        config.global.proxy_traffic.retention_days = 7;
    } else {
        config.global.proxy_traffic.retention_days =
            config.global.proxy_traffic.retention_days.clamp(1, 90);
    }
    if config.global.proxy_traffic.max_storage_mb == 0 {
        config.global.proxy_traffic.max_storage_mb = 500;
    } else {
        config.global.proxy_traffic.max_storage_mb =
            config.global.proxy_traffic.max_storage_mb.clamp(50, 10_000);
    }
    if config.global.proxy_traffic.writer_queue_size == 0 {
        config.global.proxy_traffic.writer_queue_size = 10_000;
    } else {
        config.global.proxy_traffic.writer_queue_size = config
            .global
            .proxy_traffic
            .writer_queue_size
            .clamp(100, 100_000);
    }
    config.global.proxy_traffic.redact_headers = sanitize_redaction_list(
        &config.global.proxy_traffic.redact_headers,
        default_redact_headers(),
    );
    config.global.proxy_traffic.redact_query_keys = sanitize_redaction_list(
        &config.global.proxy_traffic.redact_query_keys,
        default_redact_query_keys(),
    );
    let mut migrated_global_proxy_endpoints = Vec::new();
    let mut project_proxy_endpoint_migrations = Vec::new();
    for endpoint in sanitize_proxy_endpoints(&config.global.proxy_endpoints) {
        if let Some(project_name) = infer_endpoint_project_name(config, &endpoint) {
            project_proxy_endpoint_migrations.push((project_name, endpoint));
        } else {
            migrated_global_proxy_endpoints.push(endpoint);
        }
    }
    config.global.proxy_endpoints = migrated_global_proxy_endpoints;
    sanitize_kubernetes_settings(config);
    for (project_name, endpoint) in project_proxy_endpoint_migrations {
        if let Some(project) = config.projects.get_mut(&project_name) {
            project.proxy_endpoints.push(endpoint);
        }
    }

    for (project_name, project) in config.projects.iter_mut() {
        project.dir = project.dir.trim().to_string();
        project.health_check_interval_secs =
            sanitize_health_check_interval_secs(project.health_check_interval_secs);
        project.proxy_traffic_capture_mode = project
            .proxy_traffic_capture_mode
            .clone()
            .map(enforce_traffic_capture_mode);
        project.grpc_proto_paths = sanitize_grpc_proto_paths(&project.grpc_proto_paths);
        project.proxy_endpoints = sanitize_proxy_endpoints(&project.proxy_endpoints)
            .into_iter()
            .map(|mut endpoint| {
                endpoint.project_name = Some(project_name.to_string());
                endpoint
            })
            .collect();

        if project
            .default_open_service
            .as_ref()
            .is_some_and(|svc| !project.services.iter().any(|service| &service.name == svc))
        {
            project.default_open_service = project
                .services
                .iter()
                .find(|service| !service_ports(service).is_empty())
                .or_else(|| project.services.first())
                .map(|service| service.name.clone());
        }

        for service in &mut project.services {
            service.name = service.name.trim().to_lowercase();
            service.command = service.command.trim().to_string();
            if service.workdir.trim().is_empty() {
                service.workdir = project.dir.clone();
            } else {
                service.workdir = service.workdir.trim().to_string();
            }
            service.env_files = service
                .env_files
                .iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
            let mut seen_dependencies = std::collections::HashSet::new();
            service.depends_on = service
                .depends_on
                .iter()
                .map(|item| item.trim().to_lowercase())
                .filter(|item| !item.is_empty())
                .filter(|item| item != &service.name)
                .filter(|item| seen_dependencies.insert(item.clone()))
                .collect();

            match service.runtime {
                ServiceRuntimeKind::Process => {
                    service.container = None;
                }
                ServiceRuntimeKind::Container => {
                    service.container = sanitize_container_service_config(service.container.take());
                }
            }

            let normalized_ports = sanitize_service_ports(service);
            service.ports = normalized_ports.clone();
            service.port = normalized_ports.first().map(|entry| entry.port);
            service.protocol = normalized_ports
                .first()
                .map(|entry| entry.protocol.clone())
                .unwrap_or(ProxyEndpointProtocol::Http1);
            service.health_path = normalized_ports
                .first()
                .and_then(|entry| entry.health_path.clone());
        }
    }
}

fn sanitize_kubernetes_settings(config: &mut LoopboxConfig) {
    let mut seen_clusters = std::collections::HashSet::new();
    config.global.kubernetes.clusters = config
        .global
        .kubernetes
        .clusters
        .iter()
        .filter_map(|cluster| {
            let name = normalize_kubernetes_name(&cluster.name);
            let context = cluster.context.trim().to_string();
            if name.is_empty() || context.is_empty() || !seen_clusters.insert(name.clone()) {
                return None;
            }

            let namespace = cluster.default_namespace.trim();
            Some(crate::loopbox::KubernetesClusterConfig {
                name,
                provider: cluster.provider,
                kubeconfig_path: cluster
                    .kubeconfig_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string()),
                context,
                default_namespace: if namespace.is_empty() {
                    "default".to_string()
                } else {
                    namespace.to_string()
                },
                wireguard: sanitize_wireguard_config(cluster.wireguard.as_ref()),
            })
        })
        .collect();
}

fn normalize_kubernetes_name(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn sanitize_wireguard_config(
    value: Option<&WireGuardTunnelConfig>,
) -> Option<WireGuardTunnelConfig> {
    let config = value?;
    let interface = config
        .interface
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let config_path = config
        .config_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let name = config.name.trim();
    let name = if name.is_empty() {
        interface
            .as_deref()
            .or(config_path.as_deref())
            .unwrap_or("wireguard")
            .to_string()
    } else {
        normalize_kubernetes_name(name)
    };

    Some(WireGuardTunnelConfig {
        name,
        mode: config.mode,
        interface,
        config_path,
        required: config.required,
    })
}

fn sanitize_service_ports(service: &crate::loopbox::ServiceConfig) -> Vec<ServicePortConfig> {
    let mut sanitized = Vec::new();
    let mut seen_ports = std::collections::HashSet::new();

    for entry in service_ports(service) {
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
        sanitized.push(ServicePortConfig {
            port: entry.port,
            protocol: entry.protocol,
            health_path,
            health_check_interval_secs: sanitize_health_check_interval_secs(
                entry.health_check_interval_secs,
            ),
        });
    }

    sanitized
}

pub(super) fn sanitize_health_check_interval_secs(value: Option<u64>) -> Option<u64> {
    let value = value?;
    if value == 0 {
        return None;
    }
    Some(value.clamp(2, 300))
}

fn sanitize_grpc_proto_paths(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn sanitize_container_service_config(
    value: Option<crate::loopbox::ContainerServiceConfig>,
) -> Option<crate::loopbox::ContainerServiceConfig> {
    let mut config = value?;
    config.image = config.image.trim().to_string();
    config.args = sanitize_container_string_list(&config.args);
    config.env = sanitize_container_string_list(&config.env);
    config.volumes = sanitize_container_string_list(&config.volumes);
    if config.image.is_empty() {
        return None;
    }
    Some(config)
}

fn sanitize_container_string_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn infer_endpoint_project_name(
    config: &LoopboxConfig,
    endpoint: &ProxyEndpointConfig,
) -> Option<String> {
    if let Some(project_name) = endpoint
        .project_name
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
    {
        if config.projects.contains_key(&project_name) {
            return Some(project_name);
        }
    }

    for (project_name, project) in &config.projects {
        if !project
            .ip
            .trim()
            .eq_ignore_ascii_case(endpoint.upstream_host.trim())
        {
            continue;
        }
        if project.services.iter().any(|service| {
            service_ports(service)
                .iter()
                .any(|entry| entry.port == endpoint.upstream_port)
        }) {
            return Some(project_name.clone());
        }
    }

    None
}

fn sanitize_proxy_endpoints(values: &[ProxyEndpointConfig]) -> Vec<ProxyEndpointConfig> {
    let mut listener_protocols = std::collections::HashMap::new();
    let mut grpc_seen_authorities = std::collections::HashSet::new();
    let mut non_grpc_seen_listeners = std::collections::HashSet::new();
    let mut sanitized = Vec::new();

    for (index, endpoint) in values.iter().enumerate() {
        if endpoint.listen_port == 0 || endpoint.upstream_port == 0 {
            continue;
        }

        let listen_host = endpoint.listen_host.trim();
        let upstream_host = endpoint.upstream_host.trim();
        if upstream_host.is_empty() {
            continue;
        }

        let normalized_listen_host = if listen_host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            listen_host.to_ascii_lowercase()
        };
        let normalized_upstream_host = upstream_host.to_string();
        let listener_key = (normalized_listen_host.clone(), endpoint.listen_port);
        if let Some(existing_protocol) = listener_protocols.get(&listener_key) {
            if existing_protocol != &endpoint.protocol {
                continue;
            }
        } else {
            listener_protocols.insert(listener_key.clone(), endpoint.protocol.clone());
        }

        let normalized_authority = endpoint.authority.as_ref().and_then(|value| {
            let trimmed = value.trim().to_lowercase();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        if endpoint.protocol == ProxyEndpointProtocol::GrpcH2c {
            let authority_key = normalized_authority
                .clone()
                .unwrap_or_else(|| "*".to_string());
            let grpc_dedupe_key = format!(
                "{}:{}:{authority_key}",
                normalized_listen_host, endpoint.listen_port
            );
            if !grpc_seen_authorities.insert(grpc_dedupe_key) {
                continue;
            }
        } else if !non_grpc_seen_listeners.insert(listener_key) {
            continue;
        }

        let normalized_name = if endpoint.name.trim().is_empty() {
            format!("endpoint-{}", index + 1)
        } else {
            endpoint.name.trim().to_string()
        };
        let normalized_project_name = endpoint
            .project_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let normalized_service_name = endpoint
            .service_name
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        sanitized.push(ProxyEndpointConfig {
            name: normalized_name,
            listen_host: normalized_listen_host,
            listen_port: endpoint.listen_port,
            protocol: endpoint.protocol.clone(),
            upstream_host: normalized_upstream_host,
            upstream_port: endpoint.upstream_port,
            authority: if endpoint.protocol == ProxyEndpointProtocol::GrpcH2c {
                normalized_authority
            } else {
                None
            },
            project_name: normalized_project_name,
            service_name: normalized_service_name,
        });
    }

    sanitized
}

fn sanitize_redaction_list(values: &[String], fallback: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut sanitized = Vec::new();
    for item in values {
        let normalized = item.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            sanitized.push(normalized);
        }
    }
    if sanitized.is_empty() {
        return fallback;
    }
    sanitized
}

fn default_redact_headers() -> Vec<String> {
    vec![
        "authorization".to_string(),
        "cookie".to_string(),
        "set-cookie".to_string(),
        "x-api-key".to_string(),
        "proxy-authorization".to_string(),
    ]
}

fn default_redact_query_keys() -> Vec<String> {
    vec![
        "token".to_string(),
        "key".to_string(),
        "secret".to_string(),
        "password".to_string(),
        "code".to_string(),
    ]
}

#[cfg(test)]
mod tests;
