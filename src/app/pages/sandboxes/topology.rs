use super::*;
use crate::app::runtime_view::{format_cpu_percent, format_memory_bytes};
use crate::loopbox::{ProxyTrafficEvent, ServiceResourceSample, ServiceRuntimeKind};

const SLOW_TRAFFIC_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopologyNodeKind {
    Project,
    Service,
    ProxyEndpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopologyEdgeKind {
    DependsOn,
    HttpIngress,
    ProxyEndpoint,
    RecentTraffic,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TopologyIncidentOverlay {
    pub(super) warning: usize,
    pub(super) critical: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TopologyTrafficOverlay {
    pub(super) total: usize,
    pub(super) failures: usize,
    pub(super) slow: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopologyResourceOverlay {
    pub(super) cpu_label: String,
    pub(super) memory_label: String,
    pub(super) process_label: String,
}

impl Default for TopologyResourceOverlay {
    fn default() -> Self {
        Self {
            cpu_label: "n/a".to_string(),
            memory_label: "n/a".to_string(),
            process_label: "n/a".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopologyNode {
    pub(super) id: String,
    pub(super) kind: TopologyNodeKind,
    pub(super) label: String,
    pub(super) subtitle: String,
    pub(super) service_name: Option<String>,
    pub(super) endpoint_name: Option<String>,
    pub(super) state: Option<ServiceRuntimeState>,
    pub(super) runtime_kind: Option<ServiceRuntimeKind>,
    pub(super) ports: Vec<String>,
    pub(super) url: Option<String>,
    pub(super) command: Option<String>,
    pub(super) workdir: Option<String>,
    pub(super) health_paths: Vec<String>,
    pub(super) incidents: TopologyIncidentOverlay,
    pub(super) traffic: TopologyTrafficOverlay,
    pub(super) resource: TopologyResourceOverlay,
    pub(super) column: usize,
    pub(super) row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopologyEdge {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) kind: TopologyEdgeKind,
    pub(super) label: String,
    pub(super) count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TopologySnapshot {
    pub(super) project_name: String,
    pub(super) project_ip: String,
    pub(super) nodes: Vec<TopologyNode>,
    pub(super) edges: Vec<TopologyEdge>,
    pub(super) warnings: Vec<String>,
    pub(super) service_count: usize,
    pub(super) active_count: usize,
    pub(super) incident_count: usize,
    pub(super) traffic_count: usize,
    pub(super) max_column: usize,
    pub(super) max_row: usize,
}

impl TopologySnapshot {
    pub(super) fn node(&self, id: &str) -> Option<&TopologyNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

#[component]
pub(super) fn ProjectDetailTopologyTab(
    project_name: String,
    project: ProjectConfig,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
    mut current_page: Signal<Page>,
) -> Element {
    let mut selected_node_id = use_signal(|| None::<String>);
    let mut window_filter = use_signal(|| "1h".to_string());
    let mut show_dependencies = use_signal(|| true);
    let mut show_ingress = use_signal(|| true);
    let mut show_traffic = use_signal(|| true);
    let mut show_incidents = use_signal(|| true);

    let topology_project_name = project_name.clone();
    let topology_project = project.clone();
    let topology_snapshot = use_resource(move || {
        let cfg = config();
        let tick = runtime_tick();
        let window = window_filter();
        let topology_project_name = topology_project_name.clone();
        let topology_project = topology_project.clone();
        async move {
            let _ = tick;
            tokio::task::spawn_blocking(move || {
                let mut statuses = BTreeMap::new();
                for service in &topology_project.services {
                    if let Ok(status) =
                        loopbox::service_runtime_status(&cfg, &topology_project_name, &service.name)
                    {
                        statuses.insert(service.name.clone(), status);
                    }
                }
                let latest_resources =
                    loopbox::resource_metrics_latest_for_config(&cfg).unwrap_or_default();
                let incidents = loopbox::incident_timeline_for_project(
                    &cfg,
                    &topology_project_name,
                    None,
                    &window,
                    300,
                )
                .unwrap_or_default();
                let traffic = loopbox::proxy_traffic_events_for_project_with_persisted(
                    &topology_project_name,
                    None,
                    300,
                )
                .unwrap_or_default();
                build_topology_snapshot(
                    &cfg,
                    &topology_project_name,
                    &topology_project,
                    &statuses,
                    &latest_resources,
                    &incidents,
                    &traffic,
                )
            })
            .await
            .map_err(|err| format!("Topology task failed: {err}"))
        }
    });

    let loading = topology_snapshot().is_none();
    let snapshot_result = topology_snapshot();
    let snapshot_error = snapshot_result
        .as_ref()
        .and_then(|result| result.as_ref().err().cloned());
    let snapshot = snapshot_result.and_then(Result::ok).unwrap_or_else(|| {
        build_topology_snapshot(
            &config(),
            &project_name,
            &project,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &[],
        )
    });
    let selected_id = selected_node_id()
        .filter(|id| snapshot.node(id).is_some())
        .or_else(|| {
            snapshot
                .nodes
                .iter()
                .find(|node| node.kind == TopologyNodeKind::Service)
                .or_else(|| snapshot.nodes.first())
                .map(|node| node.id.clone())
        });
    let selected_node = selected_id
        .as_ref()
        .and_then(|id| snapshot.node(id))
        .cloned();
    let selected_id_for_class = selected_id.clone();
    let visible_edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            topology_edge_visible(
                edge.kind,
                show_dependencies(),
                show_ingress(),
                show_traffic(),
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let canvas_width = topology_canvas_width(&snapshot);
    let canvas_height = topology_canvas_height(&snapshot);
    let canvas_style = format!("min-width:{canvas_width}px;min-height:{canvas_height}px;");
    let summary_text = topology_snapshot_summary(&snapshot);
    let window_snapshot = window_filter();

    rsx! {
        div { class: "tab-content topology-tab",
            div { class: "topology-toolbar",
                div { class: "topology-toolbar-left",
                    div { class: "seg-control",
                        for window in ["15m", "1h", "24h", "7d"] {
                            button {
                                key: "topology-window-{window}",
                                class: if window_snapshot == window { "seg-btn seg-btn-on" } else { "seg-btn" },
                                onclick: move |_| window_filter.set(window.to_string()),
                                "{window}"
                            }
                        }
                    }
                    span { class: "topology-summary-chip", "{snapshot.service_count} services" }
                    span { class: "topology-summary-chip", "{snapshot.active_count} active" }
                    if show_incidents() {
                        span { class: if snapshot.incident_count > 0 { "topology-summary-chip topology-summary-chip-warn" } else { "topology-summary-chip" },
                            "{snapshot.incident_count} incidents"
                        }
                    }
                    span { class: "topology-summary-chip", "{snapshot.traffic_count} requests" }
                }
                div { class: "topology-toolbar-right",
                    button {
                        class: topology_toggle_class(show_dependencies()),
                        onclick: move |_| show_dependencies.set(!show_dependencies()),
                        "Dependencies"
                    }
                    button {
                        class: topology_toggle_class(show_ingress()),
                        onclick: move |_| show_ingress.set(!show_ingress()),
                        "Ingress"
                    }
                    button {
                        class: topology_toggle_class(show_traffic()),
                        onclick: move |_| show_traffic.set(!show_traffic()),
                        "Traffic"
                    }
                    button {
                        class: topology_toggle_class(show_incidents()),
                        onclick: move |_| show_incidents.set(!show_incidents()),
                        "Incidents"
                    }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let summary_text = summary_text.clone();
                            move |_| match copy_to_clipboard(&summary_text) {
                                Ok(()) => notice.set(Some(Notice::success("Copied topology summary."))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Copy Summary"
                    }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: move |_| runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1)),
                        "Refresh"
                    }
                }
            }

            if loading {
                div { class: "topology-inline-state", "Loading topology..." }
            }
            if let Some(error) = snapshot_error {
                div { class: "topology-inline-state topology-inline-state-error", "{error}" }
            }
            if !snapshot.warnings.is_empty() {
                div { class: "topology-warning-strip",
                    for warning in &snapshot.warnings {
                        span { key: "{warning}", "{warning}" }
                    }
                }
            }

            div { class: "topology-layout",
                section { class: "topology-map-shell",
                    div { class: "topology-map", style: "{canvas_style}",
                        svg {
                            class: "topology-edges",
                            view_box: "0 0 {canvas_width} {canvas_height}",
                            preserve_aspect_ratio: "none",
                            defs {
                                marker {
                                    id: "topology-arrow",
                                    view_box: "0 0 10 10",
                                    ref_x: "8",
                                    ref_y: "5",
                                    marker_width: "5",
                                    marker_height: "5",
                                    orient: "auto-start-reverse",
                                    path { d: "M 0 0 L 10 5 L 0 10 z" }
                                }
                            }
                            for edge in visible_edges.iter() {
                                if let Some(points) = topology_edge_points(&snapshot, edge) {
                                    line {
                                        key: "{edge.from}-{edge.to}-{topology_edge_kind_label(edge.kind)}",
                                        class: topology_edge_class(edge),
                                        x1: "{points.0}",
                                        y1: "{points.1}",
                                        x2: "{points.2}",
                                        y2: "{points.3}",
                                        marker_end: "url(#topology-arrow)",
                                    }
                                }
                            }
                        }
                        for node in &snapshot.nodes {
                            {{
                                let selected = selected_id_for_class.as_ref() == Some(&node.id);
                                let node_style = topology_node_style(node);
                                let node_class = topology_node_class(node, selected);
                                let node_id = node.id.clone();
                                let incident_label = topology_incident_label(&node.incidents);
                                let traffic_label = topology_node_traffic_label(&node.traffic);
                                rsx! {
                                    button {
                                        key: "{node.id}",
                                        class: "{node_class}",
                                        style: "{node_style}",
                                        onclick: move |_| selected_node_id.set(Some(node_id.clone())),
                                        span { class: "topology-node-kind", "{topology_node_kind_label(node.kind)}" }
                                        strong { "{node.label}" }
                                        span { class: "topology-node-subtitle", "{node.subtitle}" }
                                        if !node.ports.is_empty() {
                                            span { class: "topology-node-ports", "{node.ports.join(\", \")}" }
                                        }
                                        if node.kind == TopologyNodeKind::Service {
                                            span { class: "topology-node-metrics",
                                                "{node.resource.cpu_label} CPU · {node.resource.memory_label}"
                                            }
                                        }
                                        span { class: "topology-node-badges",
                                            if show_incidents() {
                                                if let Some(label) = incident_label.as_ref() {
                                                    span { class: "topology-badge topology-badge-warn", "{label}" }
                                                }
                                            }
                                            if let Some(label) = traffic_label.as_ref() {
                                                span { class: "topology-badge", "{label}" }
                                            }
                                        }
                                    }
                                }
                            }}
                        }
                    }
                }

                {render_topology_detail(
                    selected_node,
                    &snapshot,
                    &summary_text,
                    &project_name,
                    config,
                    notice,
                    current_page,
                )}
            }
        }
    }
}

pub(super) fn build_topology_snapshot(
    config: &LoopboxConfig,
    project_name: &str,
    project: &ProjectConfig,
    runtime: &BTreeMap<String, ServiceRuntimeSnapshot>,
    latest_resources: &BTreeMap<String, ServiceResourceSample>,
    incidents: &[IncidentTimelineEvent],
    traffic: &[ProxyTrafficEvent],
) -> TopologySnapshot {
    let service_names = project
        .services
        .iter()
        .map(|service| service.name.clone())
        .collect::<BTreeSet<_>>();
    let mut warnings = Vec::new();
    let mut edges = Vec::new();
    let mut incident_counts = BTreeMap::<String, TopologyIncidentOverlay>::new();
    let mut traffic_counts = BTreeMap::<String, TopologyTrafficOverlay>::new();

    for event in incidents
        .iter()
        .filter(|event| event.project_name == project_name)
    {
        let Some(service_name) = event.service_name.as_ref() else {
            continue;
        };
        if !service_names.contains(service_name) {
            continue;
        }
        let counts = incident_counts.entry(service_name.clone()).or_default();
        match event.severity {
            IncidentSeverity::Critical => counts.critical += 1,
            IncidentSeverity::Warning => counts.warning += 1,
            IncidentSeverity::Info => {}
        }
    }

    for event in traffic
        .iter()
        .filter(|event| event.project_name == project_name)
    {
        if !service_names.contains(&event.service_name) {
            continue;
        }
        let counts = traffic_counts
            .entry(event.service_name.clone())
            .or_default();
        counts.total += 1;
        if topology_traffic_event_failed(event) {
            counts.failures += 1;
        }
        if event.duration_ms >= SLOW_TRAFFIC_MS {
            counts.slow += 1;
        }
    }

    let depths = topology_service_depths(project, &service_names, &mut warnings);
    let rows = topology_service_rows(project, &depths);
    let mut nodes = Vec::new();
    nodes.push(TopologyNode {
        id: project_node_id(project_name),
        kind: TopologyNodeKind::Project,
        label: project_name.to_string(),
        subtitle: project.ip.clone(),
        service_name: None,
        endpoint_name: None,
        state: None,
        runtime_kind: None,
        ports: Vec::new(),
        url: Some(loopbox::project_primary_host(config, project_name)),
        command: None,
        workdir: Some(project.dir.clone()),
        health_paths: Vec::new(),
        incidents: TopologyIncidentOverlay::default(),
        traffic: TopologyTrafficOverlay::default(),
        resource: TopologyResourceOverlay::default(),
        column: 0,
        row: 0,
    });

    for service in &project.services {
        let service_id = service_node_id(&service.name);
        for dependency in &service.depends_on {
            if service_names.contains(dependency) {
                edges.push(TopologyEdge {
                    from: service_node_id(dependency),
                    to: service_id.clone(),
                    kind: TopologyEdgeKind::DependsOn,
                    label: "depends on".to_string(),
                    count: 1,
                });
            } else {
                warnings.push(format!(
                    "Service '{}' depends on unknown service '{}'.",
                    service.name, dependency
                ));
            }
        }

        let ports = loopbox::service_ports(service);
        let has_http = ports
            .iter()
            .any(|port| port.protocol == ProxyEndpointProtocol::Http1);
        if has_http {
            edges.push(TopologyEdge {
                from: project_node_id(project_name),
                to: service_id.clone(),
                kind: TopologyEdgeKind::HttpIngress,
                label: topology_service_host(project_name, &service.name, config)
                    .unwrap_or_else(|| format!("{}.{}", service.name, project_name)),
                count: ports
                    .iter()
                    .filter(|port| port.protocol == ProxyEndpointProtocol::Http1)
                    .count()
                    .max(1),
            });
        }

        if let Some(counts) = traffic_counts.get(&service.name) {
            edges.push(TopologyEdge {
                from: project_node_id(project_name),
                to: service_id.clone(),
                kind: TopologyEdgeKind::RecentTraffic,
                label: topology_traffic_edge_label(counts),
                count: counts.total,
            });
        }

        let depth = depths.get(&service.name).copied().unwrap_or(0);
        let row = rows.get(&service.name).copied().unwrap_or(0);
        let snapshot = runtime.get(&service.name);
        let resource_key = format!("{project_name}::{}", service.name);
        let resource = latest_resources
            .get(&resource_key)
            .map(topology_resource_overlay)
            .unwrap_or_default();
        let runtime_kind = service.runtime;
        nodes.push(TopologyNode {
            id: service_id,
            kind: TopologyNodeKind::Service,
            label: service.name.clone(),
            subtitle: topology_service_subtitle(service, snapshot),
            service_name: Some(service.name.clone()),
            endpoint_name: None,
            state: snapshot.map(|snapshot| snapshot.state),
            runtime_kind: Some(runtime_kind),
            ports: topology_port_labels(&ports),
            url: topology_service_host(project_name, &service.name, config),
            command: Some(topology_service_command(service)),
            workdir: Some(service.workdir.clone()),
            health_paths: ports
                .iter()
                .filter_map(|port| port.health_path.clone())
                .collect(),
            incidents: incident_counts
                .get(&service.name)
                .cloned()
                .unwrap_or_default(),
            traffic: traffic_counts
                .get(&service.name)
                .cloned()
                .unwrap_or_default(),
            resource,
            column: depth + 1,
            row,
        });
    }

    let endpoint_start_row = project.services.len().saturating_add(1);
    for (index, endpoint) in project.proxy_endpoints.iter().enumerate() {
        let endpoint_id = endpoint_node_id(&endpoint.name);
        if let Some(service_name) = endpoint.service_name.as_ref() {
            if service_names.contains(service_name) {
                edges.push(TopologyEdge {
                    from: endpoint_id.clone(),
                    to: service_node_id(service_name),
                    kind: TopologyEdgeKind::ProxyEndpoint,
                    label: topology_endpoint_edge_label(endpoint),
                    count: 1,
                });
            } else {
                warnings.push(format!(
                    "Proxy endpoint '{}' references unknown service '{}'.",
                    endpoint.name, service_name
                ));
            }
        }
        nodes.push(TopologyNode {
            id: endpoint_id,
            kind: TopologyNodeKind::ProxyEndpoint,
            label: endpoint.name.clone(),
            subtitle: topology_endpoint_label(endpoint),
            service_name: endpoint.service_name.clone(),
            endpoint_name: Some(endpoint.name.clone()),
            state: None,
            runtime_kind: None,
            ports: vec![format!(
                "{}:{}",
                endpoint.listen_host.trim(),
                endpoint.listen_port
            )],
            url: None,
            command: Some(format!(
                "{}:{} -> {}:{}",
                endpoint.listen_host.trim(),
                endpoint.listen_port,
                endpoint.upstream_host.trim(),
                endpoint.upstream_port
            )),
            workdir: None,
            health_paths: Vec::new(),
            incidents: TopologyIncidentOverlay::default(),
            traffic: TopologyTrafficOverlay::default(),
            resource: TopologyResourceOverlay::default(),
            column: 0,
            row: endpoint_start_row + index,
        });
    }

    let max_column = nodes.iter().map(|node| node.column).max().unwrap_or(0);
    let max_row = nodes.iter().map(|node| node.row).max().unwrap_or(0);
    let active_count = runtime
        .values()
        .filter(|snapshot| {
            matches!(
                snapshot.state,
                ServiceRuntimeState::Running
                    | ServiceRuntimeState::Starting
                    | ServiceRuntimeState::Unhealthy
            )
        })
        .count();
    let incident_count = incident_counts
        .values()
        .map(|counts| counts.warning + counts.critical)
        .sum();
    let traffic_count = traffic_counts.values().map(|counts| counts.total).sum();

    TopologySnapshot {
        project_name: project_name.to_string(),
        project_ip: project.ip.clone(),
        nodes,
        edges,
        warnings,
        service_count: project.services.len(),
        active_count,
        incident_count,
        traffic_count,
        max_column,
        max_row,
    }
}

fn topology_service_depths(
    project: &ProjectConfig,
    service_names: &BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, usize> {
    let by_name = project
        .services
        .iter()
        .map(|service| (service.name.clone(), service))
        .collect::<BTreeMap<_, _>>();
    let mut depths = BTreeMap::new();
    for service in &project.services {
        let mut visiting = BTreeSet::new();
        let depth = topology_service_depth(
            &service.name,
            &by_name,
            service_names,
            &mut depths,
            &mut visiting,
            warnings,
        );
        depths.insert(service.name.clone(), depth);
    }
    depths
}

fn topology_service_depth(
    service_name: &str,
    by_name: &BTreeMap<String, &ServiceConfig>,
    service_names: &BTreeSet<String>,
    depths: &mut BTreeMap<String, usize>,
    visiting: &mut BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> usize {
    if let Some(depth) = depths.get(service_name) {
        return *depth;
    }
    if !visiting.insert(service_name.to_string()) {
        warnings.push(format!(
            "Dependency cycle includes service '{service_name}'. Layout depth was clamped."
        ));
        return 0;
    }
    let Some(service) = by_name.get(service_name).copied() else {
        visiting.remove(service_name);
        return 0;
    };
    let depth = service
        .depends_on
        .iter()
        .filter(|dependency| service_names.contains(*dependency))
        .map(|dependency| {
            topology_service_depth(
                dependency,
                by_name,
                service_names,
                depths,
                visiting,
                warnings,
            ) + 1
        })
        .max()
        .unwrap_or(0);
    visiting.remove(service_name);
    depths.insert(service_name.to_string(), depth);
    depth
}

fn topology_service_rows(
    project: &ProjectConfig,
    depths: &BTreeMap<String, usize>,
) -> BTreeMap<String, usize> {
    let mut next_row_by_depth = BTreeMap::<usize, usize>::new();
    let mut rows = BTreeMap::new();
    for service in &project.services {
        let depth = depths.get(&service.name).copied().unwrap_or(0);
        let next_row = next_row_by_depth.entry(depth).or_insert(0);
        rows.insert(service.name.clone(), *next_row);
        *next_row += 1;
    }
    rows
}

fn topology_traffic_event_failed(event: &ProxyTrafficEvent) -> bool {
    event.error.is_some()
        || event.status_code.is_some_and(|code| code >= 500)
        || event.grpc_status.is_some_and(|status| status != 0)
}

fn topology_resource_overlay(sample: &ServiceResourceSample) -> TopologyResourceOverlay {
    TopologyResourceOverlay {
        cpu_label: format_cpu_percent(sample.cpu_percent),
        memory_label: format_memory_bytes(sample.memory_bytes),
        process_label: sample
            .process_count
            .map(|count| format!("{count} proc"))
            .unwrap_or_else(|| "n/a".to_string()),
    }
}

fn topology_port_labels(ports: &[crate::loopbox::ServicePortConfig]) -> Vec<String> {
    ports
        .iter()
        .map(|port| format!(":{}/{}", port.port, service_protocol_value(&port.protocol)))
        .collect()
}

fn topology_service_subtitle(
    service: &ServiceConfig,
    runtime: Option<&ServiceRuntimeSnapshot>,
) -> String {
    let state = runtime
        .map(|snapshot| runtime_state_label(snapshot.state))
        .unwrap_or("stopped");
    format!("{} · {state}", topology_runtime_kind_label(service.runtime))
}

fn topology_service_command(service: &ServiceConfig) -> String {
    match service.runtime {
        ServiceRuntimeKind::Process => format!("$ {}", service.command),
        ServiceRuntimeKind::Container => service
            .container
            .as_ref()
            .map(|container| format!("image {}", container.image))
            .unwrap_or_else(|| "image <missing>".to_string()),
    }
}

fn topology_service_host(
    project_name: &str,
    service_name: &str,
    config: &LoopboxConfig,
) -> Option<String> {
    let suffix = config.global.domain_suffix.trim();
    if suffix.is_empty() {
        return None;
    }
    Some(format!("{service_name}.{project_name}.{suffix}"))
}

fn topology_traffic_edge_label(counts: &TopologyTrafficOverlay) -> String {
    if counts.failures > 0 {
        format!("{} req · {} failed", counts.total, counts.failures)
    } else if counts.slow > 0 {
        format!("{} req · {} slow", counts.total, counts.slow)
    } else {
        format!("{} req", counts.total)
    }
}

fn topology_endpoint_label(endpoint: &ProxyEndpointConfig) -> String {
    format!(
        "{}:{} · {}",
        endpoint.listen_host.trim(),
        endpoint.listen_port,
        service_protocol_value(&endpoint.protocol)
    )
}

fn topology_endpoint_edge_label(endpoint: &ProxyEndpointConfig) -> String {
    format!(
        "{}:{}",
        endpoint.upstream_host.trim(),
        endpoint.upstream_port
    )
}

fn project_node_id(project_name: &str) -> String {
    format!("project:{project_name}")
}

fn service_node_id(service_name: &str) -> String {
    format!("service:{service_name}")
}

fn endpoint_node_id(endpoint_name: &str) -> String {
    format!("endpoint:{endpoint_name}")
}

fn topology_runtime_kind_label(kind: ServiceRuntimeKind) -> &'static str {
    match kind {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    }
}

fn runtime_state_label(state: ServiceRuntimeState) -> &'static str {
    match state {
        ServiceRuntimeState::Stopped => "stopped",
        ServiceRuntimeState::Starting => "starting",
        ServiceRuntimeState::Running => "running",
        ServiceRuntimeState::Unhealthy => "unhealthy",
        ServiceRuntimeState::Crashed => "crashed",
    }
}

fn render_topology_detail(
    selected_node: Option<TopologyNode>,
    snapshot: &TopologySnapshot,
    summary_text: &str,
    project_name: &str,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut current_page: Signal<Page>,
) -> Element {
    let Some(node) = selected_node else {
        return rsx! {
            aside { class: "topology-detail",
                div { class: "traffic-detail-empty", "Select a topology node." }
            }
        };
    };
    let service_name = node.service_name.clone();
    let can_open = node.kind == TopologyNodeKind::Service && node.url.is_some();
    let can_diagnose = matches!(
        node.kind,
        TopologyNodeKind::Project | TopologyNodeKind::Service
    );
    let node_summary = topology_node_summary(&node, snapshot);
    let project_for_actions = project_name.to_string();
    let summary_text = summary_text.to_string();

    rsx! {
        aside { class: "topology-detail",
            div { class: "topology-detail-head",
                span { class: "topology-node-kind", "{topology_node_kind_label(node.kind)}" }
                h3 { "{node.label}" }
                p { "{node.subtitle}" }
            }
            div { class: "topology-detail-actions",
                button {
                    class: "btn btn-sm btn-outline",
                    onclick: {
                        let node_summary = node_summary.clone();
                        move |_| match copy_to_clipboard(&node_summary) {
                            Ok(()) => notice.set(Some(Notice::success("Copied topology node."))),
                            Err(err) => notice.set(Some(Notice::error(err))),
                        }
                    },
                    "Copy Node"
                }
                button {
                    class: "btn btn-sm btn-outline",
                    onclick: {
                        let summary_text = summary_text.clone();
                        move |_| match copy_to_clipboard(&summary_text) {
                            Ok(()) => notice.set(Some(Notice::success("Copied topology summary."))),
                            Err(err) => notice.set(Some(Notice::error(err))),
                        }
                    },
                    "Copy Map"
                }
                if can_open {
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let service_name = service_name.clone().unwrap_or_default();
                            let project_for_actions = project_for_actions.clone();
                            move |_| match loopbox::open_url_for(
                                &config(),
                                &project_for_actions,
                                OpenTarget::Service(service_name.clone()),
                            ) {
                                Ok(url) => match webbrowser::open(&url) {
                                    Ok(_) => notice.set(Some(Notice::info(format!("Opened {url}")))),
                                    Err(err) => notice.set(Some(Notice::error(format!("Failed: {err}")))),
                                },
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Open"
                    }
                }
                if can_diagnose {
                    button {
                        class: "btn btn-sm btn-primary",
                        onclick: {
                            let service_name = service_name.clone();
                            let project_for_actions = project_for_actions.clone();
                            move |_| {
                                let source = if service_name.is_some() {
                                    loopbox::DiagnosisSource::Service
                                } else {
                                    loopbox::DiagnosisSource::Sandbox
                                };
                                match loopbox::create_diagnosis_session(
                                    &config(),
                                    loopbox::CreateDiagnosisSessionInput {
                                        project_name: project_for_actions.clone(),
                                        service_name: service_name.clone(),
                                        source,
                                        window: "1h".to_string(),
                                        incident_id: None,
                                        title: None,
                                    },
                                ) {
                                    Ok(session) => {
                                        let prompt = loopbox::diagnosis_prompt_for_session(&session);
                                        loopbox::codex_agents_prefill_diagnosis_prompt(session.id, prompt);
                                        current_page.set(Page::Agents);
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        },
                        "Diagnose"
                    }
                }
            }

            div { class: "topology-detail-grid",
                TopologyDetailField { label: "State".to_string(), value: node.state.map(runtime_state_label).unwrap_or("n/a").to_string() }
                TopologyDetailField { label: "Runtime".to_string(), value: node.runtime_kind.map(topology_runtime_kind_label).unwrap_or("n/a").to_string() }
                TopologyDetailField { label: "CPU".to_string(), value: node.resource.cpu_label.clone() }
                TopologyDetailField { label: "Memory".to_string(), value: node.resource.memory_label.clone() }
                TopologyDetailField { label: "Workers".to_string(), value: node.resource.process_label.clone() }
                TopologyDetailField { label: "Traffic".to_string(), value: topology_traffic_detail_label(&node.traffic) }
                TopologyDetailField { label: "Incidents".to_string(), value: topology_incident_detail_label(&node.incidents) }
            }
            if !node.ports.is_empty() {
                section { class: "topology-detail-section",
                    h4 { "Ports" }
                    p { "{node.ports.join(\", \")}" }
                }
            }
            if !node.health_paths.is_empty() {
                section { class: "topology-detail-section",
                    h4 { "Health" }
                    p { "{node.health_paths.join(\", \")}" }
                }
            }
            if let Some(url) = node.url.as_ref() {
                section { class: "topology-detail-section",
                    h4 { "Host" }
                    code { "{url}" }
                }
            }
            if let Some(command) = node.command.as_ref() {
                section { class: "topology-detail-section",
                    h4 { "Execution" }
                    code { "{command}" }
                }
            }
            if let Some(workdir) = node.workdir.as_ref() {
                section { class: "topology-detail-section",
                    h4 { "Workdir" }
                    code { "{workdir}" }
                }
            }
            section { class: "topology-detail-section",
                h4 { "Connected Edges" }
                if topology_connected_edges(snapshot, &node.id).is_empty() {
                    p { "No visible topology connections." }
                } else {
                    ul { class: "topology-edge-list",
                        for edge in topology_connected_edges(snapshot, &node.id) {
                            li { key: "{edge.from}-{edge.to}-{topology_edge_kind_label(edge.kind)}",
                                span { "{topology_edge_kind_label(edge.kind)}" }
                                code { "{edge.from} -> {edge.to}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TopologyDetailField(label: String, value: String) -> Element {
    rsx! {
        div { class: "topology-detail-field",
            span { "{label}" }
            strong { "{value}" }
        }
    }
}

pub(super) fn topology_snapshot_summary(snapshot: &TopologySnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Topology: {} ({})\n",
        snapshot.project_name, snapshot.project_ip
    ));
    out.push_str(&format!(
        "Services: {} total, {} active\n",
        snapshot.service_count, snapshot.active_count
    ));
    out.push_str(&format!(
        "Signals: incidents={}, traffic={}\n",
        snapshot.incident_count, snapshot.traffic_count
    ));
    for node in snapshot
        .nodes
        .iter()
        .filter(|node| node.kind == TopologyNodeKind::Service)
    {
        out.push_str(&format!(
            "- {}: {}; ports {}; incidents warn={} critical={}; traffic total={} failures={} slow={}\n",
            node.label,
            node.state.map(runtime_state_label).unwrap_or("unknown"),
            if node.ports.is_empty() {
                "none".to_string()
            } else {
                node.ports.join(", ")
            },
            node.incidents.warning,
            node.incidents.critical,
            node.traffic.total,
            node.traffic.failures,
            node.traffic.slow
        ));
    }
    if !snapshot.warnings.is_empty() {
        out.push_str("Warnings:\n");
        for warning in &snapshot.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    out
}

fn topology_node_summary(node: &TopologyNode, snapshot: &TopologySnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}: {}\n",
        topology_node_kind_label(node.kind),
        node.label
    ));
    out.push_str(&format!("Project: {}\n", snapshot.project_name));
    out.push_str(&format!(
        "State: {}\n",
        node.state.map(runtime_state_label).unwrap_or("n/a")
    ));
    out.push_str(&format!(
        "Runtime: {}\n",
        node.runtime_kind
            .map(topology_runtime_kind_label)
            .unwrap_or("n/a")
    ));
    out.push_str(&format!(
        "Ports: {}\n",
        if node.ports.is_empty() {
            "none".to_string()
        } else {
            node.ports.join(", ")
        }
    ));
    out.push_str(&format!("CPU: {}\n", node.resource.cpu_label));
    out.push_str(&format!("Memory: {}\n", node.resource.memory_label));
    out.push_str(&format!(
        "Incidents: {}\n",
        topology_incident_detail_label(&node.incidents)
    ));
    out.push_str(&format!(
        "Traffic: {}\n",
        topology_traffic_detail_label(&node.traffic)
    ));
    out
}

fn topology_node_kind_label(kind: TopologyNodeKind) -> &'static str {
    match kind {
        TopologyNodeKind::Project => "project",
        TopologyNodeKind::Service => "service",
        TopologyNodeKind::ProxyEndpoint => "endpoint",
    }
}

fn topology_edge_kind_label(kind: TopologyEdgeKind) -> &'static str {
    match kind {
        TopologyEdgeKind::DependsOn => "dependency",
        TopologyEdgeKind::HttpIngress => "ingress",
        TopologyEdgeKind::ProxyEndpoint => "endpoint",
        TopologyEdgeKind::RecentTraffic => "traffic",
    }
}

fn topology_toggle_class(active: bool) -> &'static str {
    if active {
        "topology-toggle topology-toggle-on"
    } else {
        "topology-toggle"
    }
}

fn topology_edge_visible(
    kind: TopologyEdgeKind,
    show_dependencies: bool,
    show_ingress: bool,
    show_traffic: bool,
) -> bool {
    match kind {
        TopologyEdgeKind::DependsOn => show_dependencies,
        TopologyEdgeKind::HttpIngress | TopologyEdgeKind::ProxyEndpoint => show_ingress,
        TopologyEdgeKind::RecentTraffic => show_traffic,
    }
}

fn topology_canvas_width(snapshot: &TopologySnapshot) -> usize {
    280 + snapshot.max_column.saturating_mul(270)
}

fn topology_canvas_height(snapshot: &TopologySnapshot) -> usize {
    190 + snapshot.max_row.saturating_mul(140)
}

fn topology_node_position(node: &TopologyNode) -> (usize, usize) {
    (
        32 + node.column.saturating_mul(270),
        36 + node.row.saturating_mul(140),
    )
}

fn topology_node_center(node: &TopologyNode) -> (usize, usize) {
    let (left, top) = topology_node_position(node);
    (left + 100, top + 55)
}

fn topology_node_style(node: &TopologyNode) -> String {
    let (left, top) = topology_node_position(node);
    format!("left:{left}px;top:{top}px;")
}

fn topology_edge_points(
    snapshot: &TopologySnapshot,
    edge: &TopologyEdge,
) -> Option<(usize, usize, usize, usize)> {
    let from = snapshot.node(&edge.from)?;
    let to = snapshot.node(&edge.to)?;
    let (x1, y1) = topology_node_center(from);
    let (x2, y2) = topology_node_center(to);
    Some((x1, y1, x2, y2))
}

fn topology_node_class(node: &TopologyNode, selected: bool) -> String {
    let mut classes = vec!["topology-node".to_string()];
    classes.push(match node.kind {
        TopologyNodeKind::Project => "topology-node-project".to_string(),
        TopologyNodeKind::Service => "topology-node-service".to_string(),
        TopologyNodeKind::ProxyEndpoint => "topology-node-endpoint".to_string(),
    });
    if let Some(state) = node.state {
        classes.push(format!("topology-node-{}", runtime_state_label(state)));
    }
    if selected {
        classes.push("topology-node-selected".to_string());
    }
    classes.join(" ")
}

fn topology_edge_class(edge: &TopologyEdge) -> &'static str {
    match edge.kind {
        TopologyEdgeKind::DependsOn => "topology-edge topology-edge-dependency",
        TopologyEdgeKind::HttpIngress => "topology-edge topology-edge-ingress",
        TopologyEdgeKind::ProxyEndpoint => "topology-edge topology-edge-endpoint",
        TopologyEdgeKind::RecentTraffic => {
            if edge.label.contains("failed") {
                "topology-edge topology-edge-traffic topology-edge-warn"
            } else {
                "topology-edge topology-edge-traffic"
            }
        }
    }
}

fn topology_incident_label(incidents: &TopologyIncidentOverlay) -> Option<String> {
    let total = incidents.warning + incidents.critical;
    if total == 0 {
        None
    } else if incidents.critical > 0 {
        Some(format!("{} critical", incidents.critical))
    } else {
        Some(format!("{} warn", incidents.warning))
    }
}

fn topology_node_traffic_label(traffic: &TopologyTrafficOverlay) -> Option<String> {
    if traffic.total == 0 {
        None
    } else if traffic.failures > 0 {
        Some(format!("{} failed", traffic.failures))
    } else if traffic.slow > 0 {
        Some(format!("{} slow", traffic.slow))
    } else {
        Some(format!("{} req", traffic.total))
    }
}

fn topology_incident_detail_label(incidents: &TopologyIncidentOverlay) -> String {
    format!(
        "{} warning, {} critical",
        incidents.warning, incidents.critical
    )
}

fn topology_traffic_detail_label(traffic: &TopologyTrafficOverlay) -> String {
    format!(
        "{} total, {} failed, {} slow",
        traffic.total, traffic.failures, traffic.slow
    )
}

fn topology_connected_edges(snapshot: &TopologySnapshot, node_id: &str) -> Vec<TopologyEdge> {
    snapshot
        .edges
        .iter()
        .filter(|edge| edge.from == node_id || edge.to == node_id)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{
        GlobalConfig, ProxyTrafficEvent, ProxyTrafficHeader, ServicePortConfig,
        ServiceResourceSample, ServiceRuntimeKind,
    };

    fn config(project: ProjectConfig) -> LoopboxConfig {
        LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([("demo".to_string(), project)]),
        }
    }

    fn service(name: &str, ports: Vec<ServicePortConfig>, depends_on: Vec<&str>) -> ServiceConfig {
        ServiceConfig {
            name: name.to_string(),
            runtime: ServiceRuntimeKind::Process,
            container: None,
            ports,
            port: None,
            protocol: ProxyEndpointProtocol::Http1,
            command: format!("npm run {name}"),
            workdir: format!("/repo/{name}"),
            env_files: Vec::new(),
            depends_on: depends_on.into_iter().map(str::to_string).collect(),
            autostart: false,
            health_path: None,
        }
    }

    fn http_port(port: u16) -> ServicePortConfig {
        ServicePortConfig {
            port,
            protocol: ProxyEndpointProtocol::Http1,
            health_path: Some("/health".to_string()),
        }
    }

    fn runtime(service: &str, state: ServiceRuntimeState) -> ServiceRuntimeSnapshot {
        ServiceRuntimeSnapshot {
            project: "demo".to_string(),
            service: service.to_string(),
            state,
            pid: Some(123),
            started_at: Some(1),
            exit_code: None,
            last_error: None,
        }
    }

    fn resource(service: &str) -> ServiceResourceSample {
        ServiceResourceSample {
            project_name: "demo".to_string(),
            service_name: service.to_string(),
            sampled_at_unix_ms: 1_777_000_000_000,
            sampled_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            runtime: ServiceRuntimeKind::Process,
            state: ServiceRuntimeState::Running,
            pid: Some(123),
            cpu_percent: Some(12.5),
            memory_bytes: Some(256 * 1024 * 1024),
            process_count: Some(2),
            container_name: None,
            unavailable_reason: None,
        }
    }

    fn incident(service: &str, severity: IncidentSeverity) -> IncidentTimelineEvent {
        IncidentTimelineEvent {
            id: format!("incident-{service}-{severity:?}"),
            occurred_at_unix_ms: 1_777_000_000_000,
            occurred_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            project_name: "demo".to_string(),
            service_name: Some(service.to_string()),
            severity,
            kind: IncidentKind::RuntimeTransition,
            summary: format!("{service} needs attention"),
            detail: None,
            evidence: Vec::new(),
            source: "test".to_string(),
        }
    }

    fn traffic(service: &str, status: Option<u16>, duration_ms: u64) -> ProxyTrafficEvent {
        ProxyTrafficEvent {
            id: duration_ms,
            started_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            project_name: "demo".to_string(),
            service_name: service.to_string(),
            protocol: "http1".to_string(),
            host: format!("{service}.demo.localhost"),
            method: "GET".to_string(),
            path: "/api".to_string(),
            status_code: status,
            stream_id: None,
            grpc_service: None,
            grpc_method: None,
            grpc_status: None,
            grpc_message: None,
            duration_ms,
            request_bytes: 120,
            response_bytes: 240,
            request_header_bytes: 80,
            request_body_bytes: 0,
            response_header_bytes: 90,
            response_body_bytes: 150,
            request_headers: vec![ProxyTrafficHeader {
                name: "accept".to_string(),
                value: "application/json".to_string(),
            }],
            response_headers: Vec::new(),
            request_body_preview: None,
            response_body_preview: None,
            request_body_truncated: false,
            response_body_truncated: false,
            request_body_binary: false,
            response_body_binary: false,
            error: None,
        }
    }

    #[test]
    fn snapshot_includes_service_runtime_ports_metrics_incidents_and_traffic() {
        let project = ProjectConfig {
            dir: "/repo".to_string(),
            ip: "127.0.0.2".to_string(),
            services: vec![
                service("postgres", vec![http_port(5432)], Vec::new()),
                service("api", vec![http_port(8080)], vec!["postgres"]),
            ],
            default_open_service: Some("api".to_string()),
            proxy_traffic_capture_enabled: None,
            proxy_traffic_capture_mode: None,
            grpc_proto_paths: Vec::new(),
            proxy_endpoints: vec![ProxyEndpointConfig {
                name: "grpc-alias".to_string(),
                listen_host: "127.0.0.1".to_string(),
                listen_port: 50060,
                protocol: ProxyEndpointProtocol::GrpcH2c,
                upstream_host: "127.0.0.2".to_string(),
                upstream_port: 50051,
                authority: Some("api.internal.localhost".to_string()),
                project_name: Some("demo".to_string()),
                service_name: Some("api".to_string()),
            }],
        };
        let cfg = config(project.clone());
        let runtime = BTreeMap::from([
            (
                "postgres".to_string(),
                runtime("postgres", ServiceRuntimeState::Running),
            ),
            (
                "api".to_string(),
                runtime("api", ServiceRuntimeState::Unhealthy),
            ),
        ]);
        let resources = BTreeMap::from([("demo::api".to_string(), resource("api"))]);
        let incidents = vec![
            incident("api", IncidentSeverity::Warning),
            incident("api", IncidentSeverity::Critical),
        ];
        let traffic = vec![
            traffic("api", Some(200), 100),
            traffic("api", Some(503), 1_250),
        ];

        let snapshot = build_topology_snapshot(
            &cfg, "demo", &project, &runtime, &resources, &incidents, &traffic,
        );

        let api = snapshot.node("service:api").expect("api node");
        assert_eq!(api.kind, TopologyNodeKind::Service);
        assert_eq!(api.state, Some(ServiceRuntimeState::Unhealthy));
        assert_eq!(api.runtime_kind, Some(ServiceRuntimeKind::Process));
        assert_eq!(api.ports, vec![":8080/http1"]);
        assert_eq!(api.resource.cpu_label, "12.5%");
        assert_eq!(api.resource.memory_label, "256.0 MB");
        assert_eq!(api.incidents.warning, 1);
        assert_eq!(api.incidents.critical, 1);
        assert_eq!(api.traffic.total, 2);
        assert_eq!(api.traffic.failures, 1);
        assert_eq!(api.traffic.slow, 1);
    }

    #[test]
    fn snapshot_creates_expected_edges_and_warns_for_unknown_targets() {
        let project = ProjectConfig {
            dir: "/repo".to_string(),
            ip: "127.0.0.2".to_string(),
            services: vec![
                service("db", vec![http_port(5432)], Vec::new()),
                service("api", vec![http_port(8080)], vec!["db", "redis"]),
            ],
            default_open_service: None,
            proxy_traffic_capture_enabled: None,
            proxy_traffic_capture_mode: None,
            grpc_proto_paths: Vec::new(),
            proxy_endpoints: vec![ProxyEndpointConfig {
                name: "bad-alias".to_string(),
                listen_host: "127.0.0.1".to_string(),
                listen_port: 15432,
                protocol: ProxyEndpointProtocol::TcpPassthrough,
                upstream_host: "127.0.0.2".to_string(),
                upstream_port: 5432,
                authority: None,
                project_name: Some("demo".to_string()),
                service_name: Some("missing".to_string()),
            }],
        };
        let cfg = config(project.clone());
        let snapshot = build_topology_snapshot(
            &cfg,
            "demo",
            &project,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &[traffic("api", Some(200), 50)],
        );

        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == TopologyEdgeKind::DependsOn
                && edge.from == "service:db"
                && edge.to == "service:api"
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == TopologyEdgeKind::HttpIngress
                && edge.from == "project:demo"
                && edge.to == "service:api"
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == TopologyEdgeKind::RecentTraffic
                && edge.from == "project:demo"
                && edge.to == "service:api"
        }));
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("redis")));
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("bad-alias")));
    }

    #[test]
    fn summary_text_is_concise_and_operational() {
        let project = ProjectConfig {
            dir: "/repo".to_string(),
            ip: "127.0.0.2".to_string(),
            services: vec![service("api", vec![http_port(8080)], Vec::new())],
            default_open_service: None,
            proxy_traffic_capture_enabled: None,
            proxy_traffic_capture_mode: None,
            grpc_proto_paths: Vec::new(),
            proxy_endpoints: Vec::new(),
        };
        let cfg = config(project.clone());
        let snapshot = build_topology_snapshot(
            &cfg,
            "demo",
            &project,
            &BTreeMap::from([(
                "api".to_string(),
                runtime("api", ServiceRuntimeState::Running),
            )]),
            &BTreeMap::new(),
            &[incident("api", IncidentSeverity::Warning)],
            &[traffic("api", Some(503), 1_250)],
        );

        let summary = topology_snapshot_summary(&snapshot);

        assert!(summary.contains("Topology: demo"));
        assert!(summary.contains("api"));
        assert!(summary.contains("running"));
        assert!(summary.contains("incidents warn=1 critical=0"));
        assert!(summary.contains("traffic total=1 failures=1 slow=1"));
    }
}
