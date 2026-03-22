use crate::app::models::{Notice, Page, RuntimeFilter};
use crate::loopbox::{self, LoopboxConfig, ServiceRuntimeState};
use dioxus::prelude::*;

pub(in crate::app) fn render_runtime_page(
    page: Page,
    runtime_filter_value: RuntimeFilter,
    mut runtime_filter: Signal<RuntimeFilter>,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    rsx! {
        if page == Page::Runtime {
            div { class: "page",
                div { class: "page-header",
                    div { class: "page-header-left",
                        h1 { class: "page-title", "Runtime" }
                    }
                    div { class: "filter-bar",
                        button {
                            class: if runtime_filter_value == RuntimeFilter::All { "filter-btn active" } else { "filter-btn" },
                            onclick: move |_| runtime_filter.set(RuntimeFilter::All),
                            "All"
                        }
                        button {
                            class: if runtime_filter_value == RuntimeFilter::Running { "filter-btn active" } else { "filter-btn" },
                            onclick: move |_| runtime_filter.set(RuntimeFilter::Running),
                            "Running"
                        }
                        button {
                            class: if runtime_filter_value == RuntimeFilter::Unhealthy { "filter-btn active" } else { "filter-btn" },
                            onclick: move |_| runtime_filter.set(RuntimeFilter::Unhealthy),
                            "Unhealthy"
                        }
                    }
                }

                for (project_name, project) in &config().projects {
                    {{
                        let mut running = 0_usize;
                        let mut starting = 0_usize;
                        let mut unhealthy = 0_usize;
                        let mut crashed = 0_usize;
                        let mut stopped = 0_usize;
                        let mut statuses = Vec::new();

                        for service in &project.services {
                            if let Ok(status) = loopbox::service_runtime_status(&config(), project_name, &service.name) {
                                match status.state {
                                    ServiceRuntimeState::Running => running += 1,
                                    ServiceRuntimeState::Starting => starting += 1,
                                    ServiceRuntimeState::Unhealthy => unhealthy += 1,
                                    ServiceRuntimeState::Crashed => crashed += 1,
                                    ServiceRuntimeState::Stopped => stopped += 1,
                                }
                                statuses.push((service.name.clone(), status));
                            }
                        }

                        let visible = match runtime_filter_value {
                            RuntimeFilter::All => true,
                            RuntimeFilter::Running => running > 0 || starting > 0,
                            RuntimeFilter::Unhealthy => unhealthy > 0 || crashed > 0,
                        };

                        if !visible {
                            rsx! { div {} }
                        } else {
                            rsx! {
                                section { class: "panel",
                                    div { class: "panel-header",
                                        h2 { "{project_name}" }
                                        div { class: "panel-actions",
                                            button {
                                                class: "btn btn-outline btn-sm",
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
                                                "Start All"
                                            }
                                            button {
                                                class: "btn btn-outline btn-sm",
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
                                                "Stop All"
                                            }
                                        }
                                    }

                                    div { class: "runtime-chips",
                                        if running > 0 {
                                            span { class: "chip chip-success", "running {running}" }
                                        }
                                        if starting > 0 {
                                            span { class: "chip chip-warn", "starting {starting}" }
                                        }
                                        if unhealthy > 0 {
                                            span { class: "chip chip-danger", "unhealthy {unhealthy}" }
                                        }
                                        if crashed > 0 {
                                            span { class: "chip chip-danger", "crashed {crashed}" }
                                        }
                                        if stopped > 0 {
                                            span { class: "chip", "stopped {stopped}" }
                                        }
                                    }

                                    div { class: "runtime-services",
                                        for (service_name, status) in statuses {
                                            div { class: "runtime-row", key: "{service_name}",
                                                span { class: "runtime-svc-name", "{service_name}" }
                                                span {
                                                    class: format!("runtime-svc-status {}", status_class(&status.state)),
                                                    "{status_summary(&status)}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }}
                }

                if config().projects.is_empty() {
                    div { class: "empty-state",
                        p { class: "empty-state-text", "No sandboxes configured yet." }
                    }
                }
            }
        }
    }
}

fn status_class(state: &ServiceRuntimeState) -> &'static str {
    match state {
        ServiceRuntimeState::Running => "status-running",
        ServiceRuntimeState::Starting => "status-starting",
        ServiceRuntimeState::Unhealthy | ServiceRuntimeState::Crashed => "status-danger",
        ServiceRuntimeState::Stopped => "status-stopped",
    }
}

fn status_summary(status: &crate::loopbox::ServiceRuntimeSnapshot) -> String {
    let state = match status.state {
        ServiceRuntimeState::Stopped => "stopped",
        ServiceRuntimeState::Starting => "starting",
        ServiceRuntimeState::Running => "running",
        ServiceRuntimeState::Unhealthy => "unhealthy",
        ServiceRuntimeState::Crashed => "crashed",
    };

    if let Some(pid) = status.pid {
        format!("{state} (pid {pid})")
    } else if let Some(code) = status.exit_code {
        format!("{state} (exit {code})")
    } else if let Some(err) = &status.last_error {
        format!("{state} ({err})")
    } else {
        state.to_string()
    }
}
