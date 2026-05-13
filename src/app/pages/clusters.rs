use crate::app::models::{Notice, Page};
use crate::app::utils::copy_to_clipboard;
use crate::loopbox::{
    self, KubernetesClusterDiscovery, KubernetesClusterSnapshot, KubernetesConnectivityState,
    KubernetesEventSnapshot, KubernetesPodSnapshot, KubernetesServiceSnapshot,
    KubernetesWorkloadSnapshot, LoopboxConfig,
};
use dioxus::prelude::*;
use std::collections::HashSet;

mod diagnostics;
mod state;
use diagnostics::ClusterDiagnostics;
use state::{
    cluster_edit_draft_from_config, compact_kubectl_error, configured_cluster_fallback_snapshots,
    connectivity_class, connectivity_label, discovery_key, provider_label,
    remove_configured_cluster, selectable_discovery_keys, selected_cluster_snapshot,
    selected_discoveries_to_import, update_configured_cluster, ClusterEditDraft,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterTab {
    Overview,
    Topology,
    Workloads,
    Services,
    Events,
    Config,
}

impl ClusterTab {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Topology => "Topology",
            Self::Workloads => "Workloads",
            Self::Services => "Services",
            Self::Events => "Events",
            Self::Config => "Config",
        }
    }
}

pub(in crate::app) fn render_clusters_page(
    page: Page,
    current_page: Signal<Page>,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
) -> Element {
    let mut discovery_tick = use_signal(|| 0_u64);
    let mut selected_discoveries = use_signal(HashSet::<String>::new);
    let mut selected_cluster = use_signal(|| None::<String>);
    let selected_namespace = use_signal(|| None::<String>);
    let active_tab = use_signal(|| ClusterTab::Overview);
    let cluster_search = use_signal(String::new);
    let edit_draft = use_signal(ClusterEditDraft::default);
    let discovery_resource = use_resource(move || {
        let cfg = config();
        let tick = discovery_tick();
        let active_page = current_page();
        async move {
            if active_page != Page::Clusters || tick == 0 {
                return None;
            }
            Some(
                tokio::task::spawn_blocking(move || loopbox::discover_kubernetes_clusters(&cfg))
                    .await
                    .map_err(|err| format!("Cluster discovery task failed: {err}"))
                    .and_then(|result| result),
            )
        }
    });
    let clusters_resource = use_resource(move || {
        let cfg = config();
        let active_page = current_page();
        let refresh = runtime_tick();
        async move {
            let _ = refresh;
            if active_page != Page::Clusters {
                return Ok(Vec::new());
            }
            tokio::task::spawn_blocking(move || loopbox::cluster_summaries(&cfg))
                .await
                .map_err(|err| format!("Cluster snapshot task failed: {err}"))
                .and_then(|result| result)
        }
    });
    let detail_resource = use_resource(move || {
        let cfg = config();
        let active_page = current_page();
        let refresh = runtime_tick();
        let selected = selected_cluster();
        let namespace = selected_namespace();
        async move {
            let _ = refresh;
            if active_page != Page::Clusters {
                return Ok(None);
            }
            let Some(cluster_name) = selected else {
                return Ok(None);
            };
            tokio::task::spawn_blocking(move || {
                loopbox::cluster_snapshot_for_namespace(&cfg, &cluster_name, namespace.as_deref())
            })
            .await
            .map_err(|err| format!("Cluster detail task failed: {err}"))
            .and_then(|result| result)
            .map(Some)
        }
    });

    if page != Page::Clusters {
        return rsx! {};
    }

    let configured_count = config.read().global.kubernetes.clusters.len();
    let discovery_result = discovery_resource().flatten();
    let discovered_clusters = discovery_result
        .as_ref()
        .and_then(|result| result.as_ref().ok().cloned())
        .unwrap_or_default();
    let discovery_error = discovery_result
        .as_ref()
        .and_then(|result| result.as_ref().err().cloned());
    let discovered_clusters_for_import = discovered_clusters.clone();
    let loading = clusters_resource().is_none();
    let clusters_result = clusters_resource();
    let snapshot_error = clusters_result
        .as_ref()
        .and_then(|result| result.as_ref().err().cloned());
    let detail_result = detail_resource();
    let detail_error = detail_result
        .as_ref()
        .and_then(|result| result.as_ref().err().cloned());
    let mut clusters = clusters_result.and_then(Result::ok).unwrap_or_default();
    if clusters.is_empty() && configured_count > 0 {
        clusters = configured_cluster_fallback_snapshots(&config.read());
    }
    let active_count = clusters
        .iter()
        .filter(|cluster| cluster.connectivity == KubernetesConnectivityState::Active)
        .count();
    let error_count = clusters
        .iter()
        .filter(|cluster| {
            cluster.last_error.is_some()
                || matches!(cluster.connectivity, KubernetesConnectivityState::Error(_))
        })
        .count();
    let selected_detail_cluster = detail_result
        .and_then(Result::ok)
        .flatten()
        .or_else(|| selected_cluster_snapshot(&clusters, selected_cluster()));

    rsx! {
        div { class: "page clusters-page",
            div { class: "page-header",
                div { class: "page-header-left",
                    div { class: "page-header-stack",
                        span { class: "page-eyebrow", "Kubernetes" }
                        h1 { class: "page-title", "clusters" }
                        p { class: "page-subtitle",
                            if let Some(cluster) = selected_detail_cluster.as_ref() {
                                "Editing {cluster.name}"
                            } else if configured_count == 0 {
                                "No Kubernetes clusters configured."
                            } else if loading {
                                "Loading configured clusters..."
                            } else {
                                "{configured_count} configured · {active_count} tunnel(s) active · {error_count} issue(s)"
                            }
                        }
                    }
                }
                div { class: "page-actions",
                    if selected_detail_cluster.is_some() {
                        button {
                            class: "btn btn-outline",
                            onclick: move |_| selected_cluster.set(None),
                            "Back to clusters"
                        }
                    } else {
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                selected_discoveries.write().clear();
                                discovery_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                            },
                            "Detect clusters"
                        }
                    }
                    button {
                        class: "btn btn-outline",
                        onclick: move |_| runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1)),
                        "Refresh data"
                    }
                    if selected_detail_cluster.is_none() && !discovered_clusters.is_empty() {
                        button {
                            class: "btn btn-outline",
                            onclick: move |_| {
                                let selected = selected_discoveries.read().clone();
                                let imports = selected_discoveries_to_import(
                                    &discovered_clusters_for_import,
                                    &selected,
                                );
                                if imports.is_empty() {
                                    notice.set(Some(Notice::info("Select at least one new Kubernetes context to import.")));
                                    return;
                                }
                                let mut cfg = config();
                                match loopbox::import_kubernetes_clusters(&mut cfg, &imports)
                                    .and_then(|imported| loopbox::save_config(&cfg).map(|_| imported))
                                {
                                    Ok(imported) => {
                                        config.set(cfg);
                                        notice.set(Some(Notice::success(format!("Imported {imported} Kubernetes cluster(s)."))));
                                        runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                        discovery_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            "Import selected"
                        }
                    }
                }
            }

            if let Some(error) = snapshot_error {
                div { class: "clusters-alert", "{error}" }
            }
            if let Some(error) = discovery_error {
                div { class: "clusters-alert", "{error}" }
            }
            if let Some(error) = detail_error {
                div { class: "clusters-alert", "{error}" }
            }

            if let Some(cluster) = selected_detail_cluster {
                ClusterWorkbench {
                    cluster,
                    config,
                    notice,
                    runtime_tick,
                    selected_cluster,
                    selected_namespace,
                    active_tab,
                    cluster_search,
                    edit_draft,
                }
            } else {
                if !discovered_clusters.is_empty() {
                    DiscoveryPanel { discoveries: discovered_clusters, selected_discoveries }
                }

                if configured_count == 0 {
                    div { class: "empty-state",
                        span { class: "empty-state-icon", "◊" }
                        h2 { class: "empty-state-title", "no clusters yet" }
                        p { class: "empty-state-desc",
                            "Detect kubeconfig contexts to import local or remote clusters into Loopbox."
                        }
                    }
                } else {
                    div { class: "clusters-grid",
                        for cluster in clusters {
                            ClusterCard {
                                key: "{cluster.name}",
                                cluster,
                                config,
                                notice,
                                runtime_tick,
                                selected_cluster,
                                selected_namespace,
                                active_tab,
                                edit_draft,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DiscoveryPanel(
    discoveries: Vec<KubernetesClusterDiscovery>,
    mut selected_discoveries: Signal<HashSet<String>>,
) -> Element {
    let new_count = discoveries
        .iter()
        .filter(|cluster| !cluster.already_configured)
        .count();
    let selected_count = selected_discoveries.read().len();
    let discoveries_for_select_all = discoveries.clone();
    let discoveries_for_clear = discoveries.clone();

    rsx! {
        section { class: "cluster-card",
            div { class: "cluster-card-head",
                div {
                    h2 { "Detected contexts" }
                    p { "{selected_count} selected · {new_count} new · {discoveries.len()} total" }
                }
                div { class: "cluster-actions cluster-actions-inline",
                    button {
                        class: "cluster-action cluster-action-muted",
                        onclick: move |_| {
                            selected_discoveries.set(selectable_discovery_keys(&discoveries_for_select_all));
                        },
                        "Select new"
                    }
                    button {
                        class: "cluster-action cluster-action-muted",
                        onclick: move |_| {
                            let current_keys = discoveries_for_clear
                                .iter()
                                .map(discovery_key)
                                .collect::<HashSet<_>>();
                            selected_discoveries.write().retain(|key| !current_keys.contains(key));
                        },
                        "Clear"
                    }
                }
            }
            div { class: "cluster-table cluster-discovery-table",
                for discovery in discoveries {
                    DiscoveryRow {
                        key: "{discovery_key(&discovery)}",
                        discovery,
                        selected_discoveries,
                    }
                }
            }
        }
    }
}

#[component]
fn DiscoveryRow(
    discovery: KubernetesClusterDiscovery,
    mut selected_discoveries: Signal<HashSet<String>>,
) -> Element {
    let status = if discovery.already_configured {
        "configured"
    } else if discovery.reachable {
        "reachable"
    } else {
        "not reachable"
    };
    let path = discovery
        .kubeconfig_path
        .clone()
        .unwrap_or_else(|| "default kubeconfig".to_string());
    let error = discovery.error.as_deref().map(compact_kubectl_error);
    let key = discovery_key(&discovery);
    let checked = selected_discoveries.read().contains(&key);
    let disabled = discovery.already_configured;
    let key_for_change = key.clone();

    rsx! {
        div { class: "cluster-table-row cluster-discovery-row",
            label { class: "cluster-select",
                input {
                    r#type: "checkbox",
                    checked,
                    disabled,
                    onchange: move |event| {
                        selected_discoveries.with_mut(|selected| {
                            if event.checked() {
                                selected.insert(key_for_change.clone());
                            } else {
                                selected.remove(&key_for_change);
                            }
                        });
                    },
                }
            }
            span { class: "cluster-row-main", "{discovery.name}" }
            span { "{discovery.context}" }
            span { "{discovery.default_namespace}" }
            span { "{path}" }
            span { "{status}" }
            if let Some(error) = error {
                span { "{error}" }
            }
        }
    }
}

#[component]
fn ClusterCard(
    cluster: KubernetesClusterSnapshot,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
    mut selected_cluster: Signal<Option<String>>,
    mut selected_namespace: Signal<Option<String>>,
    mut active_tab: Signal<ClusterTab>,
    mut edit_draft: Signal<ClusterEditDraft>,
) -> Element {
    let cluster_name_for_start = cluster.name.clone();
    let cluster_name_for_stop = cluster.name.clone();
    let cluster_for_detail = cluster.clone();
    let cluster_name_for_delete = cluster.name.clone();
    let has_wireguard = !matches!(
        cluster.connectivity,
        KubernetesConnectivityState::NotConfigured
    );
    let connectivity_label = connectivity_label(&cluster.connectivity);
    let connectivity_class = connectivity_class(&cluster.connectivity);
    let last_error = cluster.last_error.as_deref().map(compact_kubectl_error);

    rsx! {
        section { class: "cluster-card",
            div { class: "cluster-card-head",
                div {
                    h2 { "{cluster.name}" }
                    p { "{provider_label(cluster.provider)} · {cluster.context} · {cluster.default_namespace}" }
                }
                span { class: "{connectivity_class}", "{connectivity_label}" }
            }

            if let Some(error) = last_error {
                div { class: "clusters-alert clusters-alert-inline", "{error}" }
            }

            div { class: "cluster-actions",
                button {
                    class: "cluster-action",
                    onclick: move |_| {
                        let cfg = config();
                        edit_draft.set(cluster_edit_draft_from_config(&cfg, &cluster_for_detail));
                        selected_namespace.set(Some(cluster_for_detail.selected_namespace.clone()));
                        active_tab.set(ClusterTab::Overview);
                        selected_cluster.set(Some(cluster_for_detail.name.clone()));
                    },
                    "Details"
                }
                button {
                    class: "cluster-action cluster-action-muted",
                    onclick: move |_| runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1)),
                    "Refresh"
                }
                button {
                    class: "cluster-action cluster-action-danger",
                    onclick: move |_| {
                        let mut cfg = config();
                        if remove_configured_cluster(&mut cfg, &cluster_name_for_delete) {
                            match loopbox::save_config(&cfg) {
                                Ok(_) => {
                                    config.set(cfg);
                                    selected_cluster.set(None);
                                    notice.set(Some(Notice::success(format!("Deleted Kubernetes cluster '{cluster_name_for_delete}'."))));
                                    runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                }
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    "Delete"
                }
            }

            if has_wireguard {
                div { class: "cluster-actions",
                    button {
                        class: "cluster-action",
                        onclick: move |_| {
                            let cfg = config();
                            match loopbox::start_cluster_wireguard(&cfg, &cluster_name_for_start) {
                                Ok(()) => {
                                    notice.set(Some(Notice::success("WireGuard start requested.")));
                                    runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                }
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Start WireGuard"
                    }
                    button {
                        class: "cluster-action cluster-action-muted",
                        onclick: move |_| {
                            let cfg = config();
                            match loopbox::stop_cluster_wireguard(&cfg, &cluster_name_for_stop) {
                                Ok(()) => {
                                    notice.set(Some(Notice::success("WireGuard stop requested.")));
                                    runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                }
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Stop"
                    }
                }
            }

            ClusterDiagnostics { cluster: cluster.clone(), notice }

            div { class: "cluster-section",
                h3 { "Namespaces" }
                if cluster.namespaces.is_empty() {
                    p { class: "cluster-muted", "No namespace data available." }
                } else {
                    div { class: "cluster-chip-row",
                        for namespace in cluster.namespaces {
                            span { key: "{namespace.name}", class: "cluster-chip", "{namespace.name}" }
                        }
                    }
                }
            }

            div { class: "cluster-section",
                h3 { "Workloads" }
                if cluster.workloads.is_empty() {
                    p { class: "cluster-muted", "No workloads found in the default namespace." }
                } else {
                    div { class: "cluster-table",
                        for workload in cluster.workloads {
                            WorkloadRow { key: "{workload.kind}-{workload.namespace}-{workload.name}", workload }
                        }
                    }
                }
            }

            div { class: "cluster-section",
                h3 { "Services" }
                if cluster.services.is_empty() {
                    p { class: "cluster-muted", "No services found in the default namespace." }
                } else {
                    div { class: "cluster-table",
                        for service in cluster.services {
                            ServiceRow { key: "{service.namespace}-{service.name}", service }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ClusterWorkbench(
    cluster: KubernetesClusterSnapshot,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
    mut selected_cluster: Signal<Option<String>>,
    mut selected_namespace: Signal<Option<String>>,
    mut active_tab: Signal<ClusterTab>,
    mut cluster_search: Signal<String>,
    mut edit_draft: Signal<ClusterEditDraft>,
) -> Element {
    let original_name = cluster.name.clone();
    let save_original_name = original_name.clone();
    let close_name = original_name.clone();
    let delete_name = original_name.clone();
    let draft = edit_draft();
    let tab = active_tab();
    let search = cluster_search();
    let filtered_workloads = filtered_workloads(&cluster, &search);
    let filtered_pods = filtered_pods(&cluster, &search);
    let filtered_services = filtered_services(&cluster, &search);
    let filtered_events = filtered_events(&cluster, &search);
    let selected_namespace_value = cluster.selected_namespace.clone();
    let has_wireguard = !matches!(
        cluster.connectivity,
        KubernetesConnectivityState::NotConfigured
    );
    let cluster_for_summary = cluster.clone();

    rsx! {
        section { class: "cluster-workbench",
            div { class: "cluster-card-head",
                div {
                    h2 { "{cluster.name}" }
                    p { "{provider_label(cluster.provider)} · {cluster.context} · namespace {cluster.selected_namespace}" }
                }
                div { class: "cluster-actions cluster-actions-inline",
                    button {
                        class: "cluster-action cluster-action-muted",
                        onclick: move |_| runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1)),
                        "Refresh"
                    }
                    button {
                        class: "cluster-action cluster-action-muted",
                        onclick: move |_| {
                            if selected_cluster() == Some(close_name.clone()) {
                                selected_cluster.set(None);
                            }
                        },
                        "Close"
                    }
                }
            }

            div { class: "cluster-workbench-toolbar",
                div { class: "cluster-chip-row",
                    if cluster.namespaces.is_empty() {
                        span { class: "cluster-chip", "{cluster.selected_namespace}" }
                    } else {
                        for namespace in &cluster.namespaces {
                            button {
                                key: "{namespace.name}",
                                class: if namespace.name == selected_namespace_value {
                                    "cluster-chip cluster-chip-active"
                                } else {
                                    "cluster-chip cluster-chip-button"
                                },
                                onclick: {
                                    let namespace_name = namespace.name.clone();
                                    move |_| {
                                        selected_namespace.set(Some(namespace_name.clone()));
                                        runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                    }
                                },
                                "{namespace.name}"
                            }
                        }
                    }
                }
                label { class: "cluster-search-shell",
                    span { "Search" }
                    input {
                        value: "{search}",
                        placeholder: "workloads, pods, services, events",
                        oninput: move |event| cluster_search.set(event.value()),
                    }
                }
            }

            div { class: "cluster-tab-bar",
                for tab_item in [
                    ClusterTab::Overview,
                    ClusterTab::Topology,
                    ClusterTab::Workloads,
                    ClusterTab::Services,
                    ClusterTab::Events,
                    ClusterTab::Config,
                ] {
                    button {
                        key: "{tab_item.label()}",
                        class: if tab_item == tab { "cluster-tab cluster-tab-active" } else { "cluster-tab" },
                        onclick: move |_| active_tab.set(tab_item),
                        "{tab_item.label()}"
                    }
                }
            }

            if tab == ClusterTab::Overview {
                div { class: "cluster-overview-grid",
                    OverviewStat { label: "Nodes".to_string(), value: format!("{}/{} ready", cluster.nodes.iter().filter(|node| node.ready).count(), cluster.nodes.len()) }
                    OverviewStat { label: "Workloads".to_string(), value: format!("{}/{} healthy", healthy_workload_count(&cluster), cluster.workloads.len()) }
                    OverviewStat { label: "Pods".to_string(), value: format!("{}/{} ready", cluster_ready_pod_count(&cluster), cluster.pods.len()) }
                    OverviewStat { label: "Services".to_string(), value: format!("{}/{} with endpoints", cluster.services.iter().filter(|service| service.endpoint_count > 0).count(), cluster.services.len()) }
                    OverviewStat { label: "Warnings".to_string(), value: format!("{}", cluster_warning_event_count(&cluster)) }
                }
                ClusterDiagnostics { cluster: cluster_for_summary, notice }
            } else if tab == ClusterTab::Topology {
                ClusterTopologyTab { cluster: cluster.clone(), notice }
            } else if tab == ClusterTab::Workloads {
                div { class: "cluster-section",
                    h3 { "Workloads" }
                    if filtered_workloads.is_empty() && filtered_pods.is_empty() {
                        p { class: "cluster-muted", "No workloads or pods match this filter." }
                    } else {
                        div { class: "cluster-table cluster-table-wide",
                            for workload in filtered_workloads {
                                WorkloadRow { key: "workload-{workload.kind}-{workload.namespace}-{workload.name}", workload }
                            }
                            for pod in filtered_pods {
                                PodRow {
                                    key: "pod-{pod.namespace}-{pod.name}",
                                    pod,
                                    cluster: cluster.clone(),
                                    notice,
                                }
                            }
                        }
                    }
                }
            } else if tab == ClusterTab::Services {
                div { class: "cluster-section",
                    h3 { "Services and traffic flows" }
                    if filtered_services.is_empty() {
                        p { class: "cluster-muted", "No services match this filter." }
                    } else {
                        div { class: "cluster-table cluster-table-wide",
                            for service in filtered_services {
                                ServiceDebugRow {
                                    key: "service-{service.namespace}-{service.name}",
                                    service,
                                    cluster: cluster.clone(),
                                    notice,
                                }
                            }
                        }
                    }
                }
            } else if tab == ClusterTab::Events {
                div { class: "cluster-section",
                    h3 { "Events" }
                    if filtered_events.is_empty() {
                        p { class: "cluster-muted", "No events match this filter." }
                    } else {
                        div { class: "cluster-event-list cluster-event-list-full",
                            for event in filtered_events {
                                EventRow {
                                    key: "event-{event.namespace}-{event.involved_kind}-{event.involved_name}-{event.reason}",
                                    event,
                                    cluster: cluster.clone(),
                                    notice,
                                }
                            }
                        }
                    }
                }
            } else if tab == ClusterTab::Config {
                div { class: "cluster-detail-grid",
                    label { class: "cluster-field",
                        span { "Name" }
                        input {
                            value: "{draft.name}",
                            oninput: move |event| {
                                edit_draft.with_mut(|draft| draft.name = event.value());
                            },
                        }
                    }
                    label { class: "cluster-field",
                        span { "Provider" }
                        select {
                            value: "{draft.provider}",
                            onchange: move |event| {
                                edit_draft.with_mut(|draft| draft.provider = event.value());
                            },
                            option { value: "kubeconfig_context", "kubeconfig" }
                            option { value: "local", "local" }
                            option { value: "remote", "remote" }
                        }
                    }
                    label { class: "cluster-field",
                        span { "Context" }
                        input {
                            value: "{draft.context}",
                            oninput: move |event| {
                                edit_draft.with_mut(|draft| draft.context = event.value());
                            },
                        }
                    }
                    label { class: "cluster-field",
                        span { "Namespace" }
                        input {
                            value: "{draft.default_namespace}",
                            oninput: move |event| {
                                edit_draft.with_mut(|draft| draft.default_namespace = event.value());
                            },
                        }
                    }
                    label { class: "cluster-field cluster-field-wide",
                        span { "Kubeconfig" }
                        input {
                            value: "{draft.kubeconfig_path}",
                            oninput: move |event| {
                                edit_draft.with_mut(|draft| draft.kubeconfig_path = event.value());
                            },
                        }
                    }
                }

                div { class: "cluster-actions",
                    button {
                        class: "cluster-action",
                        onclick: move |_| {
                            let mut cfg = config();
                            let draft = edit_draft();
                            match update_configured_cluster(&mut cfg, &save_original_name, draft.clone())
                                .and_then(|updated_name| loopbox::save_config(&cfg).map(|_| updated_name))
                            {
                                Ok(updated_name) => {
                                    config.set(cfg);
                                    selected_cluster.set(Some(updated_name.clone()));
                                    selected_namespace.set(Some(draft.default_namespace.clone()));
                                    notice.set(Some(Notice::success(format!("Updated Kubernetes cluster '{updated_name}'."))));
                                    runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                }
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Save changes"
                    }
                    if has_wireguard {
                        button {
                            class: "cluster-action cluster-action-muted",
                            onclick: {
                                let cluster_name = cluster.name.clone();
                                move |_| {
                                    let cfg = config();
                                    match loopbox::start_cluster_wireguard(&cfg, &cluster_name) {
                                        Ok(()) => {
                                            notice.set(Some(Notice::success("WireGuard start requested.")));
                                            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                }
                            },
                            "Start WireGuard"
                        }
                        button {
                            class: "cluster-action cluster-action-muted",
                            onclick: {
                                let cluster_name = cluster.name.clone();
                                move |_| {
                                    let cfg = config();
                                    match loopbox::stop_cluster_wireguard(&cfg, &cluster_name) {
                                        Ok(()) => {
                                            notice.set(Some(Notice::success("WireGuard stop requested.")));
                                            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                }
                            },
                            "Stop WireGuard"
                        }
                    }
                    button {
                        class: "cluster-action cluster-action-danger",
                        onclick: move |_| {
                            let mut cfg = config();
                            if remove_configured_cluster(&mut cfg, &delete_name) {
                                match loopbox::save_config(&cfg) {
                                    Ok(_) => {
                                        config.set(cfg);
                                        selected_cluster.set(None);
                                        notice.set(Some(Notice::success(format!("Deleted Kubernetes cluster '{delete_name}'."))));
                                        runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        },
                        "Delete cluster"
                    }
                }
                ClusterDiagnostics { cluster: cluster.clone(), notice }
            }
        }
    }
}

#[component]
fn OverviewStat(label: String, value: String) -> Element {
    rsx! {
        div { class: "cluster-overview-stat",
            span { "{label}" }
            strong { "{value}" }
        }
    }
}

#[component]
fn PodRow(
    pod: KubernetesPodSnapshot,
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let owner = pod
        .owner_name
        .clone()
        .unwrap_or_else(|| "standalone".to_string());
    let node = pod
        .node_name
        .clone()
        .unwrap_or_else(|| "unscheduled".to_string());
    let images = if pod.images.is_empty() {
        "no images".to_string()
    } else {
        pod.images.join(", ")
    };
    let get_command = kubectl_get_command(&cluster, &pod.namespace, "pod", &pod.name);
    let logs_command = kubectl_logs_command(&cluster, &pod.namespace, &pod.name);

    rsx! {
        div { class: "cluster-table-row cluster-pod-table-row",
            span { class: "cluster-row-main", "{pod.name}" }
            span { "{pod.phase}" }
            span { "{pod.ready_containers}/{pod.total_containers} ready" }
            span { "{pod.restart_count} restarts" }
            span { "{node}" }
            span { "{owner}" }
            span { "{images}" }
            span { class: "cluster-row-actions",
                button {
                    class: "cluster-action cluster-action-mini",
                    onclick: move |_| match copy_to_clipboard(&get_command) {
                        Ok(()) => notice.set(Some(Notice::success("Copied pod inspect command."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "YAML"
                }
                button {
                    class: "cluster-action cluster-action-mini cluster-action-muted",
                    onclick: move |_| match copy_to_clipboard(&logs_command) {
                        Ok(()) => notice.set(Some(Notice::success("Copied pod logs command."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Logs"
                }
            }
        }
    }
}

#[component]
fn ServiceDebugRow(
    service: KubernetesServiceSnapshot,
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let ports = if service.ports.is_empty() {
        "no ports".to_string()
    } else {
        service.ports.join(", ")
    };
    let targets = if service.target_pods.is_empty() {
        "no targets".to_string()
    } else {
        service.target_pods.join(", ")
    };
    let routes = if service.ingress_routes.is_empty() {
        "no ingress".to_string()
    } else {
        service.ingress_routes.join(", ")
    };
    let cluster_ip = service
        .cluster_ip
        .clone()
        .unwrap_or_else(|| "n/a".to_string());
    let get_command = kubectl_get_command(&cluster, &service.namespace, "service", &service.name);
    let forward_command = kubectl_port_forward_command(
        &cluster,
        &service.namespace,
        &format!("service/{}", service.name),
        8080,
        first_service_port(&service).unwrap_or(80),
    );

    rsx! {
        div { class: "cluster-table-row cluster-service-table-row",
            span { class: "cluster-row-main", "{service.name}" }
            span { "{service.service_type}" }
            span { "{cluster_ip}" }
            span { "{ports}" }
            span { "{service.endpoint_count} endpoints" }
            span { "{targets}" }
            span { "{routes}" }
            span { class: "cluster-row-actions",
                button {
                    class: "cluster-action cluster-action-mini",
                    onclick: move |_| match copy_to_clipboard(&get_command) {
                        Ok(()) => notice.set(Some(Notice::success("Copied service inspect command."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "YAML"
                }
                button {
                    class: "cluster-action cluster-action-mini cluster-action-muted",
                    onclick: move |_| match copy_to_clipboard(&forward_command) {
                        Ok(()) => notice.set(Some(Notice::success("Copied port-forward command."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Forward"
                }
            }
        }
    }
}

#[component]
fn EventRow(
    event: KubernetesEventSnapshot,
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let object = format!("{}/{}", event.involved_kind, event.involved_name);
    let last_seen = event
        .last_timestamp
        .clone()
        .or(event.first_timestamp.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let command = kubectl_get_command(
        &cluster,
        &event.namespace,
        &event.involved_kind.to_ascii_lowercase(),
        &event.involved_name,
    );

    rsx! {
        div { class: "cluster-event-row cluster-event-row-full",
            span { class: if event.event_type.eq_ignore_ascii_case("warning") { "cluster-event-type cluster-event-warn" } else { "cluster-event-type" }, "{event.event_type}" }
            strong { "{event.reason}" }
            span { "{object}" }
            span { "{event.count}x" }
            span { "{last_seen}" }
            p { "{event.message}" }
            button {
                class: "cluster-action cluster-action-mini cluster-action-muted",
                onclick: move |_| match copy_to_clipboard(&command) {
                    Ok(()) => notice.set(Some(Notice::success("Copied event object command."))),
                    Err(err) => notice.set(Some(Notice::error(err))),
                },
                "Inspect object"
            }
        }
    }
}

#[component]
fn ClusterTopologyTab(
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let mut selected_node_id = use_signal(|| None::<String>);
    let mut show_workloads = use_signal(|| true);
    let mut show_pods = use_signal(|| true);
    let mut show_networking = use_signal(|| true);
    let mut show_warnings = use_signal(|| true);
    let selected = selected_node_id();
    let topology = cluster.topology.clone();
    let visible_nodes = topology
        .nodes
        .iter()
        .filter(|node| {
            (show_workloads() || node.kind != "workload")
                && (show_pods() || node.kind != "pod")
                && (show_networking() || (node.kind != "service" && node.kind != "ingress"))
                && (show_warnings() || node.status != "healthy")
        })
        .cloned()
        .collect::<Vec<_>>();
    let visible_ids = visible_nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let visible_edges = topology
        .edges
        .iter()
        .filter(|edge| visible_ids.contains(&edge.from) && visible_ids.contains(&edge.to))
        .cloned()
        .collect::<Vec<_>>();
    let selected_node = selected
        .as_ref()
        .and_then(|id| visible_nodes.iter().find(|node| &node.id == id))
        .cloned()
        .or_else(|| visible_nodes.first().cloned());
    let summary = cluster_topology_summary(&cluster, &visible_nodes, &visible_edges);

    rsx! {
        div { class: "cluster-topology-workbench",
            div { class: "cluster-topology-toolbar",
                div { class: "cluster-actions cluster-actions-inline",
                    TopologyToggle { label: "Workloads".to_string(), enabled: show_workloads(), onclick: move |_| show_workloads.set(!show_workloads()) }
                    TopologyToggle { label: "Pods".to_string(), enabled: show_pods(), onclick: move |_| show_pods.set(!show_pods()) }
                    TopologyToggle { label: "Networking".to_string(), enabled: show_networking(), onclick: move |_| show_networking.set(!show_networking()) }
                    TopologyToggle { label: "Warnings".to_string(), enabled: show_warnings(), onclick: move |_| show_warnings.set(!show_warnings()) }
                }
                button {
                    class: "cluster-action cluster-action-muted",
                    onclick: move |_| match copy_to_clipboard(&summary) {
                        Ok(()) => notice.set(Some(Notice::success("Copied topology summary."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Copy summary"
                }
            }
            div { class: "cluster-topology-layout",
                div { class: "cluster-topology-map cluster-topology-map-live",
                    for node in visible_nodes {
                        button {
                            key: "{node.id}",
                            class: if Some(node.id.clone()) == selected {
                                "cluster-topology-node cluster-topology-node-selected {topology_status_class(&node.status)}"
                            } else {
                                "cluster-topology-node {topology_status_class(&node.status)}"
                            },
                            style: "grid-column: {node.column + 1}; grid-row: {node.row + 2};",
                            onclick: {
                                let node_id = node.id.clone();
                                move |_| selected_node_id.set(Some(node_id.clone()))
                            },
                            span { class: "cluster-topology-kind", "{node.kind}" }
                            strong { "{node.label}" }
                            span { "{node.subtitle}" }
                            if !node.badges.is_empty() {
                                em { "{node.badges.join(\" · \")}" }
                            }
                        }
                    }
                }
                div { class: "cluster-topology-detail",
                    if let Some(node) = selected_node {
                        h3 { "{node.label}" }
                        p { "{node.kind} · {node.status}" }
                        div { class: "cluster-dense-table",
                            for edge in visible_edges.iter().filter(|edge| edge.from == node.id || edge.to == node.id) {
                                div { key: "{edge.from}-{edge.to}-{edge.kind}", class: "cluster-dense-row",
                                    span { "{edge.kind}" }
                                    span { "{edge.from}" }
                                    span { "{edge.to}" }
                                    span { "{edge.status}" }
                                }
                            }
                        }
                    } else {
                        p { class: "cluster-muted", "No topology nodes available." }
                    }
                }
            }
        }
    }
}

#[component]
fn TopologyToggle(label: String, enabled: bool, onclick: EventHandler<MouseEvent>) -> Element {
    rsx! {
        button {
            class: if enabled { "cluster-tab cluster-tab-active" } else { "cluster-tab" },
            onclick: move |event| onclick.call(event),
            "{label}"
        }
    }
}

fn filtered_workloads(
    cluster: &KubernetesClusterSnapshot,
    query: &str,
) -> Vec<KubernetesWorkloadSnapshot> {
    let query = query.trim().to_ascii_lowercase();
    cluster
        .workloads
        .iter()
        .filter(|workload| {
            query.is_empty()
                || searchable_values([
                    workload.kind.as_str(),
                    workload.name.as_str(),
                    workload.namespace.as_str(),
                ])
                .contains(&query)
        })
        .cloned()
        .collect()
}

fn filtered_pods(cluster: &KubernetesClusterSnapshot, query: &str) -> Vec<KubernetesPodSnapshot> {
    let query = query.trim().to_ascii_lowercase();
    cluster
        .pods
        .iter()
        .filter(|pod| {
            query.is_empty()
                || searchable_values([
                    pod.name.as_str(),
                    pod.namespace.as_str(),
                    pod.phase.as_str(),
                    pod.owner_kind.as_deref().unwrap_or_default(),
                    pod.owner_name.as_deref().unwrap_or_default(),
                    pod.node_name.as_deref().unwrap_or_default(),
                    pod.warning_reason.as_deref().unwrap_or_default(),
                ])
                .contains(&query)
                || pod
                    .images
                    .iter()
                    .chain(pod.labels.iter())
                    .any(|value| value.to_ascii_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

fn filtered_services(
    cluster: &KubernetesClusterSnapshot,
    query: &str,
) -> Vec<KubernetesServiceSnapshot> {
    let query = query.trim().to_ascii_lowercase();
    cluster
        .services
        .iter()
        .filter(|service| {
            query.is_empty()
                || searchable_values([
                    service.name.as_str(),
                    service.namespace.as_str(),
                    service.service_type.as_str(),
                    service.cluster_ip.as_deref().unwrap_or_default(),
                ])
                .contains(&query)
                || service
                    .ports
                    .iter()
                    .chain(service.selector.iter())
                    .chain(service.target_pods.iter())
                    .chain(service.ingress_routes.iter())
                    .any(|value| value.to_ascii_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

fn filtered_events(
    cluster: &KubernetesClusterSnapshot,
    query: &str,
) -> Vec<KubernetesEventSnapshot> {
    let query = query.trim().to_ascii_lowercase();
    cluster
        .events
        .iter()
        .filter(|event| {
            query.is_empty()
                || searchable_values([
                    event.namespace.as_str(),
                    event.involved_kind.as_str(),
                    event.involved_name.as_str(),
                    event.event_type.as_str(),
                    event.reason.as_str(),
                    event.message.as_str(),
                ])
                .contains(&query)
        })
        .cloned()
        .collect()
}

fn healthy_workload_count(cluster: &KubernetesClusterSnapshot) -> usize {
    cluster
        .workloads
        .iter()
        .filter(
            |workload| match (workload.ready_replicas, workload.desired_replicas) {
                (Some(ready), Some(desired)) => ready >= desired,
                _ => true,
            },
        )
        .count()
}

fn first_service_port(service: &KubernetesServiceSnapshot) -> Option<u16> {
    service.ports.first().and_then(|label| {
        let port = label
            .split(':')
            .next_back()
            .unwrap_or(label)
            .split("->")
            .next()
            .unwrap_or(label);
        port.parse::<u16>().ok()
    })
}

fn cluster_topology_summary(
    cluster: &KubernetesClusterSnapshot,
    nodes: &[loopbox::KubernetesTopologyNode],
    edges: &[loopbox::KubernetesTopologyEdge],
) -> String {
    format!(
        "Cluster: {}\nNamespace: {}\nTopology nodes: {}\nTopology edges: {}\nWarnings: {}",
        cluster.name,
        cluster.selected_namespace,
        nodes.len(),
        edges.len(),
        cluster.warnings.join("; ")
    )
}

fn topology_status_class(status: &str) -> &'static str {
    match status {
        "healthy" => "cluster-topology-node-ok",
        "warning" => "cluster-topology-node-warn",
        "error" => "cluster-topology-node-error",
        _ => "",
    }
}

#[component]
fn WorkloadRow(workload: KubernetesWorkloadSnapshot) -> Element {
    let ready = workload
        .ready_replicas
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let desired = workload
        .desired_replicas
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    rsx! {
        div { class: "cluster-table-row",
            span { class: "cluster-row-main", "{workload.name}" }
            span { "{workload.kind}" }
            span { "{ready}/{desired} ready" }
        }
    }
}

#[component]
fn ServiceRow(service: KubernetesServiceSnapshot) -> Element {
    let ports = if service.ports.is_empty() {
        "no ports".to_string()
    } else {
        service.ports.join(", ")
    };
    let cluster_ip = service.cluster_ip.unwrap_or_else(|| "n/a".to_string());

    rsx! {
        div { class: "cluster-table-row",
            span { class: "cluster-row-main", "{service.name}" }
            span { "{service.service_type}" }
            span { "{cluster_ip}" }
            span { "{ports}" }
        }
    }
}

fn cluster_ready_pod_count(cluster: &KubernetesClusterSnapshot) -> usize {
    cluster
        .pods
        .iter()
        .filter(|pod| pod.total_containers > 0 && pod.ready_containers == pod.total_containers)
        .count()
}

fn cluster_warning_event_count(cluster: &KubernetesClusterSnapshot) -> usize {
    cluster
        .events
        .iter()
        .filter(|event| event.event_type.eq_ignore_ascii_case("warning"))
        .count()
}

#[cfg(test)]
fn cluster_search_matches(cluster: &KubernetesClusterSnapshot, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    cluster.workloads.iter().any(|workload| {
        searchable_values([
            workload.kind.as_str(),
            workload.name.as_str(),
            workload.namespace.as_str(),
        ])
        .contains(&query)
    }) || cluster.pods.iter().any(|pod| {
        searchable_values([
            pod.name.as_str(),
            pod.namespace.as_str(),
            pod.phase.as_str(),
            pod.owner_kind.as_deref().unwrap_or_default(),
            pod.owner_name.as_deref().unwrap_or_default(),
            pod.node_name.as_deref().unwrap_or_default(),
            pod.warning_reason.as_deref().unwrap_or_default(),
        ])
        .contains(&query)
            || pod
                .images
                .iter()
                .chain(pod.labels.iter())
                .any(|value| value.to_ascii_lowercase().contains(&query))
    }) || cluster.services.iter().any(|service| {
        searchable_values([
            service.name.as_str(),
            service.namespace.as_str(),
            service.service_type.as_str(),
            service.cluster_ip.as_deref().unwrap_or_default(),
        ])
        .contains(&query)
            || service
                .ports
                .iter()
                .chain(service.selector.iter())
                .chain(service.target_pods.iter())
                .chain(service.ingress_routes.iter())
                .any(|value| value.to_ascii_lowercase().contains(&query))
    }) || cluster.events.iter().any(|event| {
        searchable_values([
            event.namespace.as_str(),
            event.involved_kind.as_str(),
            event.involved_name.as_str(),
            event.event_type.as_str(),
            event.reason.as_str(),
            event.message.as_str(),
        ])
        .contains(&query)
    })
}

fn searchable_values<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    values
        .into_iter()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn kubectl_base_args(cluster: &KubernetesClusterSnapshot, namespace: &str) -> Vec<String> {
    vec![
        "--context".to_string(),
        cluster.context.clone(),
        "--namespace".to_string(),
        namespace.to_string(),
    ]
}

fn kubectl_get_command(
    cluster: &KubernetesClusterSnapshot,
    namespace: &str,
    kind: &str,
    name: &str,
) -> String {
    let mut args = kubectl_base_args(cluster, namespace);
    args.extend([
        "get".to_string(),
        kind.to_string(),
        name.to_string(),
        "-o".to_string(),
        "yaml".to_string(),
    ]);
    kubectl_command(args)
}

fn kubectl_logs_command(cluster: &KubernetesClusterSnapshot, namespace: &str, pod: &str) -> String {
    let mut args = kubectl_base_args(cluster, namespace);
    args.extend([
        "logs".to_string(),
        pod.to_string(),
        "--tail=200".to_string(),
        "--follow".to_string(),
    ]);
    kubectl_command(args)
}

fn kubectl_port_forward_command(
    cluster: &KubernetesClusterSnapshot,
    namespace: &str,
    target: &str,
    local_port: u16,
    remote_port: u16,
) -> String {
    let mut args = kubectl_base_args(cluster, namespace);
    args.extend([
        "port-forward".to_string(),
        target.to_string(),
        format!("{local_port}:{remote_port}"),
    ]);
    kubectl_command(args)
}

fn kubectl_command(args: Vec<String>) -> String {
    std::iter::once("kubectl".to_string())
        .chain(args.into_iter().map(|arg| shell_quote(&arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '='))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{
        KubernetesEndpointSliceSnapshot, KubernetesIngressSnapshot, KubernetesNamespaceSnapshot,
        KubernetesProvider, KubernetesTopologySnapshot,
    };

    #[test]
    fn cluster_pod_ready_count_counts_ready_pods() {
        let cluster = cluster_with_pods(vec![
            pod("api-1", "Running", 2, 2, 0),
            pod("api-2", "Running", 1, 2, 0),
            pod("job-1", "Succeeded", 1, 1, 0),
        ]);

        assert_eq!(cluster_ready_pod_count(&cluster), 2);
    }

    #[test]
    fn cluster_warning_event_count_counts_warning_events() {
        let mut cluster = cluster_with_pods(Vec::new());
        cluster.events = vec![
            event("Warning", "BackOff", "api-1", "restarting"),
            event("Normal", "Pulled", "api-1", "image pulled"),
            event("warning", "Unhealthy", "api-2", "probe failed"),
        ];

        assert_eq!(cluster_warning_event_count(&cluster), 2);
    }

    #[test]
    fn cluster_search_matches_workload_pod_service_and_event_text() {
        let mut cluster = cluster_with_pods(vec![pod("api-7f9d", "Running", 1, 1, 0)]);
        cluster.workloads = vec![KubernetesWorkloadSnapshot {
            kind: "deployment".to_string(),
            name: "checkout-api".to_string(),
            namespace: "apps".to_string(),
            desired_replicas: Some(2),
            ready_replicas: Some(2),
            available_replicas: Some(2),
        }];
        cluster.services = vec![KubernetesServiceSnapshot {
            name: "checkout-http".to_string(),
            namespace: "apps".to_string(),
            service_type: "ClusterIP".to_string(),
            cluster_ip: Some("10.43.10.20".to_string()),
            ports: vec!["http:80->8080/TCP".to_string()],
            selector: vec!["app=checkout".to_string()],
            external_ips: Vec::new(),
            endpoint_count: 1,
            target_pods: vec!["api-7f9d".to_string()],
            ingress_routes: Vec::new(),
        }];
        cluster.events = vec![event("Warning", "BackOff", "api-7f9d", "checkout failed")];

        assert!(cluster_search_matches(&cluster, "checkout"));
        assert!(cluster_search_matches(&cluster, "BackOff"));
        assert!(cluster_search_matches(&cluster, "api-7f9d"));
        assert!(!cluster_search_matches(&cluster, "billing"));
    }

    #[test]
    fn kubectl_commands_include_context_namespace_and_shell_quoting() {
        let cluster = cluster_with_pods(Vec::new());

        assert_eq!(
            kubectl_get_command(&cluster, "apps", "pod", "api one"),
            "kubectl --context prod-context --namespace apps get pod 'api one' -o yaml"
        );
        assert_eq!(
            kubectl_logs_command(&cluster, "apps", "api one"),
            "kubectl --context prod-context --namespace apps logs 'api one' --tail=200 --follow"
        );
        assert_eq!(
            kubectl_port_forward_command(&cluster, "apps", "service/checkout", 8080, 80),
            "kubectl --context prod-context --namespace apps port-forward service/checkout 8080:80"
        );
    }

    fn cluster_with_pods(pods: Vec<KubernetesPodSnapshot>) -> KubernetesClusterSnapshot {
        KubernetesClusterSnapshot {
            name: "prod".to_string(),
            provider: KubernetesProvider::Remote,
            context: "prod-context".to_string(),
            default_namespace: "apps".to_string(),
            selected_namespace: "apps".to_string(),
            connectivity: KubernetesConnectivityState::NotConfigured,
            namespaces: vec![KubernetesNamespaceSnapshot {
                name: "apps".to_string(),
            }],
            workloads: Vec::new(),
            services: Vec::new(),
            nodes: Vec::new(),
            pods,
            ingresses: Vec::<KubernetesIngressSnapshot>::new(),
            endpoint_slices: Vec::<KubernetesEndpointSliceSnapshot>::new(),
            events: Vec::new(),
            topology: KubernetesTopologySnapshot::default(),
            warnings: Vec::new(),
            last_error: None,
        }
    }

    fn pod(
        name: &str,
        phase: &str,
        ready_containers: u64,
        total_containers: u64,
        restart_count: u64,
    ) -> KubernetesPodSnapshot {
        KubernetesPodSnapshot {
            name: name.to_string(),
            namespace: "apps".to_string(),
            phase: phase.to_string(),
            ready_containers,
            total_containers,
            restart_count,
            owner_kind: Some("ReplicaSet".to_string()),
            owner_name: Some("api-7f9d".to_string()),
            node_name: Some("node-a".to_string()),
            pod_ip: Some("10.1.0.15".to_string()),
            images: vec!["ghcr.io/acme/api:v1".to_string()],
            labels: vec!["app=checkout".to_string()],
            warning_reason: None,
        }
    }

    fn event(
        event_type: &str,
        reason: &str,
        involved_name: &str,
        message: &str,
    ) -> KubernetesEventSnapshot {
        KubernetesEventSnapshot {
            namespace: "apps".to_string(),
            involved_kind: "Pod".to_string(),
            involved_name: involved_name.to_string(),
            event_type: event_type.to_string(),
            reason: reason.to_string(),
            message: message.to_string(),
            count: 1,
            first_timestamp: None,
            last_timestamp: Some("2026-05-12T10:00:00Z".to_string()),
        }
    }
}
