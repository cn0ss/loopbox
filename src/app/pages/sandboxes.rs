use crate::app::components::{DoctorIssueRow, ProjectCard};
use crate::app::log_window::{self, LogWindowConfig};
use crate::app::models::{DetailTab, Notice, Page, ProjectEditForm};
use crate::app::utils::{
    copy_to_clipboard, persist_config_and_apply, preview_project_name, preview_service_name,
    preview_suffix,
};
use crate::loopbox::{
    self, AddProjectInput, DoctorIssue, LoopboxConfig, OpenTarget, ProjectConfig,
    ProxyEndpointConfig, ProxyEndpointProtocol, ServiceConfig, ServiceEntry, ServicePortEntry,
    ServiceRuntimeSnapshot, ServiceRuntimeState, UpdateProjectInput,
};
use dioxus::html::input_data::keyboard_types::{Key, Modifiers};
use dioxus::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod editor_helpers;
mod filesystem;
mod helpers;
mod project_detail;
mod project_detail_features;
mod wizard;
mod wizard_features;

use editor_helpers::*;
use filesystem::*;
use helpers::*;
use project_detail::*;
use project_detail_features::*;
use wizard::blank_service_entry as wizard_blank_service_entry;
use wizard::*;
use wizard_features::*;
// ════════════════════════════════════════════
// Sandboxes Overview (Project Grid)
// ════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_sandboxes_page(
    page: Page,
    selected_project_data: Option<(String, ProjectConfig)>,
    config_snapshot: LoopboxConfig,
    doctor_ok: bool,
    doctor_issue_count: usize,
    doctor_issues: Vec<DoctorIssue>,
    suffix_preview: String,
    selected_project: Signal<Option<String>>,
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    pending_auto_apply: Signal<Option<String>>,
    mut runtime_tick: Signal<u64>,
    mut current_page: Signal<Page>,
) -> Element {
    rsx! {
        // ── Overview: Project Grid ──
        if page == Page::Sandboxes && selected_project_data.is_none() {
            div { class: "page",
                div { class: "page-header",
                    div { class: "page-header-left",
                        h1 { class: "page-title", "Sandboxes" }
                        if !doctor_ok {
                            span { class: "health-badge", "{doctor_issue_count} issues" }
                        }
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| current_page.set(Page::NewSandbox),
                        "+ New Sandbox"
                    }
                }

                if config_snapshot.projects.is_empty() {
                    div { class: "empty-state",
                        div { class: "empty-state-icon", "\u{25C8}" }
                        h2 { class: "empty-state-title", "No sandboxes yet" }
                        p { class: "empty-state-desc",
                            "Create your first sandbox to assign a dedicated loopback IP and hostnames to a project."
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| current_page.set(Page::NewSandbox),
                            "Create Sandbox"
                        }
                    }
                } else {
                    div { class: "project-grid",
                        for (index, (name, project)) in config_snapshot.projects.iter().enumerate() {
                            {{
                                let mut running = 0_usize;
                                for service in &project.services {
                                    if let Ok(status) = loopbox::service_runtime_status(&config_snapshot, name, &service.name) {
                                        if matches!(status.state, ServiceRuntimeState::Running | ServiceRuntimeState::Starting) {
                                            running += 1;
                                        }
                                    }
                                }
                                rsx! {
                                    ProjectCard {
                                        key: "{name}",
                                        name: name.clone(),
                                        ip: project.ip.clone(),
                                        primary_host: loopbox::project_primary_host(&config_snapshot, name),
                                        service_count: project.services.len(),
                                        running_count: running,
                                        index,
                                        selected_project,
                                    }
                                }
                            }}
                        }
                    }

                    if !doctor_ok {
                        section { class: "panel doctor-panel",
                            div { class: "panel-header",
                                h2 { "Health Checks" }
                                div { class: "doctor-panel-actions",
                                    span { class: "panel-badge", "{doctor_issues.len()} checks" }
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: move |_| {
                                            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                        },
                                        "Rerun"
                                    }
                                }
                            }
                            ul { class: "doctor-list",
                                for issue in &doctor_issues {
                                    DoctorIssueRow {
                                        issue: issue.clone(),
                                        config,
                                        notice,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Project Detail ──
        if page == Page::Sandboxes {
            if let Some((ref project_name, ref project)) = selected_project_data {
                ProjectDetail {
                    key: "{project_name}",
                    project_name: project_name.clone(),
                    project: project.clone(),
                    suffix_preview: suffix_preview.clone(),
                    config,
                    notice,
                    selected_project,
                    pending_auto_apply,
                    runtime_tick,
                }
            }
        }
    }
}

// ════════════════════════════════════════════
// New Sandbox Page (Wizard)
// ════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_new_sandbox_page(
    page: Page,
    add_form_snapshot: AddProjectInput,
    service_host_previews: Vec<String>,
    add_form: Signal<AddProjectInput>,
    selected_project: Signal<Option<String>>,
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    pending_auto_apply: Signal<Option<String>>,
    current_page: Signal<Page>,
) -> Element {
    rsx! {
        if page == Page::NewSandbox {
            {{
                rsx! {
                    NewSandboxWizard {
                        add_form_snapshot,
                        service_host_previews,
                        add_form,
                        selected_project,
                        config,
                        notice,
                        pending_auto_apply,
                        current_page,
                    }
                }
            }}
        }
    }
}

// ════════════════════════════════════════════
// Project Detail Component
// ════════════════════════════════════════════
