use super::*;
use crate::loopbox::{ProxyCaptureMode, ProxyTrafficEvent, ServiceRuntimeKind};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn project_detail_show_traffic_tab(
    _config: &LoopboxConfig,
    _project_name: &str,
) -> bool {
    true
}

pub(super) fn project_detail_service_port_label_override(
    project_ip: &str,
    service: &ServiceConfig,
) -> Option<String> {
    if !matches!(service.runtime, ServiceRuntimeKind::Container) {
        return None;
    }

    let effective_ports = loopbox::service_ports(service);
    if effective_ports.is_empty() {
        return Some("—".to_string());
    }

    Some(
        effective_ports
            .iter()
            .map(|entry| format!("{}:{}->{}", project_ip, entry.port, entry.port))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub(super) fn project_detail_service_runtime_label(
    service: &ServiceConfig,
) -> Option<&'static str> {
    Some(match service.runtime {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    })
}

pub(super) fn project_detail_service_execution_label(service: &ServiceConfig) -> Option<String> {
    Some(match service.runtime {
        ServiceRuntimeKind::Process => format!("$ {}", service.command),
        ServiceRuntimeKind::Container => {
            let image = service
                .container
                .as_ref()
                .map(|container| container.image.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("<missing image>");
            format!("image {image}")
        }
    })
}

#[component]
pub(super) fn ProjectDetailServiceRuntimeFields(
    service_index: usize,
    entry: ServiceEntry,
    mut edit_form: Signal<ProjectEditForm>,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    rsx! {
        label { class: "field",
            span { "Runtime" }
            select {
                value: "{service_runtime_value(service_entry_runtime(&entry))}",
                onchange: move |evt: Event<FormData>| {
                    let selected_runtime = parse_service_runtime(&evt.value());
                    let runtime = service_runtime_value(selected_runtime);
                    edit_form.write().services[service_index].runtime = runtime.to_string();
                },
                option { value: "process", "Process" }
                option {
                    value: "container",
                    "Container"
                }
            }
        }
        if service_entry_requires_command(&entry) {
            label { class: "field field-wide",
                span { "Command" }
                input {
                    value: "{entry.command}",
                    placeholder: "pnpm dev",
                    oninput: move |evt: Event<FormData>| {
                        edit_form.write().services[service_index].command =
                            normalize_service_command_input(&evt.value());
                    },
                }
            }
        } else {
            label { class: "field field-wide",
                span { "Container Image" }
                input {
                    value: "{entry.container_image}",
                    placeholder: "postgres:16",
                    oninput: move |evt: Event<FormData>| {
                        edit_form.write().services[service_index].container_image = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Args" }
                input {
                    value: "{entry.container_args}",
                    placeholder: "-c shared_buffers=256MB, -c max_connections=200",
                    oninput: move |evt: Event<FormData>| {
                        edit_form.write().services[service_index].container_args = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Env (comma/newline separated KEY=VALUE)" }
                textarea {
                    class: "field-input field-textarea",
                    value: "{entry.container_env}",
                    placeholder: "POSTGRES_DB=app
    POSTGRES_PASSWORD=secret",
                    oninput: move |evt: Event<FormData>| {
                        edit_form.write().services[service_index].container_env = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Volumes (comma/newline separated)" }
                textarea {
                    class: "field-input field-textarea",
                    value: "{entry.container_volumes}",
                    placeholder: "/tmp/pg:/var/lib/postgresql/data",
                    oninput: move |evt: Event<FormData>| {
                        edit_form.write().services[service_index].container_volumes = evt.value();
                    },
                }
            }
            button {
                class: if entry.container_auto_remove { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                onclick: move |_| {
                    let current = edit_form().services[service_index].container_auto_remove;
                    edit_form.write().services[service_index].container_auto_remove = !current;
                },
                if entry.container_auto_remove { "Container auto-remove: on" } else { "Container auto-remove: off" }
            }
        }
    }
}

#[component]
pub(super) fn ProjectDetailTrafficTab(
    project_name: String,
    project: ProjectConfig,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
) -> Element {
    let initial_traffic_filter = project.services.first().map(|service| service.name.clone());
    let mut traffic_filter = use_signal(move || initial_traffic_filter.clone());
    let mut selected_traffic_event_id = use_signal(|| None::<u64>);
    let mut request_body_modes = use_signal(BTreeMap::<u64, TrafficBodyViewMode>::new);
    let mut response_body_modes = use_signal(BTreeMap::<u64, TrafficBodyViewMode>::new);
    let mut expanded_request_bodies = use_signal(HashSet::<u64>::new);
    let mut expanded_response_bodies = use_signal(HashSet::<u64>::new);

    let pn_traffic = project_name.clone();
    let traffic_events = use_memo(move || {
        let _tick = runtime_tick();
        let cfg = config();
        let filter = traffic_filter();
        let selected_service = cfg
            .projects
            .get(&pn_traffic)
            .and_then(|proj| effective_log_selection(proj.services.as_slice(), filter));
        loopbox::proxy_traffic_events_for_project_with_persisted(
            &pn_traffic,
            selected_service.as_deref(),
            300,
        )
        .unwrap_or_default()
    });

    let traffic_snapshot = traffic_events();
    let selected_traffic_service =
        effective_log_selection(project.services.as_slice(), traffic_filter());
    let capture_enabled = loopbox::project_proxy_traffic_enabled(&config(), &project_name);
    let capture_mode = loopbox::project_proxy_traffic_capture_mode(&config(), &project_name);
    let traffic_disk_stats = loopbox::proxy_traffic_disk_stats();
    let selected_traffic_event = selected_traffic_event_id()
        .and_then(|selected_id| {
            traffic_snapshot
                .iter()
                .find(|event| event.id == selected_id)
                .cloned()
        })
        .or_else(|| traffic_snapshot.first().cloned());
    let selected_traffic_event_id_snapshot = selected_traffic_event.as_ref().map(|event| event.id);
    let request_body_mode = selected_traffic_event_id_snapshot
        .and_then(|event_id| request_body_modes().get(&event_id).copied())
        .unwrap_or(TrafficBodyViewMode::Pretty);
    let response_body_mode = selected_traffic_event_id_snapshot
        .and_then(|event_id| response_body_modes().get(&event_id).copied())
        .unwrap_or(TrafficBodyViewMode::Pretty);
    let request_body_expanded = selected_traffic_event_id_snapshot
        .is_some_and(|event_id| expanded_request_bodies().contains(&event_id));
    let response_body_expanded = selected_traffic_event_id_snapshot
        .is_some_and(|event_id| expanded_response_bodies().contains(&event_id));

    let mut force_tick = move || {
        runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
    };

    rsx! {
        div { class: "tab-content-traffic",
            div { class: "traffic-toolbar-compact",
                div { class: "traffic-toolbar-left",
                    for service in &project.services {
                        button {
                            key: "traffic-{service.name}",
                            class: if selected_traffic_service.as_ref() == Some(&service.name) {
                                "btn btn-sm btn-toggle-on"
                            } else {
                                "btn btn-sm btn-outline"
                            },
                            onclick: {
                                let svc = service.name.clone();
                                move |_| traffic_filter.set(Some(svc.clone()))
                            },
                            "{service.name}"
                        }
                    }
                    span { class: "traffic-toolbar-divider" }
                    span {
                        class: if capture_enabled { "capture-badge capture-badge-on" } else { "capture-badge capture-badge-off" },
                        if capture_enabled { "● REC" } else { "○ idle" }
                    }
                    button {
                        class: "traffic-mode-chip",
                        title: "Click to cycle capture mode",
                        onclick: {
                            let pn = project_name.clone();
                            move |_| {
                                config.with_mut(|cfg| {
                                    let default_mode = cfg.global.proxy_traffic.capture_mode_default.clone();
                                    if let Some(project_cfg) = cfg.projects.get_mut(&pn) {
                                        let current_mode = project_cfg
                                            .proxy_traffic_capture_mode
                                            .clone()
                                            .unwrap_or(default_mode);
                                        project_cfg.proxy_traffic_capture_mode =
                                            Some(loopbox::enforce_traffic_capture_mode(
                                                next_traffic_capture_mode(current_mode),
                                            ));
                                    }
                                });
                                match loopbox::save_config(&config()) {
                                    Ok(path) => notice.set(Some(Notice::success(format!(
                                        "Capture mode updated. Saved {}.",
                                        path.display()
                                    )))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                                if let Err(err) = loopbox::sync_reverse_proxy_sidecar(&config()) {
                                    eprintln!("Loopbox reverse proxy sidecar sync warning: {err}");
                                }
                                force_tick();
                            }
                        },
                        "{traffic_capture_mode_label(&capture_mode)}"
                    }
                    span { class: "traffic-meta-chip",
                        "{format_bytes_compact(traffic_disk_stats.total_bytes)}"
                    }
                    if traffic_disk_stats.dropped_events > 0 {
                        span { class: "traffic-meta-chip traffic-meta-chip-warn",
                            "{traffic_disk_stats.dropped_events} dropped"
                        }
                    }
                }
                div { class: "traffic-toolbar-right",
                    button {
                        class: if capture_enabled { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                        onclick: {
                            let pn = project_name.clone();
                            move |_| {
                                config.with_mut(|cfg| {
                                    let default_enabled = cfg.global.proxy_traffic.capture_enabled_by_default;
                                    if let Some(project_cfg) = cfg.projects.get_mut(&pn) {
                                        let currently_enabled = project_cfg
                                            .proxy_traffic_capture_enabled
                                            .unwrap_or(default_enabled);
                                        project_cfg.proxy_traffic_capture_enabled = Some(!currently_enabled);
                                    }
                                });
                                match loopbox::save_config(&config()) {
                                    Ok(path) => notice.set(Some(Notice::success(format!(
                                        "Traffic capture updated. Saved {}.",
                                        path.display()
                                    )))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                                if let Err(err) = loopbox::sync_reverse_proxy_sidecar(&config()) {
                                    eprintln!("Loopbox reverse proxy sidecar sync warning: {err}");
                                }
                                force_tick();
                            }
                        },
                        if capture_enabled { "Stop" } else { "Record" }
                    }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let pn = project_name.clone();
                            let selected_service = selected_traffic_service.clone();
                            move |_| {
                                let cfg_snapshot = config();
                                let Some(output_path) = default_proxy_har_export_path(
                                    &cfg_snapshot,
                                    &pn,
                                    selected_service.as_deref(),
                                ) else {
                                    notice.set(Some(Notice::error(
                                        "Could not resolve export path for this project.".to_string(),
                                    )));
                                    return;
                                };
                                match loopbox::export_proxy_traffic_har_for_project(
                                    &pn,
                                    selected_service.as_deref(),
                                    &output_path,
                                ) {
                                    Ok(count) => notice.set(Some(Notice::success(format!(
                                        "Exported {count} request(s) to {}.",
                                        output_path.display()
                                    )))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        },
                        "Export"
                    }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let pn = project_name.clone();
                            move |_| {
                                match loopbox::clear_proxy_traffic_events_for_project(&pn) {
                                    Ok(removed) => {
                                        selected_traffic_event_id.set(None);
                                        notice.set(Some(Notice::info(format!(
                                            "Cleared {removed} traffic event(s)."
                                        ))));
                                        force_tick();
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            }
                        },
                        "Clear"
                    }
                }
            }

            div { class: "traffic-split",
                div { class: "traffic-list-pane",
                    if traffic_snapshot.is_empty() {
                        div { class: "traffic-list-empty",
                            p { "No captured traffic yet." }
                            p { class: "text-dim", "Enable capture and open a service URL." }
                        }
                    } else {
                        div { class: "traffic-list-header",
                            span { class: "traffic-list-count",
                                "{traffic_snapshot.len()} requests"
                            }
                        }
                        for event in &traffic_snapshot {
                            div {
                                key: "traffic-event-{event.id}",
                                class: if selected_traffic_event_id() == Some(event.id) {
                                    "traffic-row traffic-row-selected"
                                } else {
                                    "traffic-row"
                                },
                                onclick: {
                                    let id = event.id;
                                    move |_| selected_traffic_event_id.set(Some(id))
                                },
                                span { class: format!("traffic-method {}", traffic_method_class(&event.method)), "{event.method}" }
                                span { class: "traffic-path", "{event.path}" }
                                span { class: format!("traffic-status-code {}", traffic_status_class(event)), "{traffic_status_label(event)}" }
                                span { class: "traffic-dur", "{event.duration_ms}ms" }
                            }
                        }
                    }
                }

                div { class: "traffic-detail-pane",
                    if let Some(event) = selected_traffic_event {
                        div { class: "traffic-detail-header",
                            span { class: format!("traffic-detail-method {}", traffic_method_class(&event.method)), "{event.method}" }
                            span { class: "traffic-detail-path", "{event.path}" }
                            span { class: format!("traffic-status-code {}", traffic_status_class(&event)), "{traffic_status_label(&event)}" }
                            button {
                                class: "traffic-copy-req-btn",
                                title: "Copy full HTTP request to clipboard",
                                onclick: {
                                    let text = format_whole_request_text(&event);
                                    move |_| {
                                        if let Err(e) = copy_to_clipboard(&text) {
                                            eprintln!("Copy failed: {e}");
                                        }
                                    }
                                },
                                "⎘ copy request"
                            }
                        }
                        div { class: "traffic-detail-meta",
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "time" }
                                span { class: "traffic-detail-value", "{event.started_at_utc}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "service" }
                                span { class: "traffic-detail-value", "{event.service_name}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "host" }
                                span { class: "traffic-detail-value", "{event.host}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "duration" }
                                span { class: "traffic-detail-value", "{event.duration_ms}ms" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "req total" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.request_bytes)}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "req hdr" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.request_header_bytes)}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "req body" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.request_body_bytes)}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "resp total" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.response_bytes)}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "resp hdr" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.response_header_bytes)}" }
                            }
                            div { class: "traffic-detail-field",
                                span { class: "traffic-detail-label", "resp body" }
                                span { class: "traffic-detail-value", "{format_bytes_compact(event.response_body_bytes)}" }
                            }
                            if let Some(err) = &event.error {
                                div { class: "traffic-detail-field",
                                    span { class: "traffic-detail-label", "error" }
                                    span { class: "traffic-detail-value traffic-detail-error", "{err}" }
                                }
                            }
                        }
                        div { class: "traffic-detail-section",
                            div { class: "traffic-section-title",
                                span { "Request Headers" }
                                if !event.request_headers.is_empty() {
                                    button {
                                        class: "traffic-copy-btn",
                                        title: "Copy request headers",
                                        onclick: {
                                            let text = format_request_headers_text(&event);
                                            move |_| {
                                                if let Err(e) = copy_to_clipboard(&text) {
                                                    eprintln!("Copy failed: {e}");
                                                }
                                            }
                                        },
                                        "⎘ copy"
                                    }
                                }
                            }
                            div { class: "traffic-detail-headers",
                                if event.request_headers.is_empty() {
                                    span { class: "text-dim", "none" }
                                } else {
                                    for header in &event.request_headers {
                                        div { class: "traffic-header-row",
                                            span { class: "traffic-header-name", "{header.name}" }
                                            span { class: "traffic-header-value", "{header.value}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "traffic-detail-section",
                            div { class: "traffic-section-title",
                                span { "Response Headers" }
                                if !event.response_headers.is_empty() {
                                    button {
                                        class: "traffic-copy-btn",
                                        title: "Copy response headers",
                                        onclick: {
                                            let text = format_response_headers_text(&event);
                                            move |_| {
                                                if let Err(e) = copy_to_clipboard(&text) {
                                                    eprintln!("Copy failed: {e}");
                                                }
                                            }
                                        },
                                        "⎘ copy"
                                    }
                                }
                            }
                            div { class: "traffic-detail-headers",
                                if event.response_headers.is_empty() {
                                    span { class: "text-dim", "none" }
                                } else {
                                    for header in &event.response_headers {
                                        div { class: "traffic-header-row",
                                            span { class: "traffic-header-name", "{header.name}" }
                                            span { class: "traffic-header-value", "{header.value}" }
                                        }
                                    }
                                }
                            }
                        }
                        {
                            let request_body_render =
                                format_request_body_render(&event, request_body_mode);
                            let request_body_can_expand =
                                request_body_render.line_count > TRAFFIC_BODY_CLAMP_LINES;
                            let request_body_class =
                                if request_body_can_expand && !request_body_expanded {
                                    "traffic-detail-body traffic-detail-body-clamped"
                                } else {
                                    "traffic-detail-body"
                                };
                            let request_expand_label = if request_body_expanded {
                                "collapse".to_string()
                            } else {
                                format!("expand ({} lines)", request_body_render.line_count)
                            };
                            rsx! {
                                div { class: "traffic-detail-section",
                                    div { class: "traffic-section-title",
                                        span { "Request Body" }
                                        div { class: "traffic-section-actions",
                                            if event.request_body_preview.is_some() && !event.request_body_binary {
                                                div { class: "traffic-body-view-toggle",
                                                    button {
                                                        class: if request_body_mode == TrafficBodyViewMode::Pretty {
                                                            "traffic-toggle-btn traffic-toggle-btn-active"
                                                        } else {
                                                            "traffic-toggle-btn"
                                                        },
                                                        onclick: {
                                                            let event_id = event.id;
                                                            move |_| {
                                                                request_body_modes.with_mut(|modes| {
                                                                    modes.insert(event_id, TrafficBodyViewMode::Pretty);
                                                                });
                                                            }
                                                        },
                                                        "pretty"
                                                    }
                                                    button {
                                                        class: if request_body_mode == TrafficBodyViewMode::Raw {
                                                            "traffic-toggle-btn traffic-toggle-btn-active"
                                                        } else {
                                                            "traffic-toggle-btn"
                                                        },
                                                        onclick: {
                                                            let event_id = event.id;
                                                            move |_| {
                                                                request_body_modes.with_mut(|modes| {
                                                                    modes.insert(event_id, TrafficBodyViewMode::Raw);
                                                                });
                                                            }
                                                        },
                                                        "raw"
                                                    }
                                                }
                                            }
                                            if event.request_body_preview.is_some() && !event.request_body_binary {
                                                button {
                                                    class: "traffic-copy-btn",
                                                    title: "Copy request body",
                                                    onclick: {
                                                        let text = format_request_body_text(&event);
                                                        move |_| {
                                                            if let Err(e) = copy_to_clipboard(&text) {
                                                                eprintln!("Copy failed: {e}");
                                                            }
                                                        }
                                                    },
                                                    "⎘ copy"
                                                }
                                            }
                                        }
                                    }
                                    div { class: "traffic-body-wrapper",
                                        pre {
                                            class: request_body_class,
                                            dangerous_inner_html: request_body_render.html
                                        }
                                        if request_body_can_expand {
                                            button {
                                                class: "traffic-expand-btn",
                                                onclick: {
                                                    let event_id = event.id;
                                                    move |_| {
                                                        expanded_request_bodies.with_mut(|expanded| {
                                                            if expanded.contains(&event_id) {
                                                                expanded.remove(&event_id);
                                                            } else {
                                                                expanded.insert(event_id);
                                                            }
                                                        });
                                                    }
                                                },
                                                "{request_expand_label}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        {
                            let response_body_render = format_traffic_body_render(
                                event.response_body_preview.as_deref(),
                                &event.response_headers,
                                event.response_body_binary,
                                event.response_body_truncated,
                                response_body_mode,
                            );
                            let response_body_can_expand =
                                response_body_render.line_count > TRAFFIC_BODY_CLAMP_LINES;
                            let response_body_class =
                                if response_body_can_expand && !response_body_expanded {
                                    "traffic-detail-body traffic-detail-body-clamped"
                                } else {
                                    "traffic-detail-body"
                                };
                            let response_expand_label = if response_body_expanded {
                                "collapse".to_string()
                            } else {
                                format!("expand ({} lines)", response_body_render.line_count)
                            };
                            rsx! {
                                div { class: "traffic-detail-section",
                                    div { class: "traffic-section-title",
                                        span { "Response Body" }
                                        div { class: "traffic-section-actions",
                                            if event.response_body_preview.is_some() && !event.response_body_binary {
                                                div { class: "traffic-body-view-toggle",
                                                    button {
                                                        class: if response_body_mode == TrafficBodyViewMode::Pretty {
                                                            "traffic-toggle-btn traffic-toggle-btn-active"
                                                        } else {
                                                            "traffic-toggle-btn"
                                                        },
                                                        onclick: {
                                                            let event_id = event.id;
                                                            move |_| {
                                                                response_body_modes.with_mut(|modes| {
                                                                    modes.insert(event_id, TrafficBodyViewMode::Pretty);
                                                                });
                                                            }
                                                        },
                                                        "pretty"
                                                    }
                                                    button {
                                                        class: if response_body_mode == TrafficBodyViewMode::Raw {
                                                            "traffic-toggle-btn traffic-toggle-btn-active"
                                                        } else {
                                                            "traffic-toggle-btn"
                                                        },
                                                        onclick: {
                                                            let event_id = event.id;
                                                            move |_| {
                                                                response_body_modes.with_mut(|modes| {
                                                                    modes.insert(event_id, TrafficBodyViewMode::Raw);
                                                                });
                                                            }
                                                        },
                                                        "raw"
                                                    }
                                                }
                                            }
                                            if event.response_body_preview.is_some() && !event.response_body_binary {
                                                button {
                                                    class: "traffic-copy-btn",
                                                    title: "Copy response body",
                                                    onclick: {
                                                        let text = format_response_body_text(&event);
                                                        move |_| {
                                                            if let Err(e) = copy_to_clipboard(&text) {
                                                                eprintln!("Copy failed: {e}");
                                                            }
                                                        }
                                                    },
                                                    "⎘ copy"
                                                }
                                            }
                                        }
                                    }
                                    div { class: "traffic-body-wrapper",
                                        pre {
                                            class: response_body_class,
                                            dangerous_inner_html: response_body_render.html
                                        }
                                        if response_body_can_expand {
                                            button {
                                                class: "traffic-expand-btn",
                                                onclick: {
                                                    let event_id = event.id;
                                                    move |_| {
                                                        expanded_response_bodies.with_mut(|expanded| {
                                                            if expanded.contains(&event_id) {
                                                                expanded.remove(&event_id);
                                                            } else {
                                                                expanded.insert(event_id);
                                                            }
                                                        });
                                                    }
                                                },
                                                "{response_expand_label}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "traffic-detail-empty",
                            "Select a request to inspect"
                        }
                    }
                }
            }
        }
    }
}

fn parse_service_runtime(raw: &str) -> ServiceRuntimeKind {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "container" => ServiceRuntimeKind::Container,
        _ => ServiceRuntimeKind::Process,
    }
}

fn service_runtime_value(runtime: ServiceRuntimeKind) -> &'static str {
    match runtime {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    }
}

fn service_entry_runtime(entry: &ServiceEntry) -> ServiceRuntimeKind {
    parse_service_runtime(&entry.runtime)
}

fn service_entry_requires_command(entry: &ServiceEntry) -> bool {
    !matches!(service_entry_runtime(entry), ServiceRuntimeKind::Container)
}

fn traffic_status_label(event: &ProxyTrafficEvent) -> String {
    if let Some(code) = event.status_code {
        return code.to_string();
    }
    if event.error.is_some() {
        return "error".to_string();
    }
    "unknown".to_string()
}

fn traffic_method_class(method: &str) -> &'static str {
    match method {
        "GET" => "method-get",
        "POST" => "method-post",
        "PUT" => "method-put",
        "PATCH" => "method-patch",
        "DELETE" => "method-delete",
        "HEAD" => "method-head",
        _ => "method-other",
    }
}

fn traffic_status_class(event: &ProxyTrafficEvent) -> &'static str {
    if event.error.is_some() && event.status_code.is_none() {
        return "tstatus-err";
    }
    match event.status_code {
        Some(code) if code < 300 => "tstatus-2xx",
        Some(code) if code < 400 => "tstatus-3xx",
        Some(code) if code < 500 => "tstatus-4xx",
        Some(_) => "tstatus-5xx",
        None => "tstatus-unk",
    }
}

fn traffic_capture_mode_label(mode: &ProxyCaptureMode) -> &'static str {
    match mode {
        ProxyCaptureMode::Metadata => "metadata",
        ProxyCaptureMode::Headers => "headers",
        ProxyCaptureMode::BodyPreview => "body_preview",
    }
}

fn next_traffic_capture_mode(mode: ProxyCaptureMode) -> ProxyCaptureMode {
    match mode {
        ProxyCaptureMode::Metadata => ProxyCaptureMode::Headers,
        ProxyCaptureMode::Headers => ProxyCaptureMode::BodyPreview,
        ProxyCaptureMode::BodyPreview => ProxyCaptureMode::Metadata,
    }
}

const TRAFFIC_BODY_CLAMP_LINES: usize = 28;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrafficBodyViewMode {
    Pretty,
    Raw,
}

#[derive(Clone)]
struct TrafficBodyRender {
    html: String,
    line_count: usize,
}

fn format_request_body_render(
    event: &ProxyTrafficEvent,
    mode: TrafficBodyViewMode,
) -> TrafficBodyRender {
    if event.request_body_binary {
        return TrafficBodyRender {
            html: "<span class=\"body-hint\">[binary payload omitted]</span>".to_string(),
            line_count: 1,
        };
    }
    if event.request_body_preview.is_some() {
        return format_traffic_body_render(
            event.request_body_preview.as_deref(),
            &event.request_headers,
            false,
            event.request_body_truncated,
            mode,
        );
    }
    if request_method_typically_has_no_body(&event.method)
        && !request_headers_indicate_body(&event.request_headers)
    {
        return TrafficBodyRender {
            html: format!(
                "<span class=\"body-hint\">(empty - {} has no body)</span>",
                event.method
            ),
            line_count: 1,
        };
    }
    TrafficBodyRender {
        html: "<span class=\"body-hint\">-</span>".to_string(),
        line_count: 1,
    }
}

fn format_traffic_body_render(
    preview: Option<&str>,
    headers: &[crate::loopbox::ProxyTrafficHeader],
    is_binary: bool,
    is_truncated: bool,
    mode: TrafficBodyViewMode,
) -> TrafficBodyRender {
    if is_binary {
        return TrafficBodyRender {
            html: "<span class=\"body-hint\">[binary payload omitted]</span>".to_string(),
            line_count: 1,
        };
    }
    let Some(raw_preview) = preview else {
        return TrafficBodyRender {
            html: "<span class=\"body-hint\">-</span>".to_string(),
            line_count: 1,
        };
    };

    let content_type = header_content_type(headers);
    let mut rendered = match mode {
        TrafficBodyViewMode::Raw => html_escape(raw_preview),
        TrafficBodyViewMode::Pretty => {
            if is_json_body(content_type.as_deref(), raw_preview) {
                format_json_body_html(raw_preview)
            } else if is_form_urlencoded_body(content_type.as_deref()) {
                format_form_urlencoded_body_html(raw_preview)
            } else {
                html_escape(raw_preview)
            }
        }
    };

    if is_truncated {
        rendered.push_str("\n<span class=\"body-truncated\">...[truncated]</span>");
    }
    TrafficBodyRender {
        line_count: body_line_count(&rendered),
        html: rendered,
    }
}

fn body_line_count(rendered: &str) -> usize {
    let count = rendered.lines().count();
    if count == 0 {
        1
    } else {
        count
    }
}

fn format_request_body_text(event: &ProxyTrafficEvent) -> String {
    if event.request_body_binary {
        return "[binary payload]".to_string();
    }
    event.request_body_preview.clone().unwrap_or_default()
}

fn format_response_body_text(event: &ProxyTrafficEvent) -> String {
    if event.response_body_binary {
        return "[binary payload]".to_string();
    }
    event.response_body_preview.clone().unwrap_or_default()
}

fn format_whole_request_text(event: &ProxyTrafficEvent) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} {} HTTP/1.1\\r\\n", event.method, event.path));
    out.push_str(&format!("Host: {}\\r\\n", event.host));
    for header in &event.request_headers {
        out.push_str(&format!("{}: {}\\r\\n", header.name, header.value));
    }
    let body = format_request_body_text(event);
    if !body.is_empty() {
        out.push_str("\\r\\n");
        out.push_str(&body);
    }
    out
}

fn format_request_headers_text(event: &ProxyTrafficEvent) -> String {
    event
        .request_headers
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_response_headers_text(event: &ProxyTrafficEvent) -> String {
    event
        .response_headers
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn request_headers_indicate_body(headers: &[crate::loopbox::ProxyTrafficHeader]) -> bool {
    headers.iter().any(|header| {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return !header.value.trim().is_empty();
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            return header
                .value
                .split(',')
                .next()
                .and_then(|value| value.trim().parse::<usize>().ok())
                .is_some_and(|len| len > 0);
        }
        false
    })
}

fn request_method_typically_has_no_body(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "OPTIONS" | "TRACE")
}

fn header_content_type(headers: &[crate::loopbox::ProxyTrafficHeader]) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
        .map(|header| {
            header
                .value
                .split(';')
                .next()
                .unwrap_or(header.value.as_str())
                .trim()
                .to_ascii_lowercase()
        })
}

fn is_json_body(content_type: Option<&str>, preview: &str) -> bool {
    if content_type.is_some_and(|ct| ct == "application/json" || ct.ends_with("+json")) {
        return true;
    }
    let trimmed = preview.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

fn json_value_to_html(value: &serde_json::Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let child_pad = "  ".repeat(indent + 1);
    match value {
        serde_json::Value::Null => "<span class=\"jsn-null\">null</span>".to_string(),
        serde_json::Value::Bool(b) => format!("<span class=\"jsn-bool\">{b}</span>"),
        serde_json::Value::Number(n) => format!("<span class=\"jsn-num\">{n}</span>"),
        serde_json::Value::String(s) => {
            format!("<span class=\"jsn-str\">\"{}\"</span>", html_escape(s))
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return "<span class=\"jsn-punct\">[]</span>".to_string();
            }
            let mut out = String::from("<span class=\"jsn-punct\">[</span>\\n");
            let last = arr.len().saturating_sub(1);
            for (i, item) in arr.iter().enumerate() {
                out.push_str(&child_pad);
                out.push_str(&json_value_to_html(item, indent + 1));
                if i < last {
                    out.push_str("<span class=\"jsn-punct\">,</span>");
                }
                out.push('\n');
            }
            out.push_str(&pad);
            out.push_str("<span class=\"jsn-punct\">]</span>");
            out
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                return "<span class=\"jsn-punct\">{}</span>".to_string();
            }
            let mut out = String::from("<span class=\"jsn-punct\">{</span>\\n");
            let last = obj.len().saturating_sub(1);
            for (i, (key, val)) in obj.iter().enumerate() {
                out.push_str(&child_pad);
                out.push_str(&format!(
                    "<span class=\"jsn-key\">\"{}\"</span>",
                    html_escape(key)
                ));
                out.push_str("<span class=\"jsn-punct\">: </span>");
                out.push_str(&json_value_to_html(val, indent + 1));
                if i < last {
                    out.push_str("<span class=\"jsn-punct\">,</span>");
                }
                out.push('\n');
            }
            out.push_str(&pad);
            out.push_str("<span class=\"jsn-punct\">}</span>");
            out
        }
    }
}

fn format_json_body_html(preview: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(preview) {
        Ok(value) => json_value_to_html(&value, 0),
        Err(_) => html_escape(preview),
    }
}

fn is_form_urlencoded_body(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct == "application/x-www-form-urlencoded")
}

fn format_form_urlencoded_body_html(preview: &str) -> String {
    if preview.trim().is_empty() {
        return String::new();
    }
    preview
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = decode_url_component(raw_key);
            let value = decode_url_component(raw_value);
            format!(
                "<span class=\"furl-key\">{}</span><span class=\"jsn-punct\">=</span><span class=\"furl-val\">{}</span>",
                html_escape(&key),
                html_escape(&value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_url_component(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'+' {
            out.push(b' ');
            index += 1;
            continue;
        }
        if byte == b'%' && index + 2 < bytes.len() {
            let hi = hex_value(bytes[index + 1]);
            let lo = hex_value(bytes[index + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(byte);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn format_bytes_compact(bytes: u64) -> String {
    let kb = 1024_f64;
    let mb = kb * 1024_f64;
    let gb = mb * 1024_f64;
    let value = bytes as f64;
    if value >= gb {
        return format!("{:.2} GB", value / gb);
    }
    if value >= mb {
        return format!("{:.2} MB", value / mb);
    }
    if value >= kb {
        return format!("{:.2} KB", value / kb);
    }
    format!("{bytes} B")
}

fn default_proxy_har_export_path(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: Option<&str>,
) -> Option<PathBuf> {
    let project = config.projects.get(project_name)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let project_part = sanitize_filename_fragment(project_name);
    let service_part = service_name.map(sanitize_filename_fragment);
    let filename = if let Some(service) = service_part {
        format!("loopbox-traffic-{project_part}-{service}-{timestamp}.har")
    } else {
        format!("loopbox-traffic-{project_part}-{timestamp}.har")
    };
    Some(PathBuf::from(&project.dir).join(filename))
}

fn sanitize_filename_fragment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}
