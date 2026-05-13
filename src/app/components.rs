use crate::app::models::Notice;
use crate::app::models::{DetailTab, Page};
use crate::app::utils::copy_to_clipboard;
use crate::loopbox::{DoctorFixAction, DoctorIssue, DoctorLevel, LoopboxConfig};
use dioxus::prelude::*;

#[component]
pub(super) fn ProjectCard(
    name: String,
    ip: String,
    primary_host: String,
    service_count: usize,
    running_count: usize,
    index: usize,
    mut selected_project: Signal<Option<String>>,
) -> Element {
    let delay = format!("{}ms", index * 40);
    let project_name = name.clone();

    let status_class = if running_count > 0 {
        "project-card-status project-card-status-active"
    } else {
        "project-card-status"
    };

    let status_text = if running_count > 0 {
        format!("{running_count}/{service_count} running")
    } else {
        "idle".to_string()
    };

    rsx! {
        button {
            class: "project-card",
            style: "animation-delay: {delay};",
            onclick: move |_| selected_project.set(Some(project_name.clone())),
            div { class: "project-card-header",
                h3 { class: "project-card-name", "{name}" }
                span { class: "project-card-ip", "{ip}" }
            }
            p { class: "project-card-host", "{primary_host}" }
            div { class: "project-card-footer",
                span { class: "{status_class}",
                    if running_count > 0 {
                        span { class: "status-dot status-dot-running" }
                    }
                    "{status_text}"
                }
                span { class: "project-card-svc-count", "{service_count} services" }
            }
        }
    }
}

#[component]
pub(super) fn DoctorIssueRow(
    issue: DoctorIssue,
    mut selected_project: Signal<Option<String>>,
    mut requested_detail_tab: Signal<Option<DetailTab>>,
    mut current_page: Signal<Page>,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let level_class = match issue.level {
        DoctorLevel::Error => "issue-error",
        DoctorLevel::Warning => "issue-warning",
        DoctorLevel::Info => "issue-info",
    };
    let level_label = match issue.level {
        DoctorLevel::Error => "error",
        DoctorLevel::Warning => "warn",
        DoctorLevel::Info => "info",
    };
    let location_project = issue
        .project
        .as_ref()
        .filter(|_| is_location_issue(&issue.message))
        .cloned();

    rsx! {
        li { class: "doctor-item {level_class}",
            span { class: "doctor-badge", "{level_label}" }
            if let Some(project) = issue.project.clone() {
                span { class: "doctor-project", "{project}" }
            }
            p { class: "doctor-message", "{issue.message}" }
            if let Some(project) = location_project {
                button {
                    class: "btn btn-sm btn-outline doctor-fix-btn",
                    onclick: move |_| {
                        current_page.set(Page::Sandboxes);
                        requested_detail_tab.set(Some(DetailTab::Config));
                        selected_project.set(Some(project.clone()));
                    },
                    "Edit Location"
                }
            }
            if let Some(fix) = issue.fix.clone() {
                {{
                    let fix_label = fix.label().to_string();
                    rsx! {
                        button {
                            class: "btn btn-sm btn-outline doctor-fix-btn",
                            onclick: move |_| match fix.clone() {
                                DoctorFixAction::ApplySystemSetup => {
                                    match crate::loopbox::apply_system_setup(&config()) {
                                        Ok(msg) => notice.set(Some(Notice::success(msg))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                }
                                DoctorFixAction::CopyCommand { command, label } => {
                                    match copy_to_clipboard(&command) {
                                        Ok(()) => notice.set(Some(Notice::success(format!(
                                            "Copied '{}' command.",
                                            label
                                        )))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                }
                            },
                            "{fix_label}"
                        }
                    }
                }}
            }
        }
    }
}

fn is_location_issue(message: &str) -> bool {
    message.starts_with("Directory '")
        || (message.starts_with("Service '")
            && message.contains("' workdir '")
            && message.ends_with(" does not exist."))
}

#[cfg(test)]
mod tests {
    use super::is_location_issue;

    #[test]
    fn detects_project_and_service_location_warnings() {
        assert!(is_location_issue(
            "Directory '/Users/niklas/development/loopbox-web' does not exist."
        ));
        assert!(is_location_issue(
            "Service 'loopbox-web' workdir '/Users/niklas/development/loopbox-web' does not exist."
        ));
        assert!(!is_location_issue("Reverse proxy is not running."));
    }
}
