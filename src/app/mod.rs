mod components;
pub(crate) mod log_window;
pub(crate) mod models;
mod pages;
pub(crate) mod runtime_view;
mod sidebar;
pub(crate) mod terminal_window;
mod tray;
mod utils;

use self::models::{Notice, Page, RuntimeFilter, SetupStatus};
use self::pages::{agent_api_audit, agents, runtime, sandboxes, settings, system};
use self::sidebar::render_sidebar;
use self::utils::{apply_setup_result, preview_project_name, preview_service_name, preview_suffix};
use crate::loopbox;
use crate::loopbox::{AddProjectInput, AgentApiServerInfo, DoctorLevel, LoopboxConfig};
use dioxus::prelude::*;
use std::collections::BTreeMap;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const PROXY_SIDECAR_REFRESH_TICKS: u64 = 5;

// ── App ──

#[component]
pub(crate) fn App() -> Element {
    // ── Core State ──
    let current_page = use_signal(|| Page::Sandboxes);
    let runtime_filter = use_signal(|| RuntimeFilter::All);
    let runtime_search = use_signal(String::new);
    let mut config = use_signal(|| {
        loopbox::load_config().unwrap_or_else(|err| {
            eprintln!("{err}");
            LoopboxConfig::default()
        })
    });
    let selected_project = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<Notice>);
    let mut runtime_tick = use_signal(|| 0_u64);
    let mut pending_auto_apply = use_signal(|| None::<String>);
    let doctor_refresh = use_signal(|| 0_u64);
    use_future(move || async move {
        let mut proxy_sidecar_tick = 0_u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
            proxy_sidecar_tick = proxy_sidecar_tick.wrapping_add(1);
            if proxy_sidecar_refresh_due(proxy_sidecar_tick) {
                let cfg = config();
                let result =
                    tokio::task::spawn_blocking(move || loopbox::sync_reverse_proxy_sidecar(&cfg))
                        .await
                        .map_err(|err| format!("Reverse proxy sidecar refresh task failed: {err}"))
                        .and_then(|result| result.map(|_| ()));
                if let Err(err) = result {
                    eprintln!("Loopbox reverse proxy sidecar refresh warning: {err}");
                }
            }
        }
    });

    let auto_apply_resource = use_resource(move || {
        let request = pending_auto_apply();
        let cfg = config();
        async move {
            let saved_message = request?;

            let apply_result =
                tokio::task::spawn_blocking(move || loopbox::apply_system_setup(&cfg))
                    .await
                    .map_err(|err| format!("Auto-apply task failed: {err}"))
                    .and_then(|result| result);

            Some((saved_message, apply_result))
        }
    });
    let latest_release_resource =
        use_resource(move || async move { loopbox::fetch_latest_github_release().await.ok() });
    let config_refresh_resource = use_resource(move || {
        let page = current_page();
        let should_poll = matches!(page, Page::Sandboxes | Page::AgentApiAudit);
        if should_poll {
            let _ = runtime_tick();
        }
        async move {
            if !should_poll {
                return None;
            }
            tokio::task::spawn_blocking(loopbox::load_config)
                .await
                .ok()
                .and_then(Result::ok)
        }
    });
    let overview_runtime_counts_resource = use_resource(move || {
        let page = current_page();
        let selected_name = selected_project();
        let should_poll = page == Page::Sandboxes && selected_name.is_none();
        let cfg = if should_poll { Some(config()) } else { None };
        let tick = if should_poll {
            Some(runtime_tick())
        } else {
            None
        };
        async move {
            let Some(cfg) = cfg else {
                return BTreeMap::new();
            };
            let _ = tick;
            tokio::task::spawn_blocking(move || overview_running_counts(&cfg))
                .await
                .unwrap_or_default()
        }
    });
    let doctor_report_resource = use_resource(move || {
        let page = current_page();
        let selected_name = selected_project();
        let should_check = page == Page::Sandboxes && selected_name.is_none();
        let cfg = if should_check { Some(config()) } else { None };
        let refresh = if should_check {
            Some(doctor_refresh())
        } else {
            None
        };
        async move {
            let Some(cfg) = cfg else {
                return Vec::new();
            };
            let _ = refresh;
            tokio::task::spawn_blocking(move || loopbox::doctor_report(&cfg))
                .await
                .unwrap_or_default()
        }
    });

    // ── Form State ──
    let add_form = use_signal(AddProjectInput::default);

    // ── Settings State ──
    let domain_suffix_input = use_signal(|| config.read().global.domain_suffix.clone());
    let range_start_input = use_signal(|| config.read().global.ip_range_start.to_string());
    let range_end_input = use_signal(|| config.read().global.ip_range_end.to_string());
    let proxy_capture_default_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .capture_enabled_by_default
    });
    let proxy_capture_mode_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .capture_mode_default
            .clone()
    });
    let proxy_capture_text_only_input =
        use_signal(|| config.read().global.proxy_traffic.capture_text_only);
    let proxy_redact_headers_input =
        use_signal(|| config.read().global.proxy_traffic.redact_headers.join(", "));
    let proxy_redact_query_keys_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .redact_query_keys
            .join(", ")
    });
    let proxy_retention_days_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .retention_days
            .to_string()
    });
    let proxy_max_storage_mb_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .max_storage_mb
            .to_string()
    });
    let proxy_writer_queue_size_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .writer_queue_size
            .to_string()
    });
    let proxy_request_body_preview_max_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .request_body_preview_max_bytes
            .to_string()
    });
    let proxy_response_body_preview_max_input = use_signal(|| {
        config
            .read()
            .global
            .proxy_traffic
            .response_body_preview_max_bytes
            .to_string()
    });
    let proxy_max_events_input =
        use_signal(|| config.read().global.proxy_traffic.max_events.to_string());
    let resource_metrics_enabled_input =
        use_signal(|| config.read().global.resource_metrics.enabled);
    let resource_metrics_sample_interval_input = use_signal(|| {
        config
            .read()
            .global
            .resource_metrics
            .sample_interval_secs
            .to_string()
    });
    let resource_metrics_retention_days_input = use_signal(|| {
        config
            .read()
            .global
            .resource_metrics
            .retention_days
            .to_string()
    });
    let resource_metrics_max_storage_mb_input = use_signal(|| {
        config
            .read()
            .global
            .resource_metrics
            .max_storage_mb
            .to_string()
    });
    let agent_api_enabled_input = use_signal(|| config.read().global.agent_api.enabled);
    let agent_api_auth_enabled_input = use_signal(|| config.read().global.agent_api.auth_enabled);
    let agent_api_port_input = use_signal(|| config.read().global.agent_api.port.to_string());

    // ── System State ──
    let setup_status = use_signal(|| None::<SetupStatus>);
    let show_setup_script = use_signal(|| false);
    let hosts_content = use_signal(String::new);
    let hosts_original = use_signal(String::new);
    let hosts_loaded = use_signal(|| false);

    // ── Derived: Global ──
    let config_snapshot = config();
    let selected_name = selected_project();
    let page = current_page();
    let runtime_filter_value = runtime_filter();

    // Sync long-lived background services only when config changes.
    let sync_services = use_memo(move || {
        let cfg = config();
        if let Err(err) = loopbox::sync_reverse_proxy_sidecar(&cfg) {
            eprintln!("Loopbox reverse proxy sidecar sync warning: {err}");
        }
        if let Err(err) = loopbox::sync_resource_metrics_sampler(&cfg) {
            eprintln!("Loopbox resource metrics sync warning: {err}");
        }
        loopbox::sync_agent_api_server(&cfg).ok()
    });

    let agent_api_info: Option<AgentApiServerInfo> = sync_services();
    let app_version_label = loopbox::app_version_label();
    let latest_release = latest_release_resource().flatten();
    let project_names: Vec<String> = config_snapshot.projects.keys().cloned().collect();

    let overview_running_counts = overview_runtime_counts_resource().unwrap_or_default();
    let doctor_issues = doctor_report_resource().unwrap_or_default();
    let doctor_ok = doctor_issues.iter().all(|i| i.level == DoctorLevel::Info);
    let doctor_issue_count = doctor_issues
        .iter()
        .filter(|i| i.level != DoctorLevel::Info)
        .count();

    let selected_project_data = selected_name.as_ref().and_then(|name| {
        config_snapshot
            .projects
            .get(name)
            .map(|project| (name.clone(), project.clone()))
    });

    use_effect(move || {
        if let Some((saved_message, apply_result)) = auto_apply_resource().flatten() {
            pending_auto_apply.set(None);
            apply_setup_result(
                apply_result.map(|message| format!("{saved_message} {message}")),
                "Applied",
                "Apply Failed",
                notice,
                setup_status,
            );
        }
    });
    use_effect(move || {
        if let Some(reloaded) = config_refresh_resource().flatten() {
            if *config.read() != reloaded {
                config.set(reloaded);
            }
        }
    });
    use_effect(move || {
        let Some(current_notice) = notice() else {
            return;
        };
        let dismiss_after = current_notice.dismiss_after();
        spawn(async move {
            tokio::time::sleep(dismiss_after).await;
            let should_clear = notice.with(|notice| notice.as_ref() == Some(&current_notice));
            if should_clear {
                notice.set(None);
            }
        });
    });

    // ── Derived: Form Previews ──
    let suffix_preview = if page == Page::Sandboxes || page == Page::NewSandbox {
        preview_suffix(&domain_suffix_input())
    } else {
        String::new()
    };
    let (add_form_snapshot, service_host_previews) = if page == Page::NewSandbox {
        let add_form_snapshot = add_form();
        let sandbox_name_preview = preview_project_name(&add_form_snapshot.name);
        let mut service_host_previews = Vec::new();
        for entry in &add_form_snapshot.services {
            let service_name = preview_service_name(&entry.name);
            if service_name.is_empty() {
                continue;
            }
            let host = format!("{service_name}.{sandbox_name_preview}.{suffix_preview}");
            if !service_host_previews.contains(&host) {
                service_host_previews.push(host);
            }
        }
        (add_form_snapshot, service_host_previews)
    } else {
        (AddProjectInput::default(), Vec::new())
    };

    // ── Derived: System ──
    let (
        setup_alias_count,
        setup_lines_count,
        setup_hosts_count,
        can_apply_setup,
        hosts_is_loaded,
        hosts_dirty,
        hosts_outside_danger,
        hosts_line_count,
        hosts_byte_count,
        hosts_preview,
        apply_script_preview,
        hosts_snapshot,
    ) = if page == Page::System {
        let hosts_preview = loopbox::managed_hosts_block(&config_snapshot);
        let apply_script_preview = loopbox::apply_script(&config_snapshot);
        let setup_alias_count = config_snapshot.projects.len();
        let setup_hosts_count: usize = config_snapshot
            .projects
            .values()
            .map(|p| p.services.len())
            .sum();
        let setup_lines_count = config_snapshot
            .projects
            .values()
            .filter(|p| !p.services.is_empty())
            .count();
        let can_apply_setup = !config_snapshot.projects.is_empty();
        let hosts_is_loaded = hosts_loaded();
        let hosts_snapshot = hosts_content();
        let hosts_original_snapshot = hosts_original();
        let hosts_dirty = hosts_is_loaded && hosts_snapshot != hosts_original_snapshot;
        let hosts_outside_danger = hosts_dirty
            && loopbox::has_changes_outside_managed_block(
                &hosts_original_snapshot,
                &hosts_snapshot,
            );
        let hosts_line_count = hosts_snapshot.lines().count();
        let hosts_byte_count = hosts_snapshot.len();

        (
            setup_alias_count,
            setup_lines_count,
            setup_hosts_count,
            can_apply_setup,
            hosts_is_loaded,
            hosts_dirty,
            hosts_outside_danger,
            hosts_line_count,
            hosts_byte_count,
            hosts_preview,
            apply_script_preview,
            hosts_snapshot,
        )
    } else {
        (
            0,
            0,
            0,
            false,
            false,
            false,
            false,
            0,
            0,
            String::new(),
            String::new(),
            String::new(),
        )
    };

    // ── Derived: Display Values ──
    let ip_base = if page == Page::Settings {
        config_snapshot.global.ip_base.clone()
    } else {
        String::new()
    };

    rsx! {
        document::Title { "Loopbox" }
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: MAIN_CSS }
        document::Stylesheet { href: "https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:ital,wght@0,400;0,500;0,600;0,700;1,400&family=JetBrains+Mono:wght@400;500;600&display=swap" }
        tray::MenuBarTrayController {
            config,
            agent_api_info: agent_api_info.clone(),
            current_page,
            selected_project,
            notice,
            runtime_tick,
        }

        div { class: "app-layout",
            div { class: "grid-bg" }

            {render_sidebar(
                page,
                project_names,
                current_page,
                selected_project,
                app_version_label,
                latest_release,
            )}

            main { class: "main-area",
                if let Some(current_notice) = notice() {
                    div {
                        class: format!("notice {}", current_notice.kind.class_name()),
                        onclick: move |_| notice.set(None),
                        "{current_notice.message}"
                    }
                }

                {sandboxes::render_sandboxes_page(
                    page,
                    selected_project_data,
                    overview_running_counts,
                    config_snapshot.clone(),
                    doctor_ok,
                    doctor_issue_count,
                    doctor_issues,
                    suffix_preview.clone(),
                    selected_project,
                    config,
                    notice,
                    pending_auto_apply,
                    runtime_tick,
                    doctor_refresh,
                    current_page,
                )}

                {sandboxes::render_new_sandbox_page(
                    page,
                    add_form_snapshot,
                    service_host_previews,
                    add_form,
                    selected_project,
                    config,
                    notice,
                    pending_auto_apply,
                    current_page,
                )}

                {system::render_system_page(
                    page,
                    setup_alias_count,
                    setup_lines_count,
                    setup_hosts_count,
                    can_apply_setup,
                    hosts_is_loaded,
                    hosts_dirty,
                    hosts_outside_danger,
                    hosts_line_count,
                    hosts_byte_count,
                    hosts_preview,
                    apply_script_preview,
                    hosts_snapshot,
                    config,
                    notice,
                    setup_status,
                    show_setup_script,
                    hosts_content,
                    hosts_original,
                    hosts_loaded,
                )}

                {runtime::render_runtime_page(
                    page,
                    current_page,
                    runtime_filter_value,
                    runtime_filter,
                    runtime_search,
                    config,
                    notice,
                    runtime_tick,
                )}

                {agents::render_agents_page(
                    page,
                    config,
                    selected_project,
                    notice,
                    runtime_tick,
                )}

                {agent_api_audit::render_agent_api_audit_page(
                    page,
                    notice,
                    runtime_tick,
                )}

                {settings::render_settings_page(
                    page,
                    ip_base,
                    domain_suffix_input,
                    range_start_input,
                    range_end_input,
                    proxy_capture_default_input,
                    proxy_capture_mode_input,
                    proxy_capture_text_only_input,
                    proxy_request_body_preview_max_input,
                    proxy_response_body_preview_max_input,
                    proxy_max_events_input,
                    proxy_redact_headers_input,
                    proxy_redact_query_keys_input,
                    proxy_retention_days_input,
                    proxy_max_storage_mb_input,
                    proxy_writer_queue_size_input,
                    resource_metrics_enabled_input,
                    resource_metrics_sample_interval_input,
                    resource_metrics_retention_days_input,
                    resource_metrics_max_storage_mb_input,
                    agent_api_enabled_input,
                    agent_api_auth_enabled_input,
                    agent_api_port_input,
                    agent_api_info,
                    config,
                    notice,
                    setup_status,
                    pending_auto_apply,
                )}
            }
        }
    }
}

fn overview_running_counts(config: &LoopboxConfig) -> BTreeMap<String, usize> {
    config
        .projects
        .iter()
        .map(|(name, project)| {
            let running = project
                .services
                .iter()
                .filter(|service| {
                    loopbox::service_runtime_status(config, name, &service.name)
                        .map(|status| {
                            matches!(
                                status.state,
                                loopbox::ServiceRuntimeState::Running
                                    | loopbox::ServiceRuntimeState::Starting
                            )
                        })
                        .unwrap_or(false)
                })
                .count();
            (name.clone(), running)
        })
        .collect()
}

fn proxy_sidecar_refresh_due(tick: u64) -> bool {
    tick > 0 && tick.is_multiple_of(PROXY_SIDECAR_REFRESH_TICKS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn proxy_sidecar_refresh_runs_before_keepalive_expires() {
        assert!(!super::proxy_sidecar_refresh_due(1));
        assert!(super::proxy_sidecar_refresh_due(5));
        assert!(super::proxy_sidecar_refresh_due(10));
        assert!(!super::proxy_sidecar_refresh_due(11));
    }
}
