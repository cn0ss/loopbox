use super::merge_service_env;
use super::projects::{effective_hosts_for_project, expand_tilde, validate_project_ip};
use super::{
    effective_reverse_proxy_status, proxy_redirect_configured, proxy_redirect_required,
    reverse_proxy_fallback_port, service_ports, service_runtime_status, DoctorFixAction,
    DoctorIssue, DoctorLevel, KubernetesConnectivityState, LoopboxConfig, ServiceRuntimeState,
};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub fn doctor_report(config: &LoopboxConfig) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    issues.extend(kubernetes_doctor_issues(config));
    if config.projects.is_empty() && config.global.kubernetes.clusters.is_empty() {
        issues.push(DoctorIssue::info(
            "No projects configured yet. Add one to create a sandbox identity.",
        ));
        return issues;
    }

    let hosts_path = crate::platform::hosts::hosts_file_path();
    let loopback_interface = crate::platform::networking::loopback_interface_label();
    let proxy_redirect_label = crate::platform::networking::proxy_redirect_label();
    let dns_flush_command = crate::platform::networking::dns_flush_command();

    let proxy_status = effective_reverse_proxy_status(config);
    if !proxy_status.running {
        issues.push(DoctorIssue::warning(
            None,
            "Reverse proxy is not running. URLs will use direct host:port mode.".to_string(),
        ));
    } else if proxy_status.using_fallback_port && proxy_redirect_required(config) {
        if proxy_redirect_configured(config) {
            issues.push(DoctorIssue::info(format!(
                "Reverse proxy runs on fallback port {} and {} is configured (:80 -> :{}).",
                reverse_proxy_fallback_port(),
                proxy_redirect_label,
                reverse_proxy_fallback_port()
            )));
        } else {
            issues.push(DoctorIssue::warning_with_fix(
                None,
                format!(
                    "Reverse proxy runs on fallback port {} but no system redirect is configured. Run System → Setup System to enable domain-only URLs on :80.",
                    reverse_proxy_fallback_port()
                ),
                DoctorFixAction::ApplySystemSetup,
            ));
        }
    }
    if let Some(note) = proxy_status.note.as_ref() {
        issues.push(DoctorIssue::info(note.clone()));
    }

    let mut ips: HashMap<String, Vec<String>> = HashMap::new();
    let mut hosts: HashMap<String, HashSet<String>> = HashMap::new();
    let mut ports_by_ip: HashMap<(String, u16), Vec<String>> = HashMap::new();
    let hosts_content = std::fs::read_to_string(hosts_path).ok();

    for (name, project) in &config.projects {
        let project_name = name.clone();

        let project_ip = project.ip.trim();

        if let Err(err) = validate_project_ip(&config.global, project_ip) {
            issues.push(DoctorIssue::error(Some(project_name.clone()), err));
        }
        if !loopback_alias_present(project_ip) {
            issues.push(DoctorIssue::warning_with_fix(
                Some(project_name.clone()),
                format!(
                    "Loopback address '{}' is missing on {}. Run System → Setup System.",
                    project_ip, loopback_interface
                ),
                DoctorFixAction::ApplySystemSetup,
            ));
        }

        let expanded_dir = expand_tilde(&project.dir);
        if !Path::new(&expanded_dir).exists() {
            issues.push(DoctorIssue::warning(
                Some(project_name.clone()),
                format!("Directory '{}' does not exist.", project.dir),
            ));
        }

        let mut seen_services = HashSet::new();
        for service in &project.services {
            if service.name.trim().is_empty() {
                issues.push(DoctorIssue::error(
                    Some(project_name.clone()),
                    "A service has an empty name.",
                ));
                continue;
            }

            if !seen_services.insert(service.name.clone()) {
                issues.push(DoctorIssue::error(
                    Some(project_name.clone()),
                    format!("Service '{}' is defined more than once.", service.name),
                ));
            }

            if super::features::doctor_requires_start_command(service)
                && service.command.trim().is_empty()
            {
                issues.push(DoctorIssue::error(
                    Some(project_name.clone()),
                    format!("Service '{}' is missing a start command.", service.name),
                ));
            }

            let expanded_workdir = expand_tilde(&service.workdir);
            if !Path::new(&expanded_workdir).exists() {
                issues.push(DoctorIssue::warning(
                    Some(project_name.clone()),
                    format!(
                        "Service '{}' workdir '{}' does not exist.",
                        service.name, service.workdir
                    ),
                ));
            }

            let effective_ports = service_ports(service);
            if effective_ports.is_empty() && service.health_path.is_some() {
                issues.push(DoctorIssue::warning(
                    Some(project_name.clone()),
                    format!(
                        "Service '{}' defines a health path but has no port configured.",
                        service.name
                    ),
                ));
            }

            let runtime = service_runtime_status(config, &project_name, &service.name).ok();
            for port_entry in &effective_ports {
                let port = port_entry.port;
                ports_by_ip
                    .entry((project_ip.to_string(), port))
                    .or_default()
                    .push(format!("{project_name}:{}", service.name));

                if runtime
                    .as_ref()
                    .is_some_and(|status| status.state == ServiceRuntimeState::Running)
                    && !port_reachable(project_ip, port, 120)
                {
                    issues.push(DoctorIssue::warning(
                        Some(project_name.clone()),
                        format!(
                            "Service '{}' is marked running but {}:{} is not reachable.",
                            service.name, project_ip, port
                        ),
                    ));
                }

                if port_entry.protocol == super::ProxyEndpointProtocol::TcpPassthrough
                    && port_entry.health_path.is_some()
                {
                    issues.push(DoctorIssue::warning(
                        Some(project_name.clone()),
                        format!(
                            "Service '{}' port {} has a health value but protocol tcp_passthrough; this health check is ignored.",
                            service.name, port
                        ),
                    ));
                }
            }

            match merge_service_env(config, &project_name, &service.name) {
                Ok(result) => {
                    if !result.warnings.is_empty() {
                        issues.push(DoctorIssue::warning(
                            Some(project_name.clone()),
                            format!(
                                "Service '{}' has {} env warning(s).",
                                service.name,
                                result.warnings.len()
                            ),
                        ));
                    }
                }
                Err(err) => issues.push(DoctorIssue::warning(
                    Some(project_name.clone()),
                    format!("Service '{}' env merge failed: {err}", service.name),
                )),
            }

            issues.extend(super::features::doctor_service_extra_issues(
                config,
                &project_name,
                project,
                service,
            ));
        }

        for host in
            effective_hosts_for_project(&project_name, project, &config.global.domain_suffix)
        {
            let host_clean = host.trim().to_lowercase();
            if host_clean.contains('#') {
                issues.push(DoctorIssue::error(
                    Some(project_name.clone()),
                    format!("Hostname '{host}' contains '#', which breaks {hosts_path} lines."),
                ));
            }
            if !is_secure_context_http_host(&host_clean) {
                issues.push(DoctorIssue::warning(
                    Some(project_name.clone()),
                    format!(
                        "Hostname '{host}' is not under .localhost. HTTP on this host is not a secure browser context (crypto.subtle/WebAuthn/PKCE). Use .localhost or HTTPS."
                    ),
                ));
            }
            if let Some(content) = hosts_content.as_deref() {
                let expected_ips = expected_host_mapping_ips(project_ip);
                if !hosts_file_contains_mapping_for_any_ip(content, &expected_ips, &host_clean) {
                    issues.push(DoctorIssue::warning_with_fix(
                        Some(project_name.clone()),
                        format!(
                            "Hostname '{}' is missing from {} for {}. Run System → Setup System.",
                            host_clean, hosts_path, project_ip
                        ),
                        DoctorFixAction::ApplySystemSetup,
                    ));
                }
            }
            let allow_loopback_resolution = is_secure_context_http_host(&host_clean);
            let resolution = resolve_host_ips(&host_clean);
            let resolves_expected = resolution.as_ref().is_ok_and(|ips| {
                ips.iter().any(|ip| {
                    ip.to_string() == project_ip || (allow_loopback_resolution && ip.is_loopback())
                })
            });
            if !resolves_expected {
                let resolution_detail = match &resolution {
                    Ok(ips) => format!("resolved to {}", format_resolved_ips(ips)),
                    Err(err) => format!("lookup failed: {err}"),
                };
                issues.push(DoctorIssue::warning_with_fix(
                    Some(project_name.clone()),
                    format!(
                        "Hostname '{}' does not currently resolve to {} ({resolution_detail}).",
                        host_clean, project_ip
                    ),
                    DoctorFixAction::CopyCommand {
                        label: "Copy DNS Flush".to_string(),
                        command: dns_flush_command.to_string(),
                    },
                ));
            }

            hosts
                .entry(host_clean)
                .or_default()
                .insert(project_name.clone());
        }

        ips.entry(project_ip.to_string())
            .or_default()
            .push(project_name.clone());
    }

    for (ip, owners) in ips {
        if owners.len() > 1 {
            issues.push(DoctorIssue::error(
                None,
                format!("IP {ip} is assigned more than once: {}.", owners.join(", ")),
            ));
        }
    }

    for ((ip, port), owners) in ports_by_ip {
        if owners.len() > 1 {
            issues.push(DoctorIssue::warning(
                None,
                format!(
                    "Port {port} on IP {ip} is configured by multiple services: {}.",
                    owners.join(", ")
                ),
            ));
        }
    }

    for (host, owners) in hosts {
        if owners.len() > 1 {
            let mut owner_list: Vec<String> = owners.into_iter().collect();
            owner_list.sort();
            issues.push(DoctorIssue::error(
                None,
                format!(
                    "Hostname {host} is assigned more than once: {}.",
                    owner_list.join(", ")
                ),
            ));
        }
    }

    issues.extend(super::features::doctor_global_extra_issues(config));

    if issues.is_empty() {
        issues.push(DoctorIssue::info(
            "Doctor found no conflicts. Loopback identities look consistent.",
        ));
    } else {
        issues.sort_by_key(|issue| match issue.level {
            DoctorLevel::Error => 0_u8,
            DoctorLevel::Warning => 1_u8,
            DoctorLevel::Info => 2_u8,
        });
    }

    issues
}

fn kubernetes_doctor_issues(config: &LoopboxConfig) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    if config.global.kubernetes.clusters.is_empty() {
        return issues;
    }

    if !command_available("kubectl", &["version", "--client"]) {
        issues.push(DoctorIssue::warning(
            None,
            "Kubernetes cluster management is configured but kubectl is not available on PATH.",
        ));
    }

    for cluster in &config.global.kubernetes.clusters {
        let label = format!("Kubernetes cluster '{}'", cluster.name);
        if let Some(path) = cluster.kubeconfig_path.as_ref() {
            let expanded = expand_tilde(path);
            if !Path::new(&expanded).exists() {
                issues.push(DoctorIssue::warning(
                    None,
                    format!("{label} kubeconfig '{}' does not exist.", path),
                ));
            }
        }

        if let Some(wireguard) = cluster.wireguard.as_ref() {
            if wireguard.interface.is_none() && wireguard.config_path.is_none() {
                issues.push(DoctorIssue::warning(
                    None,
                    format!(
                        "{label} WireGuard tunnel '{}' has no interface or config_path.",
                        wireguard.name
                    ),
                ));
            }
            if wireguard.required
                && (wireguard.interface.is_some() || wireguard.config_path.is_some())
            {
                match super::kubernetes::cluster_connectivity_state(cluster) {
                    KubernetesConnectivityState::Inactive => issues.push(DoctorIssue::warning(
                        None,
                        format!(
                            "{label} requires WireGuard tunnel '{}' but it is not active.",
                            wireguard.name
                        ),
                    )),
                    KubernetesConnectivityState::Error(err) => issues.push(DoctorIssue::warning(
                        None,
                        format!(
                            "{label} WireGuard tunnel '{}' could not be checked: {err}",
                            wireguard.name
                        ),
                    )),
                    KubernetesConnectivityState::NotConfigured
                    | KubernetesConnectivityState::Active => {}
                }
            }
        }
    }

    issues
}

fn command_available(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn is_secure_context_http_host(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost")
}

fn hosts_file_contains_mapping(content: &str, ip: &str, host: &str) -> bool {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let without_comment = line.split('#').next().unwrap_or("").trim();
        let mut columns = without_comment.split_whitespace();
        let Some(entry_ip) = columns.next() else {
            continue;
        };
        if entry_ip != ip {
            continue;
        }
        if columns.any(|candidate| candidate.eq_ignore_ascii_case(host)) {
            return true;
        }
    }
    false
}

fn hosts_file_contains_mapping_for_any_ip(content: &str, ips: &[String], host: &str) -> bool {
    ips.iter()
        .any(|ip| hosts_file_contains_mapping(content, ip, host))
}

fn expected_host_mapping_ips(project_ip: &str) -> Vec<String> {
    vec![project_ip.trim().to_string()]
}

fn resolve_host_ips(host: &str) -> Result<Vec<IpAddr>, String> {
    let mut addrs = (host, 80)
        .to_socket_addrs()
        .map_err(|err| err.to_string())?;
    let mut ips = Vec::new();
    for addr in &mut addrs {
        let ip = addr.ip();
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    Ok(ips)
}

fn format_resolved_ips(ips: &[IpAddr]) -> String {
    if ips.is_empty() {
        return "none".to_string();
    }
    ips.iter()
        .map(IpAddr::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn port_reachable(ip: &str, port: u16, timeout_ms: u64) -> bool {
    let Ok(mut addrs) = (ip, port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

fn loopback_alias_present(ip: &str) -> bool {
    crate::platform::networking::loopback_alias_present(ip)
}

#[cfg(test)]
mod tests {
    use super::{
        expected_host_mapping_ips, hosts_file_contains_mapping,
        hosts_file_contains_mapping_for_any_ip, is_secure_context_http_host, resolve_host_ips,
    };
    use crate::loopbox::{
        GlobalConfig, KubernetesClusterConfig, KubernetesProvider, KubernetesSettings,
        LoopboxConfig, WireGuardMode, WireGuardTunnelConfig,
    };
    use std::collections::BTreeMap;

    #[test]
    fn secure_context_host_detection() {
        assert!(is_secure_context_http_host("localhost"));
        assert!(is_secure_context_http_host("web.vereinsapp.localhost"));
        assert!(!is_secure_context_http_host("web.vereinsapp.test"));
        assert!(!is_secure_context_http_host("example.com"));
    }

    #[test]
    fn hosts_file_mapping_detection() {
        let hosts = r#"
127.0.0.1 localhost
127.0.0.30 frontend.app.niklasschmidt.dev server.app.niklasschmidt.dev # loopbox
"#;
        assert!(hosts_file_contains_mapping(
            hosts,
            "127.0.0.30",
            "frontend.app.niklasschmidt.dev"
        ));
        assert!(!hosts_file_contains_mapping(
            hosts,
            "127.0.0.31",
            "frontend.app.niklasschmidt.dev"
        ));
        assert!(!hosts_file_contains_mapping(
            hosts,
            "127.0.0.30",
            "missing.app.niklasschmidt.dev"
        ));
    }

    #[test]
    fn localhost_resolution_returns_loopback_ip() {
        let ips = resolve_host_ips("localhost").expect("localhost should resolve");
        assert!(ips.iter().any(|ip| ip.is_loopback()));
    }

    #[test]
    fn hosts_mapping_requires_project_ip() {
        let hosts = r#"
127.0.0.30 frontend.app.niklasschmidt.dev
"#;
        let expected = expected_host_mapping_ips("127.0.0.30");
        assert!(hosts_file_contains_mapping_for_any_ip(
            hosts,
            &expected,
            "frontend.app.niklasschmidt.dev"
        ));
    }

    #[test]
    fn expected_host_mapping_ips_trims_whitespace() {
        let expected = expected_host_mapping_ips(" 127.0.0.30 ");
        assert_eq!(expected, vec!["127.0.0.30".to_string()]);
    }

    #[test]
    fn doctor_reports_kubernetes_static_configuration_issues() {
        let config = LoopboxConfig {
            global: GlobalConfig {
                kubernetes: KubernetesSettings {
                    clusters: vec![KubernetesClusterConfig {
                        name: "prod".to_string(),
                        provider: KubernetesProvider::Remote,
                        kubeconfig_path: Some("/definitely/missing/loopbox-kubeconfig".to_string()),
                        context: "prod-context".to_string(),
                        default_namespace: "apps".to_string(),
                        wireguard: Some(WireGuardTunnelConfig {
                            name: "prod-wg".to_string(),
                            mode: WireGuardMode::WgQuick,
                            interface: None,
                            config_path: None,
                            required: true,
                        }),
                    }],
                },
                ..GlobalConfig::default()
            },
            projects: BTreeMap::new(),
        };

        let messages = super::kubernetes_doctor_issues(&config)
            .into_iter()
            .map(|issue| issue.message)
            .collect::<Vec<_>>();

        assert!(messages.iter().any(|message| message
            .contains("kubeconfig '/definitely/missing/loopbox-kubeconfig' does not exist")));
        assert!(messages.iter().any(|message| message
            .contains("WireGuard tunnel 'prod-wg' has no interface or config_path")));
    }
}
