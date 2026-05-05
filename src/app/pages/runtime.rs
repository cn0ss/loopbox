use crate::app::log_window::{self, LogWindowConfig};
use crate::app::models::{Notice, Page, RuntimeFilter};
use crate::app::runtime_view::{
    build_runtime_service_row, format_cpu_percent, format_memory_bytes, format_sample_age,
    runtime_row_matches, RuntimeServiceAttachments, RuntimeServiceRow,
};
use crate::app::terminal_window::{self, TerminalWindowConfig};
use crate::app::utils::decode_service_input_sequence;
use crate::loopbox::{
    self, LoopboxConfig, OpenTarget, ServiceRuntimeKind, ServiceRuntimeSnapshot,
    ServiceRuntimeState,
};
use dioxus::prelude::*;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_runtime_page(
    page: Page,
    current_page: Signal<Page>,
    runtime_filter_value: RuntimeFilter,
    mut runtime_filter: Signal<RuntimeFilter>,
    mut runtime_search: Signal<String>,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    let service_key_inputs = use_signal(BTreeMap::<String, String>::new);
    let runtime_snapshot_resource = use_resource(move || {
        let page = current_page();
        let cfg = config();
        let filter = runtime_filter();
        let search = runtime_search();
        let refresh = if runtime_page_uses_live_refresh(page) {
            Some(runtime_tick())
        } else {
            None
        };
        async move {
            let _ = refresh;
            tokio::task::spawn_blocking(move || build_runtime_page_snapshot(cfg, filter, search))
                .await
                .unwrap_or_default()
        }
    });

    if page != Page::Runtime {
        return rsx! {};
    }

    let key_inputs_snapshot = service_key_inputs();
    let search_value = runtime_search();
    let runtime_snapshot = runtime_snapshot_resource();
    let runtime_loading = runtime_snapshot.is_none();
    let RuntimePageSnapshot {
        resource_metrics_enabled,
        docker_warning,
        doctor_issue_count,
        project_count,
        total_rows,
        visible_rows,
        running_total,
        starting_total,
        unhealthy_total,
        crashed_total,
        stopped_total,
        resource_sample_count,
        resource_total_cpu_label,
        resource_total_memory_label,
        hottest_service,
        largest_service,
        latest_sample_label,
        project_rows,
    } = runtime_snapshot.unwrap_or_default();

    let active_total = running_total
        .saturating_add(starting_total)
        .saturating_add(unhealthy_total);
    let attention_total = unhealthy_total.saturating_add(crashed_total);
    let visibility_label = if runtime_loading {
        "Loading".to_string()
    } else if total_rows > 0 {
        format!("{visible_rows}/{total_rows} visible")
    } else {
        "No services".to_string()
    };
    let health_label = if runtime_loading {
        "Checking".to_string()
    } else if attention_total > 0 {
        format!("{attention_total} need attention")
    } else if active_total > 0 {
        "Nominal".to_string()
    } else {
        "Idle".to_string()
    };
    let health_detail = if runtime_loading {
        "Loading runtime status...".to_string()
    } else if total_rows > 0 {
        format!("{running_total} running, {stopped_total} stopped")
    } else {
        format!("{project_count} configured project(s)")
    };
    let sample_detail = if resource_metrics_enabled {
        format!("{resource_sample_count} live sample(s)")
    } else {
        "collection disabled".to_string()
    };
    let health_badge_class = if !runtime_loading && attention_total > 0 {
        "runtime-health-badge runtime-health-badge-warn"
    } else {
        "runtime-health-badge"
    };
    let health_icon = if runtime_loading {
        RuntimeIconKind::Pulse
    } else if attention_total > 0 {
        RuntimeIconKind::Alert
    } else {
        RuntimeIconKind::Running
    };

    rsx! {
        div { class: "page runtime-page",
            div { class: "runtime-hero",
                div { class: "runtime-hero-copy",
                    span { class: "runtime-eyebrow",
                        RuntimeIcon { kind: RuntimeIconKind::Pulse }
                        "Live runtime"
                    }
                    h1 { class: "runtime-hero-title", "Runtime" }
                    div { class: "runtime-hero-status",
                        span { class: "runtime-visibility-badge",
                            RuntimeIcon { kind: RuntimeIconKind::Filter }
                            "{visibility_label}"
                        }
                        span { class: "{health_badge_class}",
                            RuntimeIcon { kind: health_icon }
                            "{health_label}"
                        }
                    }
                    p { class: "runtime-hero-subtitle", "{health_detail}" }
                }

                div { class: "runtime-hero-tools",
                    div { class: "runtime-search-shell",
                        RuntimeIcon { kind: RuntimeIconKind::Search }
                        input {
                            class: "runtime-search-input",
                            r#type: "text",
                            value: "{search_value}",
                            placeholder: "Search service, command, port...",
                            oninput: move |evt| runtime_search.set(evt.value()),
                        }
                    }
                }
            }

            div { class: "runtime-alert-stack",
                if let Some(message) = docker_warning {
                    div { class: "runtime-alert runtime-alert-warn",
                        RuntimeIcon { kind: RuntimeIconKind::Docker }
                        span { class: "runtime-alert-label", "Docker" }
                        span { "{message}" }
                    }
                }
                if doctor_issue_count > 0 {
                    div { class: "runtime-alert",
                        RuntimeIcon { kind: RuntimeIconKind::Doctor }
                        span { class: "runtime-alert-label", "Doctor" }
                        span { "{doctor_issue_count} setup issue(s) may affect service routing or health." }
                    }
                }
                if !resource_metrics_enabled {
                    div { class: "runtime-alert",
                        RuntimeIcon { kind: RuntimeIconKind::Metrics }
                        span { class: "runtime-alert-label", "Metrics" }
                        span { "Resource metrics collection is disabled in Settings." }
                    }
                } else if total_rows > 0 && resource_sample_count == 0 {
                    div { class: "runtime-alert",
                        RuntimeIcon { kind: RuntimeIconKind::Metrics }
                        span { class: "runtime-alert-label", "Metrics" }
                        span { "No active service resource samples are available yet." }
                    }
                }
            }

            if total_rows > 0 {
                div { class: "runtime-dashboard-grid",
                    RuntimeStatTile {
                        kind: RuntimeIconKind::Cpu,
                        label: "CPU".to_string(),
                        value: resource_total_cpu_label.clone(),
                        detail: sample_detail.clone(),
                    }
                    RuntimeStatTile {
                        kind: RuntimeIconKind::Memory,
                        label: "Memory".to_string(),
                        value: resource_total_memory_label.clone(),
                        detail: sample_detail.clone(),
                    }
                    RuntimeStatTile {
                        kind: RuntimeIconKind::Hotspot,
                        label: "Hottest".to_string(),
                        value: hottest_service.clone(),
                        detail: "highest CPU".to_string(),
                    }
                    RuntimeStatTile {
                        kind: RuntimeIconKind::Resource,
                        label: "Largest".to_string(),
                        value: largest_service.clone(),
                        detail: "highest memory".to_string(),
                    }
                    RuntimeStatTile {
                        kind: RuntimeIconKind::Sample,
                        label: "Last sample".to_string(),
                        value: latest_sample_label.clone(),
                        detail: "freshness".to_string(),
                    }
                }

                div { class: "runtime-status-dock",
                    RuntimeStatusPill {
                        kind: RuntimeIconKind::Running,
                        label: "Running".to_string(),
                        count: running_total,
                        tone: "success".to_string(),
                    }
                    RuntimeStatusPill {
                        kind: RuntimeIconKind::Starting,
                        label: "Starting".to_string(),
                        count: starting_total,
                        tone: "warn".to_string(),
                    }
                    RuntimeStatusPill {
                        kind: RuntimeIconKind::Alert,
                        label: "Unhealthy".to_string(),
                        count: unhealthy_total,
                        tone: "danger".to_string(),
                    }
                    RuntimeStatusPill {
                        kind: RuntimeIconKind::Alert,
                        label: "Crashed".to_string(),
                        count: crashed_total,
                        tone: "danger".to_string(),
                    }
                    RuntimeStatusPill {
                        kind: RuntimeIconKind::Stopped,
                        label: "Stopped".to_string(),
                        count: stopped_total,
                        tone: "muted".to_string(),
                    }
                }

                div { class: "runtime-filter-shell",
                    div { class: "runtime-filter-heading",
                        RuntimeIcon { kind: RuntimeIconKind::Filter }
                        span { "Filter" }
                    }
                    div { class: "filter-bar runtime-filter-bar",
                        for filter in [
                            RuntimeFilter::All,
                            RuntimeFilter::Running,
                            RuntimeFilter::Stopped,
                            RuntimeFilter::Unhealthy,
                            RuntimeFilter::Crashed,
                            RuntimeFilter::Containers,
                            RuntimeFilter::Processes,
                        ] {
                            button {
                                key: "{runtime_filter_label(filter)}",
                                class: if runtime_filter_value == filter { "filter-btn active" } else { "filter-btn" },
                                onclick: move |_| runtime_filter.set(filter),
                                "{runtime_filter_label(filter)}"
                            }
                        }
                    }
                }
            }

            for (project_name, rows) in &project_rows {
                section { class: "runtime-project-section", key: "{project_name}",
                    div { class: "runtime-project-topline",
                        div { class: "runtime-project-heading",
                            span { class: "runtime-project-mark",
                                RuntimeIcon { kind: RuntimeIconKind::Project }
                            }
                            div {
                                h2 { "{project_name}" }
                                span { class: "runtime-project-meta", "{rows.len()} visible service(s)" }
                            }
                        }
                        div { class: "panel-actions runtime-project-actions",
                            button {
                                class: "btn btn-outline btn-sm runtime-action-btn",
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        match loopbox::start_project_all(&config(), &pn) {
                                            Ok(results) => notice.set(Some(Notice::success(format!(
                                                "Started {} service(s) in '{pn}'.",
                                                results.len()
                                            )))),
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                RuntimeIcon { kind: RuntimeIconKind::Start }
                                "Start All"
                            }
                            button {
                                class: "btn btn-outline btn-sm runtime-action-btn",
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        match loopbox::stop_project_all(&config(), &pn) {
                                            Ok(results) => notice.set(Some(Notice::info(format!(
                                                "Stopped {} service(s) in '{pn}'.",
                                                results.len()
                                            )))),
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                RuntimeIcon { kind: RuntimeIconKind::Stop }
                                "Stop All"
                            }
                        }
                    }

                    div { class: "runtime-service-grid",
                        for row in rows {
                            {{
                                let row = row.clone();
                                let runtime_key = format!("{}::{}", row.project_name, row.service_name);
                                let key_input_value = key_inputs_snapshot
                                    .get(&runtime_key)
                                    .cloned()
                                    .unwrap_or_default();
                                let inline_error = row
                                    .snapshot
                                    .last_error
                                    .clone()
                                    .filter(|_| row.snapshot.pid.is_some() || row.snapshot.exit_code.is_some());
                                let service_card_class = format!("runtime-service-card {}", row.status_class);
                                let state_pill_class = format!("runtime-state-pill {}", row.status_class);
                                let runtime_icon = runtime_kind_icon(row.runtime_kind);
                                rsx! {
                                    article { class: "{service_card_class}", key: "{runtime_key}",
                                        div { class: "runtime-service-card-head",
                                            div { class: "runtime-service-nameblock",
                                                span { class: "runtime-state-beacon" }
                                                div {
                                                    h3 { class: "runtime-svc-name", "{row.service_name}" }
                                                    div { class: "runtime-service-tags",
                                                        span { class: "runtime-kind-badge",
                                                            RuntimeIcon { kind: runtime_icon }
                                                            "{row.runtime_label}"
                                                        }
                                                        if row.log_attached {
                                                            span { class: "runtime-mini-badge", "logs" }
                                                        }
                                                        if row.input_attached {
                                                            span { class: "runtime-mini-badge runtime-mini-badge-live", "input" }
                                                        }
                                                    }
                                                }
                                            }
                                            span { class: "{state_pill_class}",
                                                "{row.status_label}"
                                            }
                                        }

                                        div { class: "runtime-service-facts",
                                            div { class: "runtime-service-fact",
                                                span { class: "runtime-fact-label",
                                                    RuntimeIcon { kind: RuntimeIconKind::Endpoint }
                                                    "Endpoint"
                                                }
                                                span { class: "runtime-mono runtime-fact-value", "{row.port_label}" }
                                                if row.ports.is_empty() {
                                                    span { class: "runtime-inline-hint", "not routable" }
                                                }
                                            }
                                            div { class: "runtime-service-fact runtime-service-fact-wide",
                                                span { class: "runtime-fact-label",
                                                    RuntimeIcon { kind: RuntimeIconKind::Command }
                                                    "Execution"
                                                }
                                                span { class: "runtime-command-text", "{row.execution_label}" }
                                                span { class: "runtime-workdir", "{row.service.workdir}" }
                                            }
                                            div { class: "runtime-service-fact",
                                                span { class: "runtime-fact-label",
                                                    RuntimeIcon { kind: RuntimeIconKind::Resource }
                                                    "Resources"
                                                }
                                                if !resource_metrics_enabled {
                                                    span { class: "runtime-inline-hint", "disabled" }
                                                } else if let Some(sample) = row.resources.as_ref() {
                                                    if let Some(reason) = sample.unavailable_reason.as_ref() {
                                                        span { class: "runtime-inline-error", "{reason}" }
                                                    } else {
                                                        span { class: "runtime-resource-primary",
                                                            "{format_cpu_percent(sample.cpu_percent)}"
                                                        }
                                                        span { class: "runtime-resource-secondary",
                                                            "{format_memory_bytes(sample.memory_bytes)}"
                                                        }
                                                        if let Some(count) = sample.process_count {
                                                            span { class: "runtime-inline-hint", "{count} proc" }
                                                        }
                                                    }
                                                } else {
                                                    span { class: "runtime-inline-hint", "not sampled" }
                                                }
                                            }
                                            div { class: "runtime-service-fact",
                                                span { class: "runtime-fact-label",
                                                    RuntimeIcon { kind: RuntimeIconKind::State }
                                                    "State"
                                                }
                                                span { class: "{state_pill_class}",
                                                    "{row.status_label}"
                                                }
                                                if let Some(error) = inline_error {
                                                    span { class: "runtime-inline-error", "{error}" }
                                                }
                                            }
                                        }

                                        div { class: "runtime-card-footer",
                                            RuntimeServiceActions {
                                                row: row.clone(),
                                                input_value: key_input_value,
                                                service_key: runtime_key.clone(),
                                                config,
                                                notice,
                                                service_key_inputs,
                                            }
                                        }
                                    }
                                }
                            }}
                        }
                    }
                }
            }

            if runtime_loading && total_rows == 0 {
                div { class: "empty-state runtime-empty-state",
                    RuntimeIcon { kind: RuntimeIconKind::Pulse }
                    p { class: "empty-state-text", "Loading runtime status..." }
                }
            } else if project_count == 0 {
                div { class: "empty-state runtime-empty-state",
                    RuntimeIcon { kind: RuntimeIconKind::Project }
                    p { class: "empty-state-text", "No sandboxes configured yet." }
                }
            } else if total_rows > 0 && visible_rows == 0 {
                div { class: "empty-state runtime-empty-state",
                    RuntimeIcon { kind: RuntimeIconKind::Search }
                    p { class: "empty-state-text", "No services match the current Runtime filters." }
                }
            }
        }
    }
}

#[derive(Clone)]
struct RuntimePageSnapshot {
    resource_metrics_enabled: bool,
    docker_warning: Option<String>,
    doctor_issue_count: usize,
    project_count: usize,
    total_rows: usize,
    visible_rows: usize,
    running_total: usize,
    starting_total: usize,
    unhealthy_total: usize,
    crashed_total: usize,
    stopped_total: usize,
    resource_sample_count: usize,
    resource_total_cpu_label: String,
    resource_total_memory_label: String,
    hottest_service: String,
    largest_service: String,
    latest_sample_label: String,
    project_rows: BTreeMap<String, Vec<RuntimeServiceRow>>,
}

impl Default for RuntimePageSnapshot {
    fn default() -> Self {
        Self {
            resource_metrics_enabled: true,
            docker_warning: None,
            doctor_issue_count: 0,
            project_count: 0,
            total_rows: 0,
            visible_rows: 0,
            running_total: 0,
            starting_total: 0,
            unhealthy_total: 0,
            crashed_total: 0,
            stopped_total: 0,
            resource_sample_count: 0,
            resource_total_cpu_label: "n/a".to_string(),
            resource_total_memory_label: "n/a".to_string(),
            hottest_service: "n/a".to_string(),
            largest_service: "n/a".to_string(),
            latest_sample_label: "n/a".to_string(),
            project_rows: BTreeMap::new(),
        }
    }
}

fn build_runtime_page_snapshot(
    config_snapshot: LoopboxConfig,
    runtime_filter_value: RuntimeFilter,
    search_value: String,
) -> RuntimePageSnapshot {
    let resource_metrics_enabled = config_snapshot.global.resource_metrics.enabled;
    let latest_resources =
        loopbox::resource_metrics_latest_for_config(&config_snapshot).unwrap_or_default();
    let docker_warning = loopbox::docker_runtime_unavailable_message();
    let doctor_issue_count = loopbox::doctor_report(&config_snapshot)
        .iter()
        .filter(|issue| issue.level != loopbox::DoctorLevel::Info)
        .count();

    let mut total_rows = 0_usize;
    let mut visible_rows = 0_usize;
    let mut running_total = 0_usize;
    let mut starting_total = 0_usize;
    let mut unhealthy_total = 0_usize;
    let mut crashed_total = 0_usize;
    let mut stopped_total = 0_usize;
    let mut project_rows = BTreeMap::<String, Vec<RuntimeServiceRow>>::new();

    for (project_name, project) in &config_snapshot.projects {
        let mut rows = Vec::new();
        for service in &project.services {
            total_rows = total_rows.saturating_add(1);
            let snapshot =
                loopbox::service_runtime_status(&config_snapshot, project_name, &service.name)
                    .unwrap_or_else(|err| ServiceRuntimeSnapshot {
                        project: project_name.clone(),
                        service: service.name.clone(),
                        state: ServiceRuntimeState::Crashed,
                        pid: None,
                        started_at: None,
                        exit_code: None,
                        last_error: Some(err),
                    });

            match snapshot.state {
                ServiceRuntimeState::Running => running_total += 1,
                ServiceRuntimeState::Starting => starting_total += 1,
                ServiceRuntimeState::Unhealthy => unhealthy_total += 1,
                ServiceRuntimeState::Crashed => crashed_total += 1,
                ServiceRuntimeState::Stopped => stopped_total += 1,
            }

            let log_attached =
                loopbox::service_log_attached(project_name, &service.name).unwrap_or(false);
            let input_attached =
                loopbox::service_input_attached(project_name, &service.name).unwrap_or(false);
            let terminal_attached =
                loopbox::service_terminal_attached(project_name, &service.name).unwrap_or(false);
            let resources = if matches!(
                snapshot.state,
                ServiceRuntimeState::Starting
                    | ServiceRuntimeState::Running
                    | ServiceRuntimeState::Unhealthy
            ) {
                latest_resources
                    .get(&format!("{project_name}::{}", service.name))
                    .cloned()
            } else {
                None
            };
            let row = build_runtime_service_row(
                project_name,
                &project.ip,
                service,
                snapshot,
                RuntimeServiceAttachments {
                    log_attached,
                    input_attached,
                    terminal_attached,
                    resources,
                },
            );
            if runtime_row_matches(&row, runtime_filter_value, &search_value) {
                visible_rows = visible_rows.saturating_add(1);
                rows.push(row);
            }
        }
        if !rows.is_empty() {
            project_rows.insert(project_name.clone(), rows);
        }
    }

    let resource_rows = project_rows
        .values()
        .flat_map(|rows| rows.iter())
        .filter_map(|row| row.resources.as_ref())
        .collect::<Vec<_>>();
    let resource_total_cpu = resource_rows
        .iter()
        .filter_map(|sample| sample.cpu_percent)
        .sum::<f64>();
    let resource_total_memory = resource_rows
        .iter()
        .filter_map(|sample| sample.memory_bytes)
        .sum::<u64>();
    let resource_total_cpu_label = if resource_rows
        .iter()
        .any(|sample| sample.cpu_percent.is_some())
    {
        format_cpu_percent(Some(resource_total_cpu))
    } else {
        "n/a".to_string()
    };
    let resource_total_memory_label = if resource_rows
        .iter()
        .any(|sample| sample.memory_bytes.is_some())
    {
        format_memory_bytes(Some(resource_total_memory))
    } else {
        "n/a".to_string()
    };
    let hottest_service = resource_rows
        .iter()
        .filter_map(|sample| sample.cpu_percent.map(|cpu| (sample, cpu)))
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(sample, cpu)| {
            format!(
                "{} / {}",
                sample.service_name,
                format_cpu_percent(Some(cpu))
            )
        })
        .unwrap_or_else(|| "n/a".to_string());
    let largest_service = resource_rows
        .iter()
        .filter_map(|sample| sample.memory_bytes.map(|memory| (sample, memory)))
        .max_by_key(|(_, memory)| *memory)
        .map(|(sample, memory)| {
            format!(
                "{} / {}",
                sample.service_name,
                format_memory_bytes(Some(memory))
            )
        })
        .unwrap_or_else(|| "n/a".to_string());
    let latest_sample = resource_rows
        .iter()
        .max_by_key(|sample| sample.sampled_at_unix_ms)
        .copied();
    let resource_total_cpu_label = if !resource_metrics_enabled {
        "disabled".to_string()
    } else {
        resource_total_cpu_label
    };
    let resource_total_memory_label = if !resource_metrics_enabled {
        "disabled".to_string()
    } else {
        resource_total_memory_label
    };
    let hottest_service = if !resource_metrics_enabled {
        "disabled".to_string()
    } else {
        hottest_service
    };
    let largest_service = if !resource_metrics_enabled {
        "disabled".to_string()
    } else {
        largest_service
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let latest_sample_label = if !resource_metrics_enabled {
        "disabled".to_string()
    } else {
        format_sample_age(now_ms, latest_sample)
    };

    RuntimePageSnapshot {
        resource_metrics_enabled,
        docker_warning,
        doctor_issue_count,
        project_count: config_snapshot.projects.len(),
        total_rows,
        visible_rows,
        running_total,
        starting_total,
        unhealthy_total,
        crashed_total,
        stopped_total,
        resource_sample_count: resource_rows.len(),
        resource_total_cpu_label,
        resource_total_memory_label,
        hottest_service,
        largest_service,
        latest_sample_label,
        project_rows,
    }
}

fn runtime_page_uses_live_refresh(page: Page) -> bool {
    page == Page::Runtime
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeIconKind {
    Alert,
    Attach,
    Command,
    Container,
    Cpu,
    Docker,
    Doctor,
    Endpoint,
    Filter,
    Hotspot,
    Logs,
    Memory,
    Metrics,
    Open,
    Process,
    Project,
    Pulse,
    Resource,
    Restart,
    Run,
    Running,
    Sample,
    Search,
    Send,
    Start,
    Starting,
    State,
    Stop,
    Stopped,
    Terminal,
}

impl RuntimeIconKind {
    fn css_suffix(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Attach => "attach",
            Self::Command => "command",
            Self::Container => "container",
            Self::Cpu => "cpu",
            Self::Docker => "docker",
            Self::Doctor => "doctor",
            Self::Endpoint => "endpoint",
            Self::Filter => "filter",
            Self::Hotspot => "hotspot",
            Self::Logs => "logs",
            Self::Memory => "memory",
            Self::Metrics => "metrics",
            Self::Open => "open",
            Self::Process => "process",
            Self::Project => "project",
            Self::Pulse => "pulse",
            Self::Resource => "resource",
            Self::Restart => "restart",
            Self::Run => "run",
            Self::Running => "running",
            Self::Sample => "sample",
            Self::Search => "search",
            Self::Send => "send",
            Self::Start => "start",
            Self::Starting => "starting",
            Self::State => "state",
            Self::Stop => "stop",
            Self::Stopped => "stopped",
            Self::Terminal => "terminal",
        }
    }

    fn path_data(self) -> &'static str {
        match self {
            Self::Alert => "M12 8v5m0 4h.01M10.3 3.9 2.8 17a2 2 0 0 0 1.7 3h15a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z",
            Self::Attach => "M10 13a5 5 0 0 0 7.1 0l2-2a5 5 0 0 0-7.1-7.1l-1.1 1.1M14 11a5 5 0 0 0-7.1 0l-2 2a5 5 0 0 0 7.1 7.1l1.1-1.1",
            Self::Command => "M4 7h16M7 7v10m10-10v10M4 17h16M9 11l2 2-2 2m5 0h3",
            Self::Container => "M3.5 8.5 12 4l8.5 4.5-8.5 4.5-8.5-4.5ZM3.5 8.5v7L12 20l8.5-4.5v-7M12 13v7",
            Self::Cpu => "M8 3v3m4-3v3m4-3v3M8 18v3m4-3v3m4-3v3M3 8h3m-3 4h3m-3 4h3m12-8h3m-3 4h3m-3 4h3M8 8h8v8H8z",
            Self::Docker => "M4 14h13.5c1.6 0 2.8-.7 3.5-2.1-.8.1-1.5 0-2.1-.4-.4-1.4-1.4-2.3-3-2.8-.2 1.2.1 2.2.8 3H15M5 10h3v3H5zm4 0h3v3H9zm4 0h3v3h-3zM9 6h3v3H9z",
            Self::Doctor => "M12 21s7-4.1 7-10V5l-7-3-7 3v6c0 5.9 7 10 7 10Zm-3-10h6m-3-3v6",
            Self::Endpoint => "M5 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm14 6a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM8 9h3.5a3.5 3.5 0 0 1 3.5 3.5V15",
            Self::Filter => "M4 5h16l-6 7v5l-4 2v-7L4 5Z",
            Self::Hotspot => "M12 22c3.4-1.4 6-4.2 6-7.7 0-2.8-1.6-5.2-4.6-7.3.2 2.2-.6 3.8-2.4 4.9.4-3-1.1-5.6-4.1-7.9.2 3.2-.6 5.6-2.3 7.4A6 6 0 0 0 12 22Z",
            Self::Logs => "M5 5h14M5 9h14M5 13h10M5 17h8",
            Self::Memory => "M5 7h14v10H5zM8 4v3m4-3v3m4-3v3M8 17v3m4-3v3m4-3v3M2 10h3m-3 4h3m14-4h3m-3 4h3",
            Self::Metrics => "M4 19V5m0 14h16M8 15l3-4 3 2 4-7",
            Self::Open => "M14 4h6v6M20 4l-9 9M19 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4",
            Self::Process => "M4 6h16v12H4zM7 10h4m-4 4h7m4-4h.01m0 4h.01",
            Self::Project => "M4 6.5 12 3l8 3.5-8 3.5-8-3.5Zm0 5 8 3.5 8-3.5M4 16.5l8 3.5 8-3.5",
            Self::Pulse => "M3 12h4l2-7 5 14 2-7h5",
            Self::Resource => "M4 17V7m4 10v-6m4 6V4m4 13v-9m4 9v-3",
            Self::Restart => "M20 12a8 8 0 1 1-2.3-5.7M20 4v6h-6",
            Self::Run => "M5 5h14v14H5zM10 8l6 4-6 4V8Z",
            Self::Running => "M20 6 9 17l-5-5",
            Self::Sample => "M12 6v6l4 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z",
            Self::Search => "M10.8 18.2a7.4 7.4 0 1 0 0-14.8 7.4 7.4 0 0 0 0 14.8ZM16 16l5 5",
            Self::Send => "M4 12h14M13 6l6 6-6 6",
            Self::Start => "M8 5v14l11-7L8 5Z",
            Self::Starting => "M12 3v4m0 10v4m9-9h-4M7 12H3m15.4-6.4-2.8 2.8M8.4 15.6l-2.8 2.8m12.8 0-2.8-2.8M8.4 8.4 5.6 5.6",
            Self::State => "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm-4-9 2.5 2.5L16 9",
            Self::Stop => "M7 7h10v10H7z",
            Self::Stopped => "M7 12h10",
            Self::Terminal => "M4 5h16v14H4zM7 10l3 2-3 2m5 1h5",
        }
    }
}

fn runtime_kind_icon(kind: ServiceRuntimeKind) -> RuntimeIconKind {
    match kind {
        ServiceRuntimeKind::Process => RuntimeIconKind::Process,
        ServiceRuntimeKind::Container => RuntimeIconKind::Container,
    }
}

#[component]
fn RuntimeIcon(kind: RuntimeIconKind) -> Element {
    let icon_class = format!("runtime-icon runtime-icon-{}", kind.css_suffix());
    let path_data = kind.path_data();

    rsx! {
        svg {
            class: "{icon_class}",
            fill: "none",
            stroke: "currentColor",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            stroke_width: "1.8",
            view_box: "0 0 24 24",
            xmlns: "http://www.w3.org/2000/svg",
            path { d: "{path_data}" }
        }
    }
}

#[component]
fn RuntimeStatTile(kind: RuntimeIconKind, label: String, value: String, detail: String) -> Element {
    let class_name = format!("runtime-stat-tile runtime-stat-{}", kind.css_suffix());

    rsx! {
        div { class: "{class_name}",
            div { class: "runtime-stat-topline",
                span { class: "runtime-stat-icon",
                    RuntimeIcon { kind }
                }
                span { class: "runtime-stat-label", "{label}" }
            }
            strong { "{value}" }
            span { class: "runtime-stat-detail", "{detail}" }
        }
    }
}

#[component]
fn RuntimeStatusPill(kind: RuntimeIconKind, label: String, count: usize, tone: String) -> Element {
    let class_name = format!("runtime-status-pill runtime-status-{tone}");

    rsx! {
        div { class: "{class_name}",
            RuntimeIcon { kind }
            span { class: "runtime-status-label", "{label}" }
            strong { class: "runtime-status-count", "{count}" }
        }
    }
}

#[component]
fn RuntimeServiceActions(
    row: RuntimeServiceRow,
    input_value: String,
    service_key: String,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut service_key_inputs: Signal<BTreeMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "runtime-action-group",
            if row.can_start {
                button {
                    class: "btn btn-sm btn-primary runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::start_service(&config(), &pn, &svc) {
                                Ok(_) => notice.set(Some(Notice::success(format!("Started '{svc}'.")))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Start }
                    "Start"
                }
            }
            if row.can_stop {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::stop_service(&pn, &svc) {
                                Ok(_) => notice.set(Some(Notice::info(format!("Stopped '{svc}'.")))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Stop }
                    "Stop"
                }
            }
            if row.can_restart {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::restart_service(&config(), &pn, &svc) {
                                Ok(_) => notice.set(Some(Notice::success(format!("Restarted '{svc}'.")))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Restart }
                    "Restart"
                }
            }
            if row.can_open {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::open_url_for(&config(), &pn, OpenTarget::Service(svc.clone())) {
                                Ok(url) => match webbrowser::open(&url) {
                                    Ok(_) => notice.set(Some(Notice::info(format!("Opened {url}")))),
                                    Err(err) => notice.set(Some(Notice::error(format!("Failed: {err}")))),
                                },
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Open }
                    "Open"
                }
            }
            button {
                class: "btn btn-sm btn-outline runtime-action-btn",
                onclick: {
                    let pn = row.project_name.clone();
                    let svc = row.service_name.clone();
                    move |_| {
                        let title = format!("Logs — {svc} ({pn})");
                        log_window::push_config(LogWindowConfig {
                            project_name: pn.clone(),
                            service_name: svc.clone(),
                        });
                        let dom = VirtualDom::new(log_window::LogPopoutWindow);
                        let cfg = dioxus::desktop::Config::new().with_window(
                            dioxus::desktop::WindowBuilder::new()
                                .with_title(title)
                                .with_inner_size(dioxus::desktop::LogicalSize::new(900.0, 600.0)),
                        );
                        dioxus::desktop::window().new_window(dom, cfg);
                    }
                },
                RuntimeIcon { kind: RuntimeIconKind::Logs }
                "Logs"
            }
            if row.can_terminal {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        let terminal_attached = row.terminal_attached;
                        move |_| {
                            if terminal_attached {
                                let title = format!("Terminal — {svc} ({pn})");
                                terminal_window::push_config(TerminalWindowConfig {
                                    project_name: pn.clone(),
                                    service_name: svc.clone(),
                                });
                                let dom = VirtualDom::new(terminal_window::TerminalPopoutWindow);
                                let cfg = dioxus::desktop::Config::new().with_window(
                                    dioxus::desktop::WindowBuilder::new()
                                        .with_title(title)
                                        .with_inner_size(dioxus::desktop::LogicalSize::new(940.0, 620.0)),
                                );
                                dioxus::desktop::window().new_window(dom, cfg);
                            } else {
                                match loopbox::open_terminal_for_service(&config(), &pn, &svc, false) {
                                    Ok(msg) => notice.set(Some(Notice::info(msg))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Terminal }
                    "Terminal"
                }
            }
            if row.can_run {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::open_terminal_for_service(&config(), &pn, &svc, true) {
                                Ok(msg) => notice.set(Some(Notice::success(msg))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Run }
                    "Run"
                }
            }
            if row.can_attach {
                button {
                    class: "btn btn-sm btn-outline runtime-action-btn",
                    onclick: {
                        let pn = row.project_name.clone();
                        let svc = row.service_name.clone();
                        move |_| {
                            match loopbox::open_terminal_attach_for_service(&pn, &svc) {
                                Ok(msg) => notice.set(Some(Notice::info(msg))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        }
                    },
                    RuntimeIcon { kind: RuntimeIconKind::Attach }
                    "Attach"
                }
            }
            if row.can_send_input {
                div { class: "runtime-input-control",
                    input {
                        class: "field-input runtime-key-input",
                        r#type: "text",
                        value: "{input_value}",
                        placeholder: "r, \\n, \\x12",
                        oninput: {
                            let key = service_key.clone();
                            move |evt| {
                                let value = evt.value();
                                service_key_inputs.with_mut(|inputs| {
                                    if value.is_empty() {
                                        inputs.remove(&key);
                                    } else {
                                        inputs.insert(key.clone(), value);
                                    }
                                });
                            }
                        },
                    }
                    button {
                        class: "btn btn-sm btn-outline runtime-action-btn",
                        onclick: {
                            let pn = row.project_name.clone();
                            let svc = row.service_name.clone();
                            let key = service_key.clone();
                            move |_| {
                                let raw_input = service_key_inputs
                                    .read()
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                if raw_input.is_empty() {
                                    notice.set(Some(Notice::error("Enter one or more keys to send.".to_string())));
                                    return;
                                }
                                let decoded = match decode_service_input_sequence(&raw_input) {
                                    Ok(value) => value,
                                    Err(err) => {
                                        notice.set(Some(Notice::error(err)));
                                        return;
                                    }
                                };
                                match loopbox::send_service_input(&pn, &svc, &decoded) {
                                    Ok(()) => notice.set(Some(Notice::info(format!("Sent key input to '{svc}'.")))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        },
                        RuntimeIcon { kind: RuntimeIconKind::Send }
                        "Send"
                    }
                }
            }
            if row.runtime_label == "container" {
                span { class: "runtime-action-note", "Docker managed" }
            } else if !row.can_send_input && matches!(
                row.snapshot.state,
                ServiceRuntimeState::Running | ServiceRuntimeState::Starting | ServiceRuntimeState::Unhealthy
            ) {
                if row.terminal_attached {
                    span { class: "runtime-action-note", "terminal attached" }
                } else {
                    span { class: "runtime-action-note", "restart for terminal" }
                }
            }
        }
    }
}

fn runtime_filter_label(filter: RuntimeFilter) -> &'static str {
    match filter {
        RuntimeFilter::All => "All",
        RuntimeFilter::Running => "Running",
        RuntimeFilter::Stopped => "Stopped",
        RuntimeFilter::Unhealthy => "Unhealthy",
        RuntimeFilter::Crashed => "Crashed",
        RuntimeFilter::Containers => "Containers",
        RuntimeFilter::Processes => "Processes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime_view::{runtime_service_action_flags, runtime_status_summary};
    use crate::loopbox::{
        ContainerServiceConfig, ProxyEndpointProtocol, ServiceConfig, ServicePortConfig,
        ServiceRuntimeKind, ServiceRuntimeSnapshot,
    };

    fn process_service() -> ServiceConfig {
        ServiceConfig {
            name: "web".to_string(),
            runtime: ServiceRuntimeKind::Process,
            container: None,
            ports: vec![ServicePortConfig {
                port: 5173,
                protocol: ProxyEndpointProtocol::Http1,
                health_path: None,
            }],
            port: Some(5173),
            protocol: ProxyEndpointProtocol::Http1,
            command: "pnpm dev".to_string(),
            workdir: "/tmp/app".to_string(),
            env_files: Vec::new(),
            depends_on: Vec::new(),
            autostart: false,
            health_path: None,
        }
    }

    fn container_service() -> ServiceConfig {
        ServiceConfig {
            name: "db".to_string(),
            runtime: ServiceRuntimeKind::Container,
            container: Some(ContainerServiceConfig {
                image: "postgres:16-alpine".to_string(),
                args: Vec::new(),
                env: Vec::new(),
                volumes: Vec::new(),
                auto_remove: true,
            }),
            ports: vec![ServicePortConfig {
                port: 5432,
                protocol: ProxyEndpointProtocol::TcpPassthrough,
                health_path: None,
            }],
            port: Some(5432),
            protocol: ProxyEndpointProtocol::TcpPassthrough,
            command: String::new(),
            workdir: "/tmp/app".to_string(),
            env_files: Vec::new(),
            depends_on: Vec::new(),
            autostart: false,
            health_path: None,
        }
    }

    fn snapshot(service: &str, state: ServiceRuntimeState) -> ServiceRuntimeSnapshot {
        ServiceRuntimeSnapshot {
            project: "demo".to_string(),
            service: service.to_string(),
            state,
            pid: None,
            started_at: None,
            exit_code: None,
            last_error: None,
        }
    }

    #[test]
    fn process_rows_expose_terminal_and_input_actions_when_supported() {
        let mut running = snapshot("web", ServiceRuntimeState::Running);
        running.pid = Some(123);

        let row = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &process_service(),
            running,
            RuntimeServiceAttachments {
                log_attached: true,
                input_attached: true,
                ..RuntimeServiceAttachments::default()
            },
        );

        assert_eq!(row.runtime_label, "process");
        assert_eq!(row.execution_label, "$ pnpm dev");
        assert_eq!(row.port_label, ":5173/http1");
        assert!(!row.can_start);
        assert!(row.can_stop);
        assert!(row.can_restart);
        assert!(row.can_open);
        assert!(!row.can_terminal);
        assert!(!row.can_run);
        assert!(row.can_attach);
        assert!(row.can_send_input);
        assert_eq!(row.status_label, "running (pid 123)");

        let stopped = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &process_service(),
            snapshot("web", ServiceRuntimeState::Stopped),
            RuntimeServiceAttachments::default(),
        );

        assert!(stopped.can_start);
        assert!(stopped.can_terminal);
        assert!(stopped.can_run);
        assert!(!stopped.can_stop);
        assert!(!stopped.can_restart);
        assert!(!stopped.can_attach);
        assert!(!stopped.can_send_input);
    }

    #[test]
    fn process_rows_prefer_integrated_terminal_for_active_terminal_sessions() {
        let flags = runtime_service_action_flags(
            &process_service(),
            ServiceRuntimeState::Running,
            true,
            true,
        );

        assert!(flags.can_terminal);
        assert!(!flags.can_attach);
        assert!(!flags.can_send_input);
        assert!(flags.can_stop);
        assert!(flags.can_restart);
    }

    #[test]
    fn container_rows_hide_terminal_input_actions_and_show_container_metadata() {
        let row = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &container_service(),
            snapshot("db", ServiceRuntimeState::Running),
            RuntimeServiceAttachments::default(),
        );

        assert_eq!(row.runtime_label, "container");
        assert_eq!(row.execution_label, "image postgres:16-alpine");
        assert_eq!(row.port_label, "127.0.0.30:5432->5432");
        assert!(!row.can_open);
        assert!(!row.can_terminal);
        assert!(!row.can_run);
        assert!(!row.can_attach);
        assert!(!row.can_send_input);
        assert!(row.can_stop);
        assert!(row.can_restart);
    }

    #[test]
    fn runtime_status_summary_formats_all_states_and_error_details() {
        assert_eq!(
            runtime_status_summary(&snapshot("web", ServiceRuntimeState::Starting)),
            "starting"
        );
        assert_eq!(
            runtime_status_summary(&snapshot("web", ServiceRuntimeState::Running)),
            "running"
        );
        assert_eq!(
            runtime_status_summary(&snapshot("web", ServiceRuntimeState::Unhealthy)),
            "unhealthy"
        );

        let mut crashed = snapshot("web", ServiceRuntimeState::Crashed);
        crashed.exit_code = Some(1);
        assert_eq!(runtime_status_summary(&crashed), "crashed (exit 1)");

        let mut stopped = snapshot("web", ServiceRuntimeState::Stopped);
        stopped.last_error = Some("port release failed".to_string());
        assert_eq!(
            runtime_status_summary(&stopped),
            "stopped (port release failed)"
        );
    }

    #[test]
    fn runtime_rows_match_state_runtime_type_and_text_filters() {
        let process = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &process_service(),
            snapshot("web", ServiceRuntimeState::Stopped),
            RuntimeServiceAttachments::default(),
        );
        let container = build_runtime_service_row(
            "demo",
            "127.0.0.30",
            &container_service(),
            snapshot("db", ServiceRuntimeState::Crashed),
            RuntimeServiceAttachments::default(),
        );

        assert!(runtime_row_matches(&process, RuntimeFilter::Stopped, ""));
        assert!(!runtime_row_matches(&process, RuntimeFilter::Running, ""));
        assert!(runtime_row_matches(&container, RuntimeFilter::Crashed, ""));
        assert!(runtime_row_matches(
            &container,
            RuntimeFilter::Containers,
            ""
        ));
        assert!(!runtime_row_matches(
            &container,
            RuntimeFilter::Processes,
            ""
        ));
        assert!(runtime_row_matches(
            &process,
            RuntimeFilter::Processes,
            "pnpm"
        ));
        assert!(runtime_row_matches(
            &container,
            RuntimeFilter::All,
            "postgres"
        ));
        assert!(!runtime_row_matches(
            &process,
            RuntimeFilter::All,
            "postgres"
        ));
    }

    #[test]
    fn runtime_page_preloads_while_hidden_but_only_live_polls_when_visible() {
        assert!(!runtime_page_uses_live_refresh(Page::Sandboxes));
        assert!(!runtime_page_uses_live_refresh(Page::Settings));
        assert!(runtime_page_uses_live_refresh(Page::Runtime));
    }
}
