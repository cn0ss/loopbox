use crate::app::components::{DoctorIssueRow, ProjectCard};
use crate::app::log_window::{self, LogWindowConfig};
use crate::app::models::{DetailTab, Notice, Page, ProjectEditForm};
use crate::app::runtime_view::{
    format_cpu_percent, format_memory_bytes, format_sample_age, resource_sparkline_points,
    runtime_service_action_flags, ResourceMetricKind,
};
use crate::app::terminal_window::{self, TerminalWindowConfig};
use crate::app::utils::{
    copy_to_clipboard, decode_service_input_sequence, persist_config_and_apply,
    preview_project_name, preview_service_name, preview_suffix,
};
use crate::loopbox::{
    self, AddProjectInput, DoctorIssue, IncidentEvidence, IncidentKind, IncidentSeverity,
    IncidentTimelineEvent, LoopboxConfig, OpenTarget, ProjectConfig, ProxyEndpointConfig,
    ProxyEndpointProtocol, ServiceConfig, ServiceEntry, ServicePortEntry, ServiceRuntimeSnapshot,
    ServiceRuntimeState, UpdateProjectInput,
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
mod timeline;
mod topology;
mod wizard;
mod wizard_discovery;
mod wizard_features;

use editor_helpers::*;
use filesystem::*;
use helpers::*;
use project_detail::*;
use project_detail_features::*;
use timeline::*;
use topology::*;
use wizard::blank_service_entry as wizard_blank_service_entry;
use wizard::*;
use wizard_discovery::*;
use wizard_features::*;
// ════════════════════════════════════════════
// Sandboxes Overview (Project Grid)
// ════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_sandboxes_page(
    page: Page,
    selected_project_data: Option<(String, ProjectConfig)>,
    overview_running_counts: BTreeMap<String, usize>,
    config_snapshot: LoopboxConfig,
    doctor_ok: bool,
    doctor_issue_count: usize,
    doctor_issues: Vec<DoctorIssue>,
    suffix_preview: String,
    selected_project: Signal<Option<String>>,
    requested_detail_tab: Signal<Option<DetailTab>>,
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    pending_auto_apply: Signal<Option<String>>,
    mut runtime_tick: Signal<u64>,
    mut doctor_refresh: Signal<u64>,
    mut current_page: Signal<Page>,
) -> Element {
    rsx! {
        // ── Overview: Project Grid ──
        if page == Page::Sandboxes && selected_project_data.is_none() {
            {{
                let sandbox_count = config_snapshot.projects.len();
                let service_count: usize = config_snapshot.projects.values().map(|p| p.services.len()).sum();
                let running_count: usize = overview_running_counts.values().sum();
                let domain_suffix = config_snapshot.global.domain_suffix.clone();
                let ip_base = config_snapshot.global.ip_base.clone();

                rsx! {
                    div { class: "page sandboxes-overview",
                        div { class: "page-header",
                            div { class: "page-header-left",
                                div { class: "page-header-stack",
                                    span { class: "page-eyebrow", "Workspace" }
                                    div {
                                        style: "display:flex; align-items:baseline; gap:14px; flex-wrap:wrap;",
                                        h1 { class: "page-title", "sandboxes" }
                                        if !doctor_ok {
                                            span { class: "status-badge status-badge--warn",
                                                "{doctor_issue_count} issues"
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "page-actions",
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| current_page.set(Page::NewSandbox),
                                    "+ New Sandbox"
                                }
                            }
                        }

                        // Scope eyebrow — domain suffix + ip base
                        if !config_snapshot.projects.is_empty() {
                            div { class: "overview-scope",
                                span { class: "overview-scope-item",
                                    span { class: "overview-scope-label", "suffix" }
                                    span { class: "overview-scope-value", ".{domain_suffix}" }
                                }
                                span { class: "overview-scope-sep", "·" }
                                span { class: "overview-scope-item",
                                    span { class: "overview-scope-label", "ip range" }
                                    span { class: "overview-scope-value", "{ip_base}" }
                                }
                            }

                            // Stats strip — italic-serif numerals signature
                            div { class: "overview-stats",
                                div { class: "overview-stat",
                                    span { class: "overview-stat-value", "{sandbox_count}" }
                                    span { class: "overview-stat-label", "sandboxes" }
                                }
                                div { class: "overview-stat",
                                    span { class: "overview-stat-value", "{service_count}" }
                                    span { class: "overview-stat-label", "services" }
                                }
                                div { class: "overview-stat overview-stat-accent",
                                    span { class: "overview-stat-value", "{running_count}" }
                                    span { class: "overview-stat-label", "running" }
                                }
                            }
                        }

                        if config_snapshot.projects.is_empty() {
                            div { class: "empty-state",
                                div { class: "empty-state-icon", "—" }
                                h2 { class: "empty-state-title", "no sandboxes yet" }
                                p { class: "empty-state-desc",
                                    "Create your first sandbox to assign a dedicated loopback IP and hostnames to a project. Loopbox keeps your local DNS, hosts file, and service runtime in sync."
                                }
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| current_page.set(Page::NewSandbox),
                                    "Create your first sandbox →"
                                }
                            }
                        } else {
                            div { class: "project-grid",
                                for (index, (name, project)) in config_snapshot.projects.iter().enumerate() {
                                    {{
                                        let running = overview_running_counts.get(name).copied().unwrap_or(0);
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
                                                    doctor_refresh.with_mut(|tick| *tick = tick.wrapping_add(1));
                                                },
                                                "Rerun"
                                            }
                                        }
                                    }
                                    ul { class: "doctor-list",
                                        for issue in &doctor_issues {
                                            DoctorIssueRow {
                                                issue: issue.clone(),
                                                selected_project,
                                                requested_detail_tab,
                                                current_page,
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
            }}
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
                    requested_detail_tab,
                    pending_auto_apply,
                    runtime_tick,
                    current_page,
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
