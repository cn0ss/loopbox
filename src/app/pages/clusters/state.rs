use crate::loopbox::{
    KubernetesClusterDiscovery, KubernetesClusterImport, KubernetesClusterSnapshot,
    KubernetesConnectivityState, KubernetesProvider, LoopboxConfig,
};
use std::collections::HashSet;

pub(super) fn connectivity_label(state: &KubernetesConnectivityState) -> &'static str {
    match state {
        KubernetesConnectivityState::NotConfigured => "no tunnel",
        KubernetesConnectivityState::Active => "wireguard active",
        KubernetesConnectivityState::Inactive => "wireguard inactive",
        KubernetesConnectivityState::Error(_) => "wireguard error",
    }
}

pub(super) fn connectivity_class(state: &KubernetesConnectivityState) -> &'static str {
    match state {
        KubernetesConnectivityState::Active => "cluster-status cluster-status-ok",
        KubernetesConnectivityState::Inactive | KubernetesConnectivityState::Error(_) => {
            "cluster-status cluster-status-warn"
        }
        KubernetesConnectivityState::NotConfigured => "cluster-status",
    }
}

pub(super) fn provider_label(provider: KubernetesProvider) -> &'static str {
    match provider {
        KubernetesProvider::KubeconfigContext => "kubeconfig",
        KubernetesProvider::Local => "local",
        KubernetesProvider::Remote => "remote",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct ClusterEditDraft {
    pub name: String,
    pub provider: String,
    pub kubeconfig_path: String,
    pub context: String,
    pub default_namespace: String,
}

pub(super) fn cluster_edit_draft_from_config(
    config: &LoopboxConfig,
    snapshot: &KubernetesClusterSnapshot,
) -> ClusterEditDraft {
    if let Some(cluster) = config
        .global
        .kubernetes
        .clusters
        .iter()
        .find(|cluster| cluster.name == snapshot.name)
    {
        return ClusterEditDraft {
            name: cluster.name.clone(),
            provider: provider_value(cluster.provider).to_string(),
            kubeconfig_path: cluster.kubeconfig_path.clone().unwrap_or_default(),
            context: cluster.context.clone(),
            default_namespace: cluster.default_namespace.clone(),
        };
    }

    ClusterEditDraft {
        name: snapshot.name.clone(),
        provider: provider_value(snapshot.provider).to_string(),
        kubeconfig_path: String::new(),
        context: snapshot.context.clone(),
        default_namespace: snapshot.default_namespace.clone(),
    }
}

fn provider_value(provider: KubernetesProvider) -> &'static str {
    match provider {
        KubernetesProvider::KubeconfigContext => "kubeconfig_context",
        KubernetesProvider::Local => "local",
        KubernetesProvider::Remote => "remote",
    }
}

fn provider_from_value(value: &str) -> KubernetesProvider {
    match value.trim() {
        "local" => KubernetesProvider::Local,
        "remote" => KubernetesProvider::Remote,
        _ => KubernetesProvider::KubeconfigContext,
    }
}

fn discovery_to_import(discovery: &KubernetesClusterDiscovery) -> KubernetesClusterImport {
    KubernetesClusterImport {
        name: Some(discovery.name.clone()),
        provider: discovery.provider,
        kubeconfig_path: discovery.kubeconfig_path.clone(),
        context: discovery.context.clone(),
        default_namespace: Some(discovery.default_namespace.clone()),
    }
}

pub(super) fn remove_configured_cluster(config: &mut LoopboxConfig, cluster_name: &str) -> bool {
    let before = config.global.kubernetes.clusters.len();
    config
        .global
        .kubernetes
        .clusters
        .retain(|cluster| cluster.name != cluster_name);
    config.global.kubernetes.clusters.len() != before
}

pub(super) fn update_configured_cluster(
    config: &mut LoopboxConfig,
    original_name: &str,
    draft: ClusterEditDraft,
) -> Result<String, String> {
    let name = draft.name.trim();
    let context = draft.context.trim();
    if name.is_empty() {
        return Err("Cluster name is required.".to_string());
    }
    if context.is_empty() {
        return Err("Cluster context is required.".to_string());
    }
    if config
        .global
        .kubernetes
        .clusters
        .iter()
        .any(|cluster| cluster.name == name && cluster.name != original_name)
    {
        return Err(format!("Kubernetes cluster '{name}' already exists."));
    }
    let cluster = config
        .global
        .kubernetes
        .clusters
        .iter_mut()
        .find(|cluster| cluster.name == original_name)
        .ok_or_else(|| format!("Kubernetes cluster '{original_name}' not found."))?;

    cluster.name = name.to_string();
    cluster.provider = provider_from_value(&draft.provider);
    let kubeconfig_path = draft.kubeconfig_path.trim();
    cluster.kubeconfig_path = if kubeconfig_path.is_empty() {
        None
    } else {
        Some(kubeconfig_path.to_string())
    };
    cluster.context = context.to_string();
    let default_namespace = draft.default_namespace.trim();
    cluster.default_namespace = if default_namespace.is_empty() {
        "default".to_string()
    } else {
        default_namespace.to_string()
    };

    Ok(cluster.name.clone())
}

pub(super) fn selected_discoveries_to_import(
    discoveries: &[KubernetesClusterDiscovery],
    selected: &HashSet<String>,
) -> Vec<KubernetesClusterImport> {
    discoveries
        .iter()
        .filter(|discovery| !discovery.already_configured)
        .filter(|discovery| selected.contains(&discovery_key(discovery)))
        .map(discovery_to_import)
        .collect()
}

pub(super) fn selectable_discovery_keys(
    discoveries: &[KubernetesClusterDiscovery],
) -> HashSet<String> {
    discoveries
        .iter()
        .filter(|discovery| !discovery.already_configured)
        .map(discovery_key)
        .collect()
}

pub(super) fn discovery_key(discovery: &KubernetesClusterDiscovery) -> String {
    format!(
        "context:{}|{}",
        discovery.kubeconfig_path.clone().unwrap_or_default(),
        discovery.context
    )
}

pub(super) fn compact_kubectl_error(error: &str) -> String {
    let compact = error
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("memcache.go:"))
        .collect::<Vec<_>>()
        .join(" ");
    let compact = if compact.is_empty() {
        error.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        compact
    };
    if compact.chars().count() <= 260 {
        return compact;
    }
    let mut truncated = compact.chars().take(257).collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn configured_cluster_fallback_snapshots(
    config: &LoopboxConfig,
) -> Vec<KubernetesClusterSnapshot> {
    config
        .global
        .kubernetes
        .clusters
        .iter()
        .map(|cluster| KubernetesClusterSnapshot {
            name: cluster.name.clone(),
            provider: cluster.provider,
            context: cluster.context.clone(),
            default_namespace: cluster.default_namespace.clone(),
            selected_namespace: cluster.default_namespace.clone(),
            connectivity: KubernetesConnectivityState::NotConfigured,
            namespaces: Vec::new(),
            workloads: Vec::new(),
            services: Vec::new(),
            nodes: Vec::new(),
            pods: Vec::new(),
            ingresses: Vec::new(),
            endpoint_slices: Vec::new(),
            events: Vec::new(),
            topology: Default::default(),
            warnings: Vec::new(),
            last_error: None,
        })
        .collect()
}

pub(super) fn selected_cluster_snapshot(
    clusters: &[KubernetesClusterSnapshot],
    selected: Option<String>,
) -> Option<KubernetesClusterSnapshot> {
    let selected = selected?;
    clusters
        .iter()
        .find(|cluster| cluster.name == selected)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::KubernetesClusterConfig;

    fn discovery(
        name: &str,
        context: &str,
        kubeconfig_path: Option<&str>,
        already_configured: bool,
    ) -> KubernetesClusterDiscovery {
        KubernetesClusterDiscovery {
            name: name.to_string(),
            provider: KubernetesProvider::KubeconfigContext,
            kubeconfig_path: kubeconfig_path.map(str::to_string),
            context: context.to_string(),
            default_namespace: "default".to_string(),
            already_configured,
            reachable: true,
            error: None,
        }
    }

    #[test]
    fn selected_discoveries_to_import_imports_only_selected_new_contexts() {
        let discoveries = vec![
            discovery("docker-desktop", "docker-desktop", Some("/tmp/kube"), true),
            discovery("kind-loopbox", "kind-loopbox", Some("/tmp/kube"), false),
            discovery("orbstack", "orbstack", Some("/tmp/kube"), false),
        ];
        let selected = HashSet::from([
            "context:/tmp/kube|docker-desktop".to_string(),
            "context:/tmp/kube|orbstack".to_string(),
        ]);

        let imports = selected_discoveries_to_import(&discoveries, &selected);

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].name.as_deref(), Some("orbstack"));
        assert_eq!(imports[0].context, "orbstack");
    }

    #[test]
    fn configured_cluster_fallback_snapshots_keep_configured_clusters_visible() {
        let mut config = LoopboxConfig::default();
        config.global.kubernetes.clusters = vec![
            cluster_config("docker-desktop", "docker-desktop"),
            cluster_config("orbstack", "orbstack"),
        ];

        let snapshots = configured_cluster_fallback_snapshots(&config);

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].name, "docker-desktop");
        assert_eq!(snapshots[1].name, "orbstack");
        assert!(snapshots
            .iter()
            .all(|snapshot| snapshot.namespaces.is_empty()));
    }

    #[test]
    fn remove_configured_cluster_removes_only_named_cluster() {
        let mut config = LoopboxConfig::default();
        config.global.kubernetes.clusters = vec![
            cluster_config("docker-desktop", "docker-desktop"),
            cluster_config("orbstack", "orbstack"),
        ];

        assert!(remove_configured_cluster(&mut config, "docker-desktop"));

        assert_eq!(config.global.kubernetes.clusters.len(), 1);
        assert_eq!(config.global.kubernetes.clusters[0].name, "orbstack");
    }

    #[test]
    fn update_configured_cluster_updates_fields_and_rejects_duplicate_names() {
        let mut config = LoopboxConfig::default();
        config.global.kubernetes.clusters = vec![
            cluster_config("docker-desktop", "docker-desktop"),
            cluster_config("orbstack", "orbstack"),
        ];

        let duplicate = update_configured_cluster(
            &mut config,
            "docker-desktop",
            ClusterEditDraft {
                name: "orbstack".to_string(),
                provider: "remote".to_string(),
                kubeconfig_path: "/tmp/kube".to_string(),
                context: "prod".to_string(),
                default_namespace: "apps".to_string(),
            },
        );
        assert!(duplicate.is_err());

        update_configured_cluster(
            &mut config,
            "docker-desktop",
            ClusterEditDraft {
                name: "docker-prod".to_string(),
                provider: "remote".to_string(),
                kubeconfig_path: " /tmp/kube ".to_string(),
                context: " prod ".to_string(),
                default_namespace: " apps ".to_string(),
            },
        )
        .expect("cluster update should succeed");

        let cluster = &config.global.kubernetes.clusters[0];
        assert_eq!(cluster.name, "docker-prod");
        assert_eq!(cluster.provider, KubernetesProvider::Remote);
        assert_eq!(cluster.kubeconfig_path.as_deref(), Some("/tmp/kube"));
        assert_eq!(cluster.context, "prod");
        assert_eq!(cluster.default_namespace, "apps");
    }

    #[test]
    fn selected_cluster_snapshot_returns_only_selected_cluster() {
        let snapshots = vec![snapshot("docker-desktop"), snapshot("orbstack")];

        let selected = selected_cluster_snapshot(&snapshots, Some("orbstack".to_string()));

        assert_eq!(
            selected.map(|cluster| cluster.name),
            Some("orbstack".to_string())
        );
    }

    fn cluster_config(name: &str, context: &str) -> KubernetesClusterConfig {
        KubernetesClusterConfig {
            name: name.to_string(),
            provider: KubernetesProvider::KubeconfigContext,
            kubeconfig_path: Some("/tmp/kube".to_string()),
            context: context.to_string(),
            default_namespace: "default".to_string(),
            wireguard: None,
        }
    }

    fn snapshot(name: &str) -> KubernetesClusterSnapshot {
        KubernetesClusterSnapshot {
            name: name.to_string(),
            provider: KubernetesProvider::KubeconfigContext,
            context: name.to_string(),
            default_namespace: "default".to_string(),
            selected_namespace: "default".to_string(),
            connectivity: KubernetesConnectivityState::NotConfigured,
            namespaces: Vec::new(),
            workloads: Vec::new(),
            services: Vec::new(),
            nodes: Vec::new(),
            pods: Vec::new(),
            ingresses: Vec::new(),
            endpoint_slices: Vec::new(),
            events: Vec::new(),
            topology: Default::default(),
            warnings: Vec::new(),
            last_error: None,
        }
    }
}
