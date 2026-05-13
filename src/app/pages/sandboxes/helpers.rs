use super::*;

pub(super) fn service_status_summary(
    statuses: &BTreeMap<String, ServiceRuntimeSnapshot>,
) -> Vec<(&'static str, usize)> {
    let mut running = 0;
    let mut starting = 0;
    let mut unhealthy = 0;
    let mut crashed = 0;
    let mut stopped = 0;
    for s in statuses.values() {
        match s.state {
            ServiceRuntimeState::Running => running += 1,
            ServiceRuntimeState::Starting => starting += 1,
            ServiceRuntimeState::Unhealthy => unhealthy += 1,
            ServiceRuntimeState::Crashed => crashed += 1,
            ServiceRuntimeState::Stopped => stopped += 1,
        }
    }
    vec![
        ("running", running),
        ("starting", starting),
        ("unhealthy", unhealthy),
        ("crashed", crashed),
        ("stopped", stopped),
    ]
}

pub(super) fn service_protocol_value(protocol: &ProxyEndpointProtocol) -> &'static str {
    match protocol {
        ProxyEndpointProtocol::Http1 => "http1",
        ProxyEndpointProtocol::GrpcH2c => "grpc_h2c",
        ProxyEndpointProtocol::TcpPassthrough => "tcp_passthrough",
    }
}

pub(super) fn normalize_service_command_input(raw: &str) -> String {
    raw.replace('\u{2014}', "--")
        .replace(['\u{2013}', '\u{2212}'], "-")
}

pub(super) fn parse_service_protocol(raw: &str) -> Option<ProxyEndpointProtocol> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "http1" => Some(ProxyEndpointProtocol::Http1),
        "grpc_h2c" => Some(ProxyEndpointProtocol::GrpcH2c),
        "tcp_passthrough" => Some(ProxyEndpointProtocol::TcpPassthrough),
        _ => None,
    }
}

pub(super) fn blank_service_port_entry() -> ServicePortEntry {
    ServicePortEntry {
        port: String::new(),
        protocol: "http1".to_string(),
        health_path: String::new(),
        health_check_interval_secs: String::new(),
    }
}

pub(super) fn service_entry_port_rows(entry: &ServiceEntry) -> Vec<ServicePortEntry> {
    if !entry.ports.is_empty() {
        return entry.ports.clone();
    }

    if !entry.port.trim().is_empty()
        || !entry.health_path.trim().is_empty()
        || !entry.protocol.trim().is_empty()
    {
        return vec![ServicePortEntry {
            port: entry.port.clone(),
            protocol: if parse_service_protocol(&entry.protocol).is_some() {
                entry.protocol.clone()
            } else {
                "http1".to_string()
            },
            health_path: entry.health_path.clone(),
            health_check_interval_secs: String::new(),
        }];
    }

    vec![blank_service_port_entry()]
}

pub(super) fn sync_service_entry_primary_port(entry: &mut ServiceEntry) {
    if entry.ports.is_empty() {
        entry.ports.push(blank_service_port_entry());
    }

    for port_entry in &mut entry.ports {
        if parse_service_protocol(&port_entry.protocol).is_none() {
            port_entry.protocol = "http1".to_string();
        }
    }

    let primary = entry.ports.iter().find(|port_entry| {
        !port_entry.port.trim().is_empty() || !port_entry.health_path.trim().is_empty()
    });

    if let Some(primary) = primary.or_else(|| entry.ports.first()) {
        entry.port = primary.port.trim().to_string();
        entry.protocol = service_protocol_value(
            &parse_service_protocol(&primary.protocol).unwrap_or(ProxyEndpointProtocol::Http1),
        )
        .to_string();
        entry.health_path = primary.health_path.trim().to_string();
    } else {
        entry.port.clear();
        entry.protocol = "http1".to_string();
        entry.health_path.clear();
    }
}

pub(super) fn service_entry_configured_ports(entry: &ServiceEntry) -> Vec<u16> {
    let mut ports = Vec::new();
    let mut seen = BTreeSet::new();
    for port_entry in service_entry_port_rows(entry) {
        let raw = port_entry.port.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(parsed) = raw.parse::<u16>() else {
            continue;
        };
        if seen.insert(parsed) {
            ports.push(parsed);
        }
    }
    ports
}

pub(super) fn project_proxy_endpoint_protocol_value(
    protocol: &ProxyEndpointProtocol,
) -> &'static str {
    match protocol {
        ProxyEndpointProtocol::Http1 => "http1",
        ProxyEndpointProtocol::GrpcH2c => "grpc_h2c",
        ProxyEndpointProtocol::TcpPassthrough => "tcp_passthrough",
    }
}

pub(super) fn parse_project_proxy_endpoint_protocol(raw: &str) -> Option<ProxyEndpointProtocol> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "http1" => Some(ProxyEndpointProtocol::Http1),
        "grpc_h2c" => Some(ProxyEndpointProtocol::GrpcH2c),
        "tcp_passthrough" => Some(ProxyEndpointProtocol::TcpPassthrough),
        _ => None,
    }
}

pub(super) fn optional_trimmed_endpoint_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn normalize_grpc_proto_paths_for_form(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(super) fn default_project_proxy_endpoint_config(project_name: &str) -> ProxyEndpointConfig {
    ProxyEndpointConfig {
        name: "endpoint".to_string(),
        listen_host: "127.0.0.1".to_string(),
        listen_port: 50051,
        protocol: ProxyEndpointProtocol::GrpcH2c,
        upstream_host: "127.0.0.1".to_string(),
        upstream_port: 50051,
        authority: None,
        project_name: Some(project_name.trim().to_lowercase()),
        service_name: None,
    }
}

pub(super) fn sanitize_project_proxy_endpoints_for_form(
    project_name: &str,
    values: &mut [ProxyEndpointConfig],
) {
    let normalized_project = project_name.trim().to_lowercase();
    for (index, endpoint) in values.iter_mut().enumerate() {
        let normalized_name = endpoint.name.trim();
        endpoint.name = if normalized_name.is_empty() {
            format!("endpoint-{}", index + 1)
        } else {
            normalized_name.to_string()
        };

        let normalized_listen_host = endpoint.listen_host.trim().to_ascii_lowercase();
        endpoint.listen_host = if normalized_listen_host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            normalized_listen_host
        };
        endpoint.upstream_host = endpoint.upstream_host.trim().to_string();
        endpoint.service_name = endpoint
            .service_name
            .as_deref()
            .and_then(optional_trimmed_endpoint_value)
            .map(|value| value.to_ascii_lowercase());
        endpoint.project_name = Some(normalized_project.clone());
        endpoint.authority = if endpoint.protocol == ProxyEndpointProtocol::GrpcH2c {
            endpoint
                .authority
                .as_deref()
                .and_then(optional_trimmed_endpoint_value)
                .map(|value| value.to_ascii_lowercase())
        } else {
            None
        };
    }
}

pub(super) fn validate_project_proxy_endpoints_for_form(
    values: &[ProxyEndpointConfig],
) -> Result<(), String> {
    let mut listener_protocols = std::collections::HashMap::new();
    let mut grpc_seen_authorities = std::collections::HashSet::new();
    let mut non_grpc_seen_listeners = std::collections::HashSet::new();

    for endpoint in values {
        let name = endpoint.name.trim();
        if name.is_empty() {
            return Err("Endpoint name must not be empty.".to_string());
        }
        if endpoint.listen_port == 0 {
            return Err(format!("Endpoint '{name}' has invalid listen port 0."));
        }
        if endpoint.upstream_port == 0 {
            return Err(format!("Endpoint '{name}' has invalid upstream port 0."));
        }
        let listen_host = endpoint.listen_host.trim();
        if listen_host.is_empty() {
            return Err(format!("Endpoint '{name}' requires a listen host."));
        }
        let upstream_host = endpoint.upstream_host.trim();
        if upstream_host.is_empty() {
            return Err(format!("Endpoint '{name}' requires an upstream host."));
        }

        let listener_key = (listen_host.to_ascii_lowercase(), endpoint.listen_port);
        if let Some(existing_protocol) = listener_protocols.get(&listener_key) {
            if existing_protocol != &endpoint.protocol {
                return Err(format!(
                    "Endpoint '{name}' conflicts on {}:{}: all routes on one listener must use the same protocol.",
                    listener_key.0, listener_key.1
                ));
            }
        } else {
            listener_protocols.insert(listener_key.clone(), endpoint.protocol.clone());
        }

        match endpoint.protocol {
            ProxyEndpointProtocol::GrpcH2c => {
                let authority_key = endpoint
                    .authority
                    .as_deref()
                    .and_then(optional_trimmed_endpoint_value)
                    .map(|value| value.to_ascii_lowercase())
                    .unwrap_or_else(|| "*".to_string());
                let dedupe_key = format!("{}:{}:{authority_key}", listener_key.0, listener_key.1);
                if !grpc_seen_authorities.insert(dedupe_key) {
                    return Err(format!(
                        "Endpoint '{name}' duplicates gRPC authority route on {}:{}.",
                        listener_key.0, listener_key.1
                    ));
                }
            }
            ProxyEndpointProtocol::Http1 | ProxyEndpointProtocol::TcpPassthrough => {
                if endpoint
                    .authority
                    .as_deref()
                    .and_then(optional_trimmed_endpoint_value)
                    .is_some()
                {
                    return Err(format!(
                        "Endpoint '{name}' uses authority, but authority is only valid for grpc_h2c routes."
                    ));
                }
                if !non_grpc_seen_listeners.insert(listener_key.clone()) {
                    return Err(format!(
                        "Endpoint '{name}' duplicates listener {}:{}; only grpc_h2c supports multiple routes per listener.",
                        listener_key.0, listener_key.1
                    ));
                }
            }
        }
    }

    Ok(())
}

pub(super) fn runtime_badge(
    snapshot: Option<&ServiceRuntimeSnapshot>,
) -> (&'static str, &'static str) {
    let Some(snapshot) = snapshot else {
        return ("stopped", "");
    };

    match snapshot.state {
        ServiceRuntimeState::Stopped => ("stopped", ""),
        ServiceRuntimeState::Starting => ("starting", "status-dot-starting"),
        ServiceRuntimeState::Running => ("running", "status-dot-running"),
        ServiceRuntimeState::Unhealthy => ("unhealthy", "status-dot-unhealthy"),
        ServiceRuntimeState::Crashed => ("crashed", "status-dot-crashed"),
    }
}

pub(super) fn svc_card_border_class(snapshot: Option<&ServiceRuntimeSnapshot>) -> &'static str {
    let Some(snapshot) = snapshot else {
        return "";
    };
    match snapshot.state {
        ServiceRuntimeState::Running => "svc-card-running",
        ServiceRuntimeState::Starting => "svc-card-starting",
        ServiceRuntimeState::Unhealthy => "svc-card-unhealthy",
        ServiceRuntimeState::Crashed => "svc-card-crashed",
        ServiceRuntimeState::Stopped => "",
    }
}

pub(super) fn effective_log_selection(
    services: &[crate::loopbox::ServiceConfig],
    current: Option<String>,
) -> Option<String> {
    match current {
        Some(selected) if services.iter().any(|service| service.name == selected) => Some(selected),
        _ => services.first().map(|service| service.name.clone()),
    }
}

pub(super) fn compact_env_source(project_dir: &str, source: &str) -> String {
    let prefix = format!("{}/", project_dir.trim_end_matches('/'));
    source.strip_prefix(&prefix).unwrap_or(source).to_string()
}

pub(super) fn redact_env_value(key: &str, value: &str) -> String {
    if !is_sensitive_env_key(key) {
        return value.to_string();
    }

    if value.len() <= 6 {
        return "******".to_string();
    }
    let start = &value[..3];
    let end = &value[value.len() - 2..];
    format!("{start}***{end}")
}

pub(super) fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PRIVATE",
        "API_KEY",
        "ACCESS_KEY",
        "CLIENT_SECRET",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

pub(super) fn log_line_outer_class(line: &str) -> &'static str {
    if line.starts_with("[stderr]") {
        "log-line log-line-err"
    } else {
        "log-line"
    }
}

pub(super) fn log_line_text_class(line: &str) -> &'static str {
    if line.starts_with("[stderr]") {
        "log-text log-text-err"
    } else {
        "log-text"
    }
}

pub(super) fn strip_log_prefix(line: &str) -> String {
    let unprefixed = if let Some(rest) = line.strip_prefix("[stdout] ") {
        rest
    } else if let Some(rest) = line.strip_prefix("[stderr] ") {
        rest
    } else {
        line
    };
    strip_terminal_control_sequences(unprefixed)
}

fn strip_terminal_control_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0_usize;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        if (0x40..=0x7e).contains(&c) {
                            break;
                        }
                    }
                }
                b']' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == 0x07 {
                            i += 1;
                            break;
                        }
                        if c == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => {
                    i += 1;
                }
            }
            continue;
        }

        if byte < 0x20 && byte != b'\t' {
            i += 1;
            continue;
        }
        if byte == 0x7f {
            i += 1;
            continue;
        }

        output.push(byte);
        i += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}
