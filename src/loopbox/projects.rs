use super::{
    default_domain_suffix, reverse_proxy_url_for_host, service_ports, AddProjectInput,
    ContainerServiceConfig, GlobalConfig, LoopboxConfig, OpenTarget, ProjectConfig,
    ProxyEndpointProtocol, ServiceConfig, ServiceEntry, ServicePortConfig, ServiceRuntimeKind,
    UpdateProjectInput,
};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::env;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub fn add_project(config: &mut LoopboxConfig, input: &AddProjectInput) -> Result<String, String> {
    let name = normalize_project_name(&input.name)?;
    if config.projects.contains_key(&name) {
        return Err(format!("Project '{name}' already exists."));
    }

    let dir = normalize_directory(&input.dir)?;
    let services = parse_services(&input.services, &dir)?;
    let candidate_hosts = generated_service_hosts(&name, &services, &config.global.domain_suffix);
    ensure_unique_hosts(config, &candidate_hosts)?;

    let ip = if input.ip.trim().is_empty() {
        allocate_ip(config, &format!("{name}:{dir}"))?
    } else {
        let requested = input.ip.trim().to_string();
        validate_project_ip(&config.global, &requested)?;
        if config
            .projects
            .values()
            .any(|project| project.ip == requested)
        {
            return Err(format!(
                "IP {requested} is already assigned to another project."
            ));
        }
        requested
    };

    let default_open_service = services
        .iter()
        .find(|service| !service_ports(service).is_empty())
        .or_else(|| services.first())
        .map(|service| service.name.clone());

    config.projects.insert(
        name.clone(),
        ProjectConfig {
            dir,
            ip,
            services,
            default_open_service,
            proxy_traffic_capture_enabled: None,
            proxy_traffic_capture_mode: None,
            grpc_proto_paths: Vec::new(),
            proxy_endpoints: Vec::new(),
        },
    );

    if let Err(err) = super::ensure_project_agent_guidance(config, &name) {
        config.projects.remove(&name);
        return Err(err);
    }

    Ok(name)
}

pub fn update_project(
    config: &mut LoopboxConfig,
    name: &str,
    input: &UpdateProjectInput,
) -> Result<(), String> {
    let project_name = name.trim();
    let existing = config
        .projects
        .get(project_name)
        .cloned()
        .ok_or_else(|| format!("Project '{project_name}' does not exist."))?;

    let dir = normalize_directory(&input.dir)?;
    let services = parse_services(&input.services, &dir)?;

    let candidate_hosts =
        generated_service_hosts(project_name, &services, &config.global.domain_suffix);
    ensure_unique_hosts_for_update(config, project_name, &candidate_hosts)?;

    let ip = if input.ip.trim().is_empty() {
        existing.ip.clone()
    } else {
        let requested = input.ip.trim().to_string();
        validate_project_ip(&config.global, &requested)?;
        if config
            .projects
            .iter()
            .filter(|(other_name, _)| other_name.as_str() != project_name)
            .any(|(_, project)| project.ip == requested)
        {
            return Err(format!(
                "IP {requested} is already assigned to another project."
            ));
        }
        requested
    };

    let mut default_open_service = existing.default_open_service.clone();
    if default_open_service
        .as_ref()
        .is_none_or(|svc| !services.iter().any(|service| &service.name == svc))
    {
        default_open_service = services
            .iter()
            .find(|service| !service_ports(service).is_empty())
            .or_else(|| services.first())
            .map(|service| service.name.clone());
    }

    let updated_project = ProjectConfig {
        dir,
        ip,
        services,
        default_open_service,
        proxy_traffic_capture_enabled: existing.proxy_traffic_capture_enabled,
        proxy_traffic_capture_mode: existing.proxy_traffic_capture_mode.clone(),
        grpc_proto_paths: existing.grpc_proto_paths.clone(),
        proxy_endpoints: existing.proxy_endpoints.clone(),
    };

    config
        .projects
        .insert(project_name.to_string(), updated_project);

    if let Err(err) = super::ensure_project_agent_guidance(config, project_name) {
        config.projects.insert(project_name.to_string(), existing);
        return Err(err);
    }

    Ok(())
}

pub fn remove_project(config: &mut LoopboxConfig, name: &str) -> Result<(), String> {
    if config.projects.remove(name).is_some() {
        Ok(())
    } else {
        Err(format!("Project '{name}' does not exist."))
    }
}

pub fn project_primary_host(config: &LoopboxConfig, name: &str) -> String {
    if let Some(project) = config.projects.get(name) {
        if let Some(service) = primary_service(project) {
            return service_host_for(name, &service.name, &config.global.domain_suffix);
        }
    }
    project_host_for(name, &config.global.domain_suffix)
}

pub fn project_env_exports(config: &LoopboxConfig, name: &str) -> Result<String, String> {
    let project = config
        .projects
        .get(name)
        .ok_or_else(|| format!("Project '{name}' not found."))?;

    let primary_host = project_primary_host(config, name);
    let mut lines = vec![
        format!("export LOOPBOX_PROJECT=\"{name}\""),
        format!("export LOOPBOX_DIR=\"{}\"", project.dir),
        format!("export LOOPBOX_IP=\"{}\"", project.ip),
        format!("export LOOPBOX_HOST=\"{primary_host}\""),
    ];

    for service in &project.services {
        let key = service.name.to_uppercase();
        let service_host = service_host_for(name, &service.name, &config.global.domain_suffix);
        let primary_port = primary_service_port(service);
        let url_port = primary_url_port(service);
        let all_ports = service_ports(service)
            .iter()
            .map(|entry| entry.port.to_string())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "export LOOPBOX_PORT_{key}=\"{}\"",
            primary_port
                .map(|value| value.to_string())
                .unwrap_or_default()
        ));
        lines.push(format!("export LOOPBOX_PORTS_{key}=\"{all_ports}\""));
        lines.push(format!(
            "export LOOPBOX_URL_{key}=\"{}\"",
            format_service_url(&service_host, url_port, Some(&project.ip))
        ));
        lines.push(format!(
            "export LOOPBOX_CMD_{key}=\"{}\"",
            service.command.replace('"', "\\\"")
        ));
        lines.push(format!(
            "export LOOPBOX_WORKDIR_{key}=\"{}\"",
            service.workdir.replace('"', "\\\"")
        ));
    }

    lines.push(String::new());
    Ok(lines.join("\n"))
}

pub fn open_url_for(
    config: &LoopboxConfig,
    name: &str,
    target: OpenTarget,
) -> Result<String, String> {
    let project = config
        .projects
        .get(name)
        .ok_or_else(|| format!("Project '{name}' not found."))?;

    let OpenTarget::Service(service_name) = target;
    let service = project
        .services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| format!("Service '{service_name}' not found in project '{name}'."))?;
    let host = service_host_for(name, &service_name, &config.global.domain_suffix);
    Ok(format_service_url(
        &host,
        primary_url_port(service),
        Some(&project.ip),
    ))
}

pub(super) fn normalize_domain_suffix(raw: &str) -> String {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        return default_domain_suffix();
    }
    trimmed.trim_start_matches('.').to_string()
}

fn normalize_project_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Project name is required.".to_string());
    }

    let normalized = trimmed.to_lowercase().replace(' ', "-");
    let valid = normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !valid {
        return Err("Project name may only contain letters, numbers, '-' and '_'.".to_string());
    }

    Ok(normalized)
}

fn normalize_service_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        return Err("Service name is required.".to_string());
    }

    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !valid {
        return Err(format!(
            "Service '{trimmed}' may only contain letters, numbers, '-' and '_'."
        ));
    }

    Ok(trimmed)
}

fn parse_services(
    entries: &[ServiceEntry],
    project_dir: &str,
) -> Result<Vec<ServiceConfig>, String> {
    let mut services = Vec::new();
    let mut seen_names = HashSet::new();

    for entry in entries {
        let service_name = entry.name.trim();
        if service_name.is_empty() {
            continue;
        }

        let name = normalize_service_name(service_name)?;
        if !seen_names.insert(name.clone()) {
            return Err(format!("Service '{name}' is defined more than once."));
        }

        let ports = parse_service_ports(entry, &name)?;
        let primary_port = ports.first().cloned();
        let runtime = parse_service_runtime(&entry.runtime, &name)?;
        let command = normalize_service_command(entry.command.trim());
        if matches!(runtime, ServiceRuntimeKind::Process) && command.is_empty() {
            return Err(format!(
                "Service '{name}' command is required (e.g. 'npm run dev')."
            ));
        }
        let container = parse_container_service_config(entry, &name, &runtime)?;

        let workdir = if entry.workdir.trim().is_empty() {
            project_dir.to_string()
        } else {
            normalize_directory(&entry.workdir)?
        };

        let env_files = parse_env_files(&entry.env_files);
        let depends_on = parse_depends_on(&entry.depends_on, &name);
        services.push(ServiceConfig {
            name,
            runtime,
            container,
            ports,
            port: primary_port.as_ref().map(|entry| entry.port),
            protocol: primary_port
                .as_ref()
                .map(|entry| entry.protocol.clone())
                .unwrap_or(ProxyEndpointProtocol::Http1),
            command,
            workdir,
            env_files,
            depends_on,
            autostart: entry.autostart,
            health_path: primary_port.and_then(|entry| entry.health_path),
        });
    }

    if services.is_empty() {
        return Err("At least one service is required.".to_string());
    }

    let service_names: HashSet<String> = services
        .iter()
        .map(|service| service.name.clone())
        .collect();
    for service in &services {
        for dependency in &service.depends_on {
            if !service_names.contains(dependency) {
                return Err(format!(
                    "Service '{}' depends on unknown service '{}'.",
                    service.name, dependency
                ));
            }
        }
    }

    Ok(services)
}

fn normalize_service_command(raw: &str) -> String {
    raw.replace('\u{2014}', "--")
        .replace(['\u{2013}', '\u{2212}'], "-")
}

fn parse_env_files(raw: &str) -> Vec<String> {
    raw.split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(expand_tilde)
        .collect()
}

fn parse_depends_on(raw: &str, service_name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .filter(|dependency| dependency != service_name)
        .filter(|dependency| seen.insert(dependency.clone()))
        .collect()
}

fn parse_service_runtime(raw: &str, service_name: &str) -> Result<ServiceRuntimeKind, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "process" => Ok(ServiceRuntimeKind::Process),
        "container" => Ok(ServiceRuntimeKind::Container),
        _ => Err(format!(
            "Service '{service_name}' has invalid runtime '{raw}'. Supported values: process, container."
        )),
    }
}

fn parse_container_service_config(
    entry: &ServiceEntry,
    service_name: &str,
    runtime: &ServiceRuntimeKind,
) -> Result<Option<ContainerServiceConfig>, String> {
    if matches!(runtime, ServiceRuntimeKind::Process) {
        return Ok(None);
    }

    let image = entry.container_image.trim().to_string();
    if image.is_empty() {
        return Err(format!(
            "Service '{service_name}' runtime 'container' requires an image."
        ));
    }

    Ok(Some(ContainerServiceConfig {
        image,
        args: parse_container_string_list(&entry.container_args),
        env: parse_container_string_list(&entry.container_env),
        volumes: parse_container_string_list(&entry.container_volumes),
        auto_remove: entry.container_auto_remove,
    }))
}

fn parse_container_string_list(raw: &str) -> Vec<String> {
    raw.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

fn parse_service_protocol(raw: &str, service_name: &str) -> Result<ProxyEndpointProtocol, String> {
    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() || normalized == "http1" || normalized == "http" {
        return Ok(ProxyEndpointProtocol::Http1);
    }
    if normalized == "grpc_h2c" || normalized == "grpc" {
        return Ok(ProxyEndpointProtocol::GrpcH2c);
    }
    if normalized == "tcp_passthrough" || normalized == "tcp" {
        return Ok(ProxyEndpointProtocol::TcpPassthrough);
    }

    Err(format!(
        "Service '{service_name}' has invalid protocol '{raw}'. Supported values: http1, grpc_h2c, tcp_passthrough."
    ))
}

fn parse_service_ports(
    entry: &ServiceEntry,
    service_name: &str,
) -> Result<Vec<ServicePortConfig>, String> {
    let mut parsed = Vec::new();
    let mut seen_ports = HashSet::new();

    for port_entry in &entry.ports {
        let Some(port) = parse_optional_port(&port_entry.port, service_name)? else {
            continue;
        };
        if !seen_ports.insert(port) {
            return Err(format!(
                "Service '{service_name}' defines port {port} more than once."
            ));
        }
        let protocol = parse_service_protocol(&port_entry.protocol, service_name)?;
        parsed.push(ServicePortConfig {
            port,
            protocol,
            health_path: option_trimmed(&port_entry.health_path),
        });
    }

    if !parsed.is_empty() {
        return Ok(parsed);
    }

    let fallback_port = parse_optional_port(&entry.port, service_name)?;
    if let Some(port) = fallback_port {
        let protocol = parse_service_protocol(&entry.protocol, service_name)?;
        return Ok(vec![ServicePortConfig {
            port,
            protocol,
            health_path: option_trimmed(&entry.health_path),
        }]);
    }

    Ok(Vec::new())
}

fn option_trimmed(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn dedupe_hosts(hosts: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for host in hosts {
        let cleaned = host.trim().to_lowercase();
        if cleaned.is_empty() {
            continue;
        }
        if seen.insert(cleaned.clone()) {
            deduped.push(cleaned);
        }
    }
    deduped
}

fn service_host_for(project_name: &str, service_name: &str, suffix: &str) -> String {
    let clean_project = project_name.trim().to_lowercase();
    let clean_service = service_name.trim().to_lowercase();

    if clean_service.is_empty() {
        project_host_for(&clean_project, suffix)
    } else {
        format!(
            "{clean_service}.{}",
            project_host_for(&clean_project, suffix)
        )
    }
}

fn project_host_for(project_name: &str, suffix: &str) -> String {
    let clean_project = project_name.trim().to_lowercase();
    let clean_suffix = normalize_domain_suffix(suffix);
    format!("{clean_project}.{clean_suffix}")
}

fn primary_service(project: &ProjectConfig) -> Option<&ServiceConfig> {
    if let Some(default_name) = project.default_open_service.as_ref() {
        if let Some(service) = project
            .services
            .iter()
            .find(|service| &service.name == default_name)
        {
            return Some(service);
        }
    }
    project
        .services
        .iter()
        .find(|service| primary_url_port(service).is_some())
        .or_else(|| project.services.first())
}

fn primary_service_port(service: &ServiceConfig) -> Option<u16> {
    service_ports(service).first().map(|entry| entry.port)
}

fn primary_url_port(service: &ServiceConfig) -> Option<u16> {
    let ports = service_ports(service);
    ports
        .iter()
        .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
        .map(|entry| entry.port)
        .or_else(|| ports.first().map(|entry| entry.port))
}

pub(super) fn generated_service_hosts(
    project_name: &str,
    services: &[ServiceConfig],
    suffix: &str,
) -> Vec<String> {
    let mut generated = Vec::new();
    for service in services {
        generated.push(service_host_for(project_name, &service.name, suffix));
    }
    dedupe_hosts(generated)
}

pub(super) fn effective_hosts_for_project(
    project_name: &str,
    project: &ProjectConfig,
    suffix: &str,
) -> Vec<String> {
    generated_service_hosts(project_name, &project.services, suffix)
}

fn ensure_unique_hosts(config: &LoopboxConfig, candidate_hosts: &[String]) -> Result<(), String> {
    let mut existing = HashMap::new();
    for (project_name, project) in &config.projects {
        for host in effective_hosts_for_project(project_name, project, &config.global.domain_suffix)
        {
            existing.insert(host.to_lowercase(), project_name.clone());
        }
    }

    for host in candidate_hosts {
        if let Some(owner) = existing.get(&host.to_lowercase()) {
            return Err(format!(
                "Hostname '{host}' is already used by project '{owner}'."
            ));
        }
    }
    Ok(())
}

fn ensure_unique_hosts_for_update(
    config: &LoopboxConfig,
    project_name: &str,
    candidate_hosts: &[String],
) -> Result<(), String> {
    let mut existing = HashMap::new();
    for (other_name, project) in &config.projects {
        if other_name == project_name {
            continue;
        }
        for host in effective_hosts_for_project(other_name, project, &config.global.domain_suffix) {
            existing.insert(host.to_lowercase(), other_name.clone());
        }
    }

    for host in candidate_hosts {
        if let Some(owner) = existing.get(&host.to_lowercase()) {
            return Err(format!(
                "Hostname '{host}' is already used by project '{owner}'."
            ));
        }
    }
    Ok(())
}

fn normalize_directory(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Project directory is required.".to_string());
    }
    Ok(expand_tilde(trimmed))
}

pub(super) fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = env::var_os("HOME") {
            let mut expanded = PathBuf::from(home);
            if path.len() > 2 {
                expanded.push(&path[2..]);
            }
            return expanded.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn parse_optional_port(raw: &str, label: &str) -> Result<Option<u16>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let port = trimmed
        .parse::<u16>()
        .map_err(|_| format!("{label} port must be a number between 1 and 65535."))?;
    if port == 0 {
        return Err(format!("{label} port must be > 0."));
    }
    Ok(Some(port))
}

fn allocate_ip(config: &LoopboxConfig, seed: &str) -> Result<String, String> {
    let start = config.global.ip_range_start;
    let end = config.global.ip_range_end;
    if start > end {
        return Err("Invalid IP range configuration.".to_string());
    }

    let span = (u16::from(end) - u16::from(start)) + 1;
    let mut used_octets = HashSet::new();
    for project in config.projects.values() {
        if let Ok(octet) = ip_octet(&config.global, &project.ip) {
            used_octets.insert(octet);
        }
    }

    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let offset = (hasher.finish() % u64::from(span)) as u16;
    let mut current = u16::from(start) + offset;

    for _ in 0..span {
        let octet = current as u8;
        if !used_octets.contains(&octet) {
            return Ok(format!("{}{}", config.global.ip_base, octet));
        }
        current += 1;
        if current > u16::from(end) {
            current = u16::from(start);
        }
    }

    Err(format!(
        "No free loopback IPs left in {}{}..{}.",
        config.global.ip_base, config.global.ip_range_start, config.global.ip_range_end
    ))
}

fn ip_octet(global: &GlobalConfig, ip: &str) -> Result<u8, String> {
    let trimmed_ip = ip.trim();
    let suffix = trimmed_ip
        .strip_prefix(&global.ip_base)
        .ok_or_else(|| format!("IP {trimmed_ip} must start with '{}'.", global.ip_base))?;
    if suffix.contains('.') {
        return Err(format!(
            "IP {trimmed_ip} is not within '{}X' format.",
            global.ip_base
        ));
    }

    suffix
        .parse::<u8>()
        .map_err(|_| format!("IP {trimmed_ip} has an invalid last octet."))
}

pub(super) fn validate_project_ip(global: &GlobalConfig, ip: &str) -> Result<(), String> {
    let octet = ip_octet(global, ip)?;
    if octet < global.ip_range_start || octet > global.ip_range_end {
        return Err(format!(
            "IP {ip} is outside configured range {}{}..{}.",
            global.ip_base, global.ip_range_start, global.ip_range_end
        ));
    }

    let parsed = ip
        .trim()
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| format!("IP {ip} is not a valid IPv4 address."))?;
    if !parsed.is_loopback() {
        return Err(format!("IP {ip} is not in loopback space."));
    }
    Ok(())
}

fn format_http_url(host: &str, port: Option<u16>) -> String {
    match port {
        Some(80) | None => format!("http://{host}"),
        Some(port) => format!("http://{host}:{port}"),
    }
}

fn format_service_url(host: &str, port: Option<u16>, direct_ip: Option<&str>) -> String {
    if port.is_some() {
        if let Some(proxy_url) = reverse_proxy_url_for_host(host) {
            return proxy_url;
        }
        if let (Some(ip), Some(service_port)) = (direct_ip, port) {
            return format_http_url(ip, Some(service_port));
        }
    }
    format_http_url(host, port)
}

#[cfg(test)]
mod tests;
