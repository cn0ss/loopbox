use crate::app::models::Page;
use crate::app::pages::agent_api_audit::sidebar_show_agent_api_audit_tab;
use crate::loopbox::{
    can_check_for_updates, check_for_updates, is_newer_release_tag, LatestReleaseInfo,
};
use dioxus::prelude::*;

pub(super) fn render_sidebar(
    page: Page,
    projects: Vec<String>,
    mut current_page: Signal<Page>,
    mut selected_project: Signal<Option<String>>,
    app_version_label: String,
    latest_release: Option<LatestReleaseInfo>,
) -> Element {
    let update_release =
        latest_release.filter(|release| is_newer_release_tag(&app_version_label, &release.tag));
    let updater_ready = can_check_for_updates();

    let mut sandboxes_expanded = use_signal(|| true);
    let is_expanded = sandboxes_expanded();
    let selected_name = selected_project();
    let project_count = projects.len();
    let show_agent_api_audit_tab = sidebar_show_agent_api_audit_tab();
    let sandboxes_header_active =
        matches!(page, Page::NewSandbox) || (page == Page::Sandboxes && selected_name.is_none());

    rsx! {
        aside { class: "sidebar",
            div { class: "sidebar-brand",
                span { class: "brand-glyph", "\u{25C8}" }
                span { class: "brand-name", "loopbox" }
            }

            nav { class: "sidebar-nav",
                // ── Workspaces ──
                div { class: "nav-section",
                    div { class: "nav-section-label", "workspaces" }

                    button {
                        class: if sandboxes_header_active { "nav-link nav-link-tree active" } else { "nav-link nav-link-tree" },
                        onclick: move |_| {
                            if sandboxes_header_active {
                                sandboxes_expanded.set(!is_expanded);
                            } else {
                                current_page.set(Page::Sandboxes);
                                selected_project.set(None);
                                sandboxes_expanded.set(true);
                            }
                        },
                        span {
                            class: if is_expanded { "nav-tree-arrow nav-tree-arrow-open" } else { "nav-tree-arrow" },
                            "▸"
                        }
                        span { class: "nav-label", "Sandboxes" }
                        if project_count > 0 {
                            span { class: "nav-count", "{project_count}" }
                        }
                    }

                    if is_expanded {
                        for name in projects.iter() {
                            {{
                                let is_active = page == Page::Sandboxes
                                    && selected_name.as_deref() == Some(name.as_str());
                                let n = name.clone();
                                let label = name.clone();
                                rsx! {
                                    button {
                                        key: "{label}",
                                        class: if is_active { "nav-sandbox active" } else { "nav-sandbox" },
                                        onclick: move |_| {
                                            selected_project.set(Some(n.clone()));
                                            current_page.set(Page::Sandboxes);
                                        },
                                        span { class: "nav-sandbox-dot" }
                                        span { class: "nav-sandbox-label", "{label}" }
                                    }
                                }
                            }}
                        }
                    }

                    button {
                        class: if page == Page::Clusters { "nav-link active" } else { "nav-link" },
                        onclick: move |_| {
                            selected_project.set(None);
                            current_page.set(Page::Clusters);
                        },
                        span { class: "nav-label", "Clusters" }
                    }
                }

                // ── Activity ──
                div { class: "nav-section",
                    div { class: "nav-section-label", "activity" }

                    button {
                        class: if page == Page::Runtime { "nav-link active" } else { "nav-link" },
                        onclick: move |_| current_page.set(Page::Runtime),
                        span { class: "nav-label", "Runtime" }
                    }
                    button {
                        class: if page == Page::Agents { "nav-link active" } else { "nav-link" },
                        onclick: move |_| current_page.set(Page::Agents),
                        span { class: "nav-label", "Agents" }
                    }
                    if show_agent_api_audit_tab {
                        button {
                            class: if page == Page::AgentApiAudit { "nav-link active" } else { "nav-link" },
                            onclick: move |_| {
                                selected_project.set(None);
                                current_page.set(Page::AgentApiAudit);
                            },
                            span { class: "nav-label", "Agent API" }
                        }
                    }
                }

                // ── System ──
                div { class: "nav-section",
                    div { class: "nav-section-label", "system" }

                    button {
                        class: if page == Page::Diagnostics { "nav-link active" } else { "nav-link" },
                        onclick: move |_| current_page.set(Page::Diagnostics),
                        span { class: "nav-label", "Diagnostics" }
                    }
                    button {
                        class: if page == Page::System { "nav-link active" } else { "nav-link" },
                        onclick: move |_| current_page.set(Page::System),
                        span { class: "nav-label", "System" }
                    }
                    button {
                        class: if page == Page::Settings { "nav-link active" } else { "nav-link" },
                        onclick: move |_| current_page.set(Page::Settings),
                        span { class: "nav-label", "Settings" }
                    }
                }
            }

            div { class: "sidebar-spacer" }

            div { class: "sidebar-footer",
                div { class: "sidebar-edition-row",
                    span { class: "sidebar-version", "{app_version_label}" }
                    span { class: "sidebar-edition-badge", "public" }
                }
                if let Some(release) = update_release {
                    button {
                        class: "sidebar-update-link",
                        onclick: {
                            let url = release.url.clone();
                            move |_| {
                                if updater_ready {
                                    if let Err(err) = check_for_updates() {
                                        eprintln!("Loopbox updater check warning: {err}");
                                        let _ = webbrowser::open(&url);
                                    }
                                } else {
                                    let _ = webbrowser::open(&url);
                                }
                            }
                        },
                        if updater_ready {
                            "Check for Updates..."
                        } else {
                            "Update {release.tag}"
                        }
                    }
                }
            }
        }
    }
}
