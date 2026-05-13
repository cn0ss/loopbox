use crate::app::models::Notice;
use crate::app::utils::copy_to_clipboard;
use crate::loopbox::KubernetesClusterSnapshot;
use dioxus::prelude::*;

#[component]
pub(super) fn ClusterDiagnostics(
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let ready_nodes = cluster.nodes.iter().filter(|node| node.ready).count();
    let ready_pods = cluster
        .pods
        .iter()
        .filter(|pod| pod.ready_containers == pod.total_containers && pod.total_containers > 0)
        .count();
    let warning_events = cluster
        .events
        .iter()
        .filter(|event| event.event_type.eq_ignore_ascii_case("warning"))
        .count();
    let services_with_endpoints = cluster
        .services
        .iter()
        .filter(|service| service.endpoint_count > 0)
        .count();
    let summary = cluster_diagnostic_summary(&cluster);
    let summary_for_copy = summary.clone();

    rsx! {
        section { class: "cluster-diagnostics",
            div { class: "cluster-diagnostics-head",
                div {
                    h3 { "Diagnosis cockpit" }
                    p { "Namespace {cluster.selected_namespace} · {cluster.topology.nodes.len()} topology nodes · {cluster.warnings.len()} warning(s)" }
                }
                button {
                    class: "cluster-action cluster-action-muted",
                    onclick: move |_| match copy_to_clipboard(&summary_for_copy) {
                        Ok(()) => notice.set(Some(Notice::success("Copied cluster diagnosis summary."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Copy summary"
                }
            }

            div { class: "cluster-diagnostic-stats",
                DiagnosticStat { label: "Nodes".to_string(), value: format!("{ready_nodes}/{} ready", cluster.nodes.len()) }
                DiagnosticStat { label: "Pods".to_string(), value: format!("{ready_pods}/{} ready", cluster.pods.len()) }
                DiagnosticStat { label: "Services".to_string(), value: format!("{services_with_endpoints}/{} with endpoints", cluster.services.len()) }
                DiagnosticStat { label: "Events".to_string(), value: format!("{warning_events} warning") }
            }

            if !cluster.warnings.is_empty() {
                div { class: "cluster-warning-strip",
                    for warning in &cluster.warnings {
                        span { key: "{warning}", "{warning}" }
                    }
                }
            }

            ClusterTopologyPreview { cluster: cluster.clone(), notice }

            div { class: "cluster-diagnostic-grid",
                DiagnosticPanel { title: "Nodes".to_string(),
                    if cluster.nodes.is_empty() {
                        p { class: "cluster-muted", "No node data available." }
                    } else {
                        div { class: "cluster-dense-table",
                            for node in &cluster.nodes {
                                div { key: "{node.name}", class: "cluster-dense-row",
                                    span { class: "cluster-row-main", "{node.name}" }
                                    span { if node.ready { "ready" } else { "not ready" } }
                                    span { "{node.roles.join(\", \")}" }
                                    span { "{node.internal_ip.clone().unwrap_or_else(|| \"n/a\".to_string())}" }
                                }
                            }
                        }
                    }
                }

                DiagnosticPanel { title: "Pods".to_string(),
                    if cluster.pods.is_empty() {
                        p { class: "cluster-muted", "No pod data available." }
                    } else {
                        div { class: "cluster-pod-list",
                            for pod in &cluster.pods {
                                div { key: "{pod.namespace}-{pod.name}", class: "cluster-pod-row",
                                    div { class: "cluster-pod-main",
                                        strong { "{pod.name}" }
                                        span { "{pod.node_name.clone().unwrap_or_else(|| \"n/a\".to_string())}" }
                                    }
                                    div { class: "cluster-pod-meta",
                                        span { class: "{pod_phase_class(&pod.phase)}", "{pod.phase}" }
                                        span { "{pod.ready_containers}/{pod.total_containers} ready" }
                                        span { "{pod.restart_count} restarts" }
                                    }
                                }
                            }
                        }
                    }
                }

                DiagnosticPanel { title: "Ingress".to_string(),
                    if cluster.ingresses.is_empty() {
                        p { class: "cluster-muted", "No ingress data available." }
                    } else {
                        div { class: "cluster-dense-table",
                            for ingress in &cluster.ingresses {
                                div { key: "{ingress.namespace}-{ingress.name}", class: "cluster-dense-row",
                                    span { class: "cluster-row-main", "{ingress.name}" }
                                    span { "{ingress.hosts.join(\", \")}" }
                                    span { "{ingress.service_backends.join(\", \")}" }
                                    span { "{ingress.class_name.clone().unwrap_or_else(|| \"n/a\".to_string())}" }
                                }
                            }
                        }
                    }
                }

                DiagnosticPanel { title: "Events".to_string(),
                    if cluster.events.is_empty() {
                        p { class: "cluster-muted", "No event data available." }
                    } else {
                        div { class: "cluster-event-list",
                            for event in cluster.events.iter().take(8) {
                                div { key: "{event.namespace}-{event.involved_kind}-{event.involved_name}-{event.reason}", class: "cluster-event-row",
                                    span { class: if event.event_type.eq_ignore_ascii_case("warning") { "cluster-event-type cluster-event-warn" } else { "cluster-event-type" }, "{event.event_type}" }
                                    strong { "{event.reason}" }
                                    span { "{event.involved_kind}/{event.involved_name}" }
                                    p { "{event.message}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DiagnosticStat(label: String, value: String) -> Element {
    rsx! {
        div { class: "cluster-diagnostic-stat",
            span { "{label}" }
            strong { "{value}" }
        }
    }
}

#[component]
fn DiagnosticPanel(title: String, children: Element) -> Element {
    rsx! {
        div { class: "cluster-diagnostic-panel",
            h4 { "{title}" }
            {children}
        }
    }
}

#[component]
fn ClusterTopologyPreview(
    cluster: KubernetesClusterSnapshot,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let topology = cluster.topology.clone();
    let copy_command = kubectl_get_namespace_command(&cluster);
    let copy_command_for_click = copy_command.clone();
    let mut topology_columns = Vec::new();
    for node in topology.nodes.iter().cloned() {
        if let Some((_, nodes)) = topology_columns
            .iter_mut()
            .find(|(column, _): &&mut (usize, Vec<_>)| *column == node.column)
        {
            nodes.push(node);
        } else {
            topology_columns.push((node.column, vec![node]));
        }
    }
    topology_columns.sort_by_key(|(column, _)| *column);
    for (_, nodes) in &mut topology_columns {
        nodes.sort_by_key(|node| node.row);
    }
    let hidden_edges = topology.edges.len().saturating_sub(12);

    rsx! {
        div { class: "cluster-topology-shell",
            div { class: "cluster-topology-toolbar",
                span { "{topology.nodes.len()} nodes" }
                span { "{topology.edges.len()} edges" }
                button {
                    class: "cluster-action cluster-action-muted",
                    onclick: move |_| match copy_to_clipboard(&copy_command_for_click) {
                        Ok(()) => notice.set(Some(Notice::success("Copied kubectl command."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Copy kubectl"
                }
            }
            if !topology.edges.is_empty() {
                div { class: "cluster-topology-edges",
                    for edge in topology.edges.iter().take(12) {
                        span {
                            key: "{edge.from}-{edge.to}-{edge.kind}",
                            class: "cluster-topology-edge",
                            "{edge.kind}"
                        }
                    }
                    if hidden_edges > 0 {
                        span { class: "cluster-topology-edge cluster-topology-edge-muted", "+{hidden_edges} more" }
                    }
                }
            }
            div { class: "cluster-topology-map",
                for (column, nodes) in &topology_columns {
                    div { key: "{column}", class: "cluster-topology-column",
                        span { class: "cluster-topology-column-label", "{topology_column_label(*column)}" }
                        for node in nodes {
                            div {
                                key: "{node.id}",
                                class: "cluster-topology-node {topology_status_class(&node.status)}",
                                span { class: "cluster-topology-kind", "{node.kind}" }
                                strong { "{node.label}" }
                                span { "{node.subtitle}" }
                                if !node.badges.is_empty() {
                                    em { "{node.badges.join(\" · \")}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn topology_column_label(column: usize) -> &'static str {
    match column {
        0 => "Namespace",
        1 => "Workloads",
        2 => "Pods",
        3 => "Services",
        4 => "Ingress",
        _ => "Other",
    }
}

fn topology_status_class(status: &str) -> &'static str {
    match status {
        "healthy" => "cluster-topology-node-ok",
        "warning" => "cluster-topology-node-warn",
        "error" => "cluster-topology-node-error",
        _ => "",
    }
}

fn pod_phase_class(phase: &str) -> &'static str {
    match phase {
        "Running" => "cluster-pod-chip cluster-pod-chip-ok",
        "Succeeded" => "cluster-pod-chip",
        "Failed" => "cluster-pod-chip cluster-pod-chip-error",
        _ => "cluster-pod-chip cluster-pod-chip-warn",
    }
}

fn kubectl_get_namespace_command(cluster: &KubernetesClusterSnapshot) -> String {
    format!(
        "kubectl --context {} --namespace {} get pods,svc,ingress,events",
        shell_quote(&cluster.context),
        shell_quote(&cluster.selected_namespace)
    )
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn cluster_diagnostic_summary(cluster: &KubernetesClusterSnapshot) -> String {
    let warning_events = cluster
        .events
        .iter()
        .filter(|event| event.event_type.eq_ignore_ascii_case("warning"))
        .count();
    format!(
        "Cluster: {}\nContext: {}\nNamespace: {}\nNodes: {}\nPods: {}\nServices: {}\nIngresses: {}\nWarning events: {}\nWarnings: {}",
        cluster.name,
        cluster.context,
        cluster.selected_namespace,
        cluster.nodes.len(),
        cluster.pods.len(),
        cluster.services.len(),
        cluster.ingresses.len(),
        warning_events,
        cluster.warnings.join("; ")
    )
}
