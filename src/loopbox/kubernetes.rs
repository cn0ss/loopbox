mod parsers;
#[cfg(test)]
mod tests;
mod topology;
mod types;

pub use parsers::{
    parse_endpoint_slice_snapshots, parse_event_snapshots, parse_ingress_snapshots,
    parse_namespace_names, parse_node_snapshots, parse_pod_snapshots, parse_service_snapshots,
    parse_workload_snapshots,
};
pub use topology::build_kubernetes_topology;
use types::KubectlInvocation;
pub use types::{
    KubernetesClusterDiscovery, KubernetesClusterImport, KubernetesClusterSnapshot,
    KubernetesConnectivityState, KubernetesEndpointSliceSnapshot, KubernetesEventSnapshot,
    KubernetesIngressSnapshot, KubernetesNamespaceSnapshot, KubernetesNodeSnapshot,
    KubernetesPodSnapshot, KubernetesServiceSnapshot, KubernetesTopologyEdge,
    KubernetesTopologyNode, KubernetesTopologySnapshot, KubernetesWorkloadSnapshot,
};

use super::{
    KubernetesClusterConfig, KubernetesProvider, LoopboxConfig, WireGuardMode,
    WireGuardTunnelConfig,
};
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::process::Command;

pub fn discover_kubernetes_clusters(
    config: &LoopboxConfig,
) -> Result<Vec<KubernetesClusterDiscovery>, String> {
    let kubeconfig_paths = discover_kubeconfig_paths();
    let mut discovered = Vec::new();
    let mut seen = HashSet::new();

    for kubeconfig_path in kubeconfig_paths {
        let contexts = kubectl_context_names(kubeconfig_path.as_deref())?;
        for context in contexts {
            let key = format!(
                "{}|{}",
                kubeconfig_path.clone().unwrap_or_default(),
                context
            );
            if !seen.insert(key) {
                continue;
            }
            let already_configured =
                cluster_already_configured(config, kubeconfig_path.as_deref(), &context);
            let default_namespace = kubectl_default_namespace(kubeconfig_path.as_deref(), &context)
                .unwrap_or_else(|_| "default".to_string());
            let reachability = kubectl_context_reachable(kubeconfig_path.as_deref(), &context);
            let (reachable, error) = match reachability {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err)),
            };
            discovered.push(KubernetesClusterDiscovery {
                name: unique_cluster_name(config, &discovered, &context),
                provider: KubernetesProvider::KubeconfigContext,
                kubeconfig_path: kubeconfig_path.clone(),
                context,
                default_namespace,
                already_configured,
                reachable,
                error,
            });
        }
    }

    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(discovered)
}

pub fn import_kubernetes_clusters(
    config: &mut LoopboxConfig,
    imports: &[KubernetesClusterImport],
) -> Result<usize, String> {
    let mut added = 0_usize;
    for import in imports {
        let context = import.context.trim();
        if context.is_empty() {
            continue;
        }
        let kubeconfig_path = import
            .kubeconfig_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if cluster_already_configured(config, kubeconfig_path.as_deref(), context) {
            continue;
        }
        let discovery_stub = Vec::<KubernetesClusterDiscovery>::new();
        let name = import
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(normalize_cluster_name)
            .unwrap_or_else(|| unique_cluster_name(config, &discovery_stub, context));
        let namespace = import
            .default_namespace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default")
            .to_string();
        config
            .global
            .kubernetes
            .clusters
            .push(KubernetesClusterConfig {
                name,
                provider: import.provider,
                kubeconfig_path,
                context: context.to_string(),
                default_namespace: namespace,
                wireguard: None,
            });
        added = added.saturating_add(1);
    }
    Ok(added)
}

pub fn cluster_summaries(config: &LoopboxConfig) -> Result<Vec<KubernetesClusterSnapshot>, String> {
    config
        .global
        .kubernetes
        .clusters
        .iter()
        .map(|cluster| cluster_snapshot(config, &cluster.name))
        .collect()
}

pub fn cluster_snapshot(
    config: &LoopboxConfig,
    cluster_name: &str,
) -> Result<KubernetesClusterSnapshot, String> {
    cluster_snapshot_for_namespace(config, cluster_name, None)
}

pub fn cluster_snapshot_for_namespace(
    config: &LoopboxConfig,
    cluster_name: &str,
    namespace_override: Option<&str>,
) -> Result<KubernetesClusterSnapshot, String> {
    let cluster = config
        .global
        .kubernetes
        .clusters
        .iter()
        .find(|cluster| cluster.name == cluster_name)
        .ok_or_else(|| format!("Kubernetes cluster '{cluster_name}' not found."))?;

    let connectivity = cluster_connectivity_state(cluster);
    let mut last_error = match &connectivity {
        KubernetesConnectivityState::Error(err) => Some(err.clone()),
        _ => None,
    };

    let namespaces = match run_kubectl_json(cluster, None, &["get", "namespaces", "-o", "json"])
        .and_then(|stdout| parse_namespace_names(&stdout))
    {
        Ok(names) => names
            .into_iter()
            .map(|name| KubernetesNamespaceSnapshot { name })
            .collect(),
        Err(err) => {
            last_error.get_or_insert(err);
            Vec::new()
        }
    };

    let selected_namespace = namespace_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(cluster.default_namespace.as_str())
        .to_string();
    let namespace = selected_namespace.as_str();
    let mut warnings = Vec::new();

    let nodes = match run_kubectl_json(cluster, None, &["get", "nodes", "-o", "json"])
        .and_then(|stdout| parse_node_snapshots(&stdout))
    {
        Ok(items) => items,
        Err(err) => {
            warnings.push(format!("nodes: {err}"));
            Vec::new()
        }
    };

    let mut workloads = Vec::new();
    for (kind, resource) in [
        ("deployment", "deployments"),
        ("statefulset", "statefulsets"),
        ("daemonset", "daemonsets"),
        ("replicaset", "replicasets"),
        ("job", "jobs"),
        ("cronjob", "cronjobs"),
    ] {
        match run_kubectl_json(cluster, Some(namespace), &["get", resource, "-o", "json"])
            .and_then(|stdout| parse_workload_snapshots(kind, &stdout))
        {
            Ok(mut items) => workloads.append(&mut items),
            Err(err) => {
                last_error.get_or_insert(err.clone());
                warnings.push(format!("{resource}: {err}"));
            }
        }
    }

    let pods = match run_kubectl_json(cluster, Some(namespace), &["get", "pods", "-o", "json"])
        .and_then(|stdout| parse_pod_snapshots(&stdout))
    {
        Ok(items) => items,
        Err(err) => {
            warnings.push(format!("pods: {err}"));
            Vec::new()
        }
    };

    let mut services =
        match run_kubectl_json(cluster, Some(namespace), &["get", "services", "-o", "json"])
            .and_then(|stdout| parse_service_snapshots(&stdout))
        {
            Ok(items) => items,
            Err(err) => {
                last_error.get_or_insert(err.clone());
                warnings.push(format!("services: {err}"));
                Vec::new()
            }
        };

    let endpoint_slices = match run_kubectl_json(
        cluster,
        Some(namespace),
        &["get", "endpointslices", "-o", "json"],
    )
    .and_then(|stdout| parse_endpoint_slice_snapshots(&stdout))
    {
        Ok(items) => items,
        Err(err) => {
            warnings.push(format!("endpointslices: {err}"));
            Vec::new()
        }
    };
    let ingresses = match run_kubectl_json(
        cluster,
        Some(namespace),
        &["get", "ingresses", "-o", "json"],
    )
    .and_then(|stdout| parse_ingress_snapshots(&stdout))
    {
        Ok(items) => items,
        Err(err) => {
            warnings.push(format!("ingresses: {err}"));
            Vec::new()
        }
    };
    let events = match run_kubectl_json(cluster, Some(namespace), &["get", "events", "-o", "json"])
        .and_then(|stdout| parse_event_snapshots(&stdout))
    {
        Ok(items) => items,
        Err(err) => {
            warnings.push(format!("events: {err}"));
            Vec::new()
        }
    };

    enrich_service_relationships(&mut services, &pods, &endpoint_slices, &ingresses);

    let mut snapshot = KubernetesClusterSnapshot {
        name: cluster.name.clone(),
        provider: cluster.provider,
        context: cluster.context.clone(),
        default_namespace: cluster.default_namespace.clone(),
        selected_namespace,
        connectivity,
        namespaces,
        workloads,
        services,
        nodes,
        pods,
        ingresses,
        endpoint_slices,
        events,
        topology: KubernetesTopologySnapshot::default(),
        warnings,
        last_error,
    };
    snapshot.topology = build_kubernetes_topology(&snapshot);
    Ok(snapshot)
}

pub fn start_cluster_wireguard(config: &LoopboxConfig, cluster_name: &str) -> Result<(), String> {
    let cluster = find_cluster(config, cluster_name)?;
    let wireguard = cluster
        .wireguard
        .as_ref()
        .ok_or_else(|| format!("Kubernetes cluster '{cluster_name}' has no WireGuard tunnel."))?;
    run_wg_quick(wireguard, "up")
}

pub fn stop_cluster_wireguard(config: &LoopboxConfig, cluster_name: &str) -> Result<(), String> {
    let cluster = find_cluster(config, cluster_name)?;
    let wireguard = cluster
        .wireguard
        .as_ref()
        .ok_or_else(|| format!("Kubernetes cluster '{cluster_name}' has no WireGuard tunnel."))?;
    run_wg_quick(wireguard, "down")
}

pub fn wireguard_active_from_show_output(
    success: bool,
    stdout: &str,
    stderr: &str,
) -> Result<bool, String> {
    if success {
        return Ok(!stdout.trim().is_empty());
    }
    let detail = stderr.trim();
    if detail.is_empty()
        || detail.contains("Unable to access interface")
        || detail.contains("No such device")
        || detail.contains("does not exist")
    {
        return Ok(false);
    }
    Err(detail.to_string())
}

pub(crate) fn kubectl_invocation(
    cluster: &KubernetesClusterConfig,
    namespace: Option<&str>,
    trailing: &[&str],
) -> KubectlInvocation {
    let mut args = vec!["--context".to_string(), cluster.context.clone()];
    if let Some(namespace) = namespace.filter(|value| !value.trim().is_empty()) {
        args.push("--namespace".to_string());
        args.push(namespace.trim().to_string());
    }
    args.extend(trailing.iter().map(|arg| arg.to_string()));
    KubectlInvocation {
        program: "kubectl".to_string(),
        args,
        kubeconfig_env: cluster.kubeconfig_path.clone(),
    }
}

fn discover_kubeconfig_paths() -> Vec<Option<String>> {
    let mut paths = Vec::new();
    if let Some(raw) = env::var_os("KUBECONFIG") {
        for path in env::split_paths(&raw) {
            let text = path.to_string_lossy().trim().to_string();
            if !text.is_empty() {
                paths.push(Some(text));
            }
        }
    }
    if paths.is_empty() {
        if let Some(home) = env::var_os("HOME") {
            let default_path = PathBuf::from(home).join(".kube").join("config");
            if default_path.exists() {
                paths.push(Some(default_path.to_string_lossy().to_string()));
            } else {
                paths.push(None);
            }
        } else {
            paths.push(None);
        }
    }
    paths
}

fn kubectl_context_names(kubeconfig_path: Option<&str>) -> Result<Vec<String>, String> {
    let mut command = Command::new("kubectl");
    command.args(["config", "get-contexts", "-o", "name"]);
    if let Some(path) = kubeconfig_path {
        command.env("KUBECONFIG", path);
    }
    let output = command
        .output()
        .map_err(|err| format!("Failed to run kubectl context discovery: {err}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "kubectl context discovery exited with a non-zero status".to_string()
        } else {
            detail
        });
    }
    Ok(parse_context_names(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn kubectl_default_namespace(
    kubeconfig_path: Option<&str>,
    context: &str,
) -> Result<String, String> {
    let mut command = Command::new("kubectl");
    command.args([
        "--context",
        context,
        "config",
        "view",
        "--minify",
        "-o",
        "jsonpath={..namespace}",
    ]);
    if let Some(path) = kubeconfig_path {
        command.env("KUBECONFIG", path);
    }
    let output = command
        .output()
        .map_err(|err| format!("Failed to read default namespace: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let namespace = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if namespace.is_empty() {
        "default".to_string()
    } else {
        namespace
    })
}

fn kubectl_context_reachable(kubeconfig_path: Option<&str>, context: &str) -> Result<(), String> {
    let mut command = Command::new("kubectl");
    command.args([
        "--context",
        context,
        "--request-timeout=2s",
        "get",
        "--raw=/version",
    ]);
    if let Some(path) = kubeconfig_path {
        command.env("KUBECONFIG", path);
    }
    let output = command
        .output()
        .map_err(|err| format!("Failed to check Kubernetes context reachability: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            "context is not reachable".to_string()
        } else {
            detail
        })
    }
}

pub(crate) fn parse_context_names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn cluster_already_configured(
    config: &LoopboxConfig,
    kubeconfig_path: Option<&str>,
    context: &str,
) -> bool {
    config.global.kubernetes.clusters.iter().any(|cluster| {
        cluster.context == context
            && cluster
                .kubeconfig_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                == kubeconfig_path
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
    })
}

fn unique_cluster_name(
    config: &LoopboxConfig,
    discovered: &[KubernetesClusterDiscovery],
    context: &str,
) -> String {
    let base = normalize_cluster_name(context);
    let used = config
        .global
        .kubernetes
        .clusters
        .iter()
        .map(|cluster| cluster.name.as_str())
        .chain(discovered.iter().map(|cluster| cluster.name.as_str()))
        .collect::<HashSet<_>>();
    if !used.contains(base.as_str()) {
        return base;
    }
    for index in 2..=999 {
        let candidate = format!("{base}-{index}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
    }
    format!("{base}-import")
}

fn normalize_cluster_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let cleaned = out.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "cluster".to_string()
    } else {
        cleaned
    }
}

fn find_cluster<'a>(
    config: &'a LoopboxConfig,
    cluster_name: &str,
) -> Result<&'a KubernetesClusterConfig, String> {
    config
        .global
        .kubernetes
        .clusters
        .iter()
        .find(|cluster| cluster.name == cluster_name)
        .ok_or_else(|| format!("Kubernetes cluster '{cluster_name}' not found."))
}

pub(crate) fn cluster_connectivity_state(
    cluster: &KubernetesClusterConfig,
) -> KubernetesConnectivityState {
    let Some(wireguard) = cluster.wireguard.as_ref() else {
        return KubernetesConnectivityState::NotConfigured;
    };
    let Some(interface) = wireguard_interface(wireguard) else {
        return KubernetesConnectivityState::Error(format!(
            "WireGuard tunnel '{}' has no interface or config_path.",
            wireguard.name
        ));
    };
    let output = Command::new("wg").arg("show").arg(&interface).output();
    match output {
        Ok(output) => match wireguard_active_from_show_output(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ) {
            Ok(true) => KubernetesConnectivityState::Active,
            Ok(false) => KubernetesConnectivityState::Inactive,
            Err(err) => KubernetesConnectivityState::Error(err),
        },
        Err(err) => KubernetesConnectivityState::Error(format!("Failed to run wg: {err}")),
    }
}

fn run_kubectl_json(
    cluster: &KubernetesClusterConfig,
    namespace: Option<&str>,
    trailing: &[&str],
) -> Result<String, String> {
    let invocation = kubectl_invocation(cluster, namespace, trailing);
    let mut command = Command::new(&invocation.program);
    command.args(&invocation.args);
    if let Some(kubeconfig) = invocation.kubeconfig_env.as_ref() {
        command.env("KUBECONFIG", kubeconfig);
    }
    let output = command
        .output()
        .map_err(|err| format!("Failed to run kubectl: {err}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            "kubectl exited with a non-zero status".to_string()
        } else {
            detail
        })
    }
}

fn run_wg_quick(wireguard: &WireGuardTunnelConfig, action: &str) -> Result<(), String> {
    if wireguard.mode != WireGuardMode::WgQuick {
        return Err(format!(
            "WireGuard tunnel '{}' is manual and cannot be controlled by Loopbox.",
            wireguard.name
        ));
    }
    let target = wireguard
        .config_path
        .as_deref()
        .or(wireguard.interface.as_deref())
        .ok_or_else(|| {
            format!(
                "WireGuard tunnel '{}' needs config_path or interface.",
                wireguard.name
            )
        })?;
    let output = Command::new("wg-quick")
        .arg(action)
        .arg(target)
        .output()
        .map_err(|err| format!("Failed to run wg-quick {action}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            format!("wg-quick {action} exited with a non-zero status")
        } else {
            detail
        })
    }
}

fn wireguard_interface(wireguard: &WireGuardTunnelConfig) -> Option<String> {
    wireguard.interface.clone().or_else(|| {
        wireguard
            .config_path
            .as_deref()
            .and_then(interface_from_config_path)
    })
}

fn interface_from_config_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn enrich_service_relationships(
    services: &mut [KubernetesServiceSnapshot],
    pods: &[KubernetesPodSnapshot],
    endpoint_slices: &[KubernetesEndpointSliceSnapshot],
    ingresses: &[KubernetesIngressSnapshot],
) {
    for service in services {
        let endpoint_pods = endpoint_slices
            .iter()
            .filter(|slice| {
                slice.namespace == service.namespace
                    && slice.service_name.as_deref() == Some(service.name.as_str())
            })
            .flat_map(|slice| slice.target_pods.iter().cloned())
            .collect::<Vec<_>>();
        let selector_pods = pods
            .iter()
            .filter(|pod| pod.namespace == service.namespace)
            .filter(|pod| selector_matches_labels(&service.selector, &pod.labels))
            .map(|pod| pod.name.clone())
            .collect::<Vec<_>>();
        let mut target_pods = endpoint_pods;
        target_pods.extend(selector_pods);
        target_pods.sort();
        target_pods.dedup();
        service.endpoint_count = endpoint_slices
            .iter()
            .filter(|slice| {
                slice.namespace == service.namespace
                    && slice.service_name.as_deref() == Some(service.name.as_str())
            })
            .map(|slice| slice.ready_endpoints)
            .sum::<u64>()
            .max(target_pods.len() as u64);
        service.target_pods = target_pods;
        service.ingress_routes = ingresses
            .iter()
            .filter(|ingress| {
                ingress.namespace == service.namespace
                    && ingress
                        .service_backends
                        .iter()
                        .any(|backend| backend == &service.name)
            })
            .flat_map(|ingress| {
                if ingress.hosts.is_empty() {
                    vec![ingress.name.clone()]
                } else {
                    ingress.hosts.clone()
                }
            })
            .collect();
        service.ingress_routes.sort();
        service.ingress_routes.dedup();
    }
}

fn selector_matches_labels(selector: &[String], labels: &[String]) -> bool {
    !selector.is_empty() && selector.iter().all(|item| labels.contains(item))
}
