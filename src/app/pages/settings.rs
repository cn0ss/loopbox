use crate::app::models::{Notice, Page, SetupStatus};
use crate::app::utils::copy_to_clipboard;
use crate::loopbox::{self, AgentApiServerInfo, LoopboxConfig, ProxyCaptureMode};
use dioxus::prelude::*;

mod helpers;
use helpers::*;
mod feature_sections;
use feature_sections::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_settings_page(
    page: Page,
    ip_base: String,
    mut domain_suffix_input: Signal<String>,
    mut range_start_input: Signal<String>,
    mut range_end_input: Signal<String>,
    proxy_capture_default_input: Signal<bool>,
    proxy_capture_mode_input: Signal<ProxyCaptureMode>,
    proxy_capture_text_only_input: Signal<bool>,
    proxy_request_body_preview_max_input: Signal<String>,
    proxy_response_body_preview_max_input: Signal<String>,
    proxy_max_events_input: Signal<String>,
    proxy_redact_headers_input: Signal<String>,
    proxy_redact_query_keys_input: Signal<String>,
    proxy_retention_days_input: Signal<String>,
    proxy_max_storage_mb_input: Signal<String>,
    proxy_writer_queue_size_input: Signal<String>,
    resource_metrics_enabled_input: Signal<bool>,
    resource_metrics_sample_interval_input: Signal<String>,
    resource_metrics_retention_days_input: Signal<String>,
    resource_metrics_max_storage_mb_input: Signal<String>,
    mut agent_api_enabled_input: Signal<bool>,
    mut agent_api_auth_enabled_input: Signal<bool>,
    mut agent_api_port_input: Signal<String>,
    agent_api_info: Option<AgentApiServerInfo>,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    _setup_status: Signal<Option<SetupStatus>>,
    pending_auto_apply: Signal<Option<String>>,
) -> Element {
    let agent_api_status_label = if let Some(info) = agent_api_info.as_ref() {
        if info.running {
            "running"
        } else {
            "stopped"
        }
    } else {
        "unknown"
    };
    let agent_api_base_url = agent_api_info
        .as_ref()
        .and_then(|info| info.base_url.clone())
        .unwrap_or_else(|| "n/a".to_string());
    let agent_api_openapi_url = agent_api_info
        .as_ref()
        .and_then(|info| info.openapi_url.clone())
        .unwrap_or_else(|| "n/a".to_string());
    let agent_api_discovery = agent_api_info
        .as_ref()
        .map(|info| info.discovery_path.clone())
        .unwrap_or_else(|| loopbox::agent_api_discovery_path().display().to_string());
    let agent_api_token = agent_api_info
        .as_ref()
        .and_then(|info| info.token_path.clone())
        .unwrap_or_else(|| "n/a".to_string());
    let agent_api_prompt = agent_api_bootstrap_prompt(
        &agent_api_base_url,
        &agent_api_openapi_url,
        &agent_api_discovery,
        agent_api_auth_enabled_input(),
        &agent_api_token,
    );
    let updater_can_check = loopbox::can_check_for_updates();
    let updater_status_label = if updater_can_check {
        "ready"
    } else {
        "browser fallback"
    };
    let updater_feed_url = loopbox::updater_feed_url().unwrap_or_else(|| "n/a".to_string());
    let updater_auto_checks_label = match loopbox::updater_automatic_checks_enabled() {
        Some(true) => "enabled",
        Some(false) => "disabled",
        None => "unknown",
    };
    let updater_last_checked_label =
        loopbox::updater_last_checked_utc().unwrap_or_else(|| "never".to_string());

    rsx! {
        if page == Page::Settings {
            div { class: "page",
                div { class: "page-header",
                    div { class: "page-header-left",
                        h1 { class: "page-title", "Settings" }
                    }
                }

                // ── Network ──────────────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⊞" }
                        div {
                            p { class: "settings-section-title", "Network" }
                            p { class: "settings-section-desc", "Sandbox domain names and IP address allocation." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-fields-3",
                            label { class: "field",
                                span { class: "field-label", "Domain Suffix" }
                                input {
                                    class: "field-input",
                                    value: "{domain_suffix_input}",
                                    placeholder: "localhost",
                                    oninput: move |evt| domain_suffix_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "IP Range Start" }
                                input {
                                    class: "field-input",
                                    value: "{range_start_input}",
                                    placeholder: "2",
                                    oninput: move |evt| range_start_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "IP Range End" }
                                input {
                                    class: "field-input",
                                    value: "{range_end_input}",
                                    placeholder: "254",
                                    oninput: move |evt| range_end_input.set(evt.value()),
                                }
                            }
                        }
                        p { class: "settings-hint",
                            "Sandboxes use IPs {ip_base}{range_start_input}\u{2013}{range_end_input} with the .{domain_suffix_input} TLD."
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let previous = config();
                                    let suffix = domain_suffix_input();
                                    let start = range_start_input();
                                    let end = range_end_input();

                                    let result = {
                                        let mut cfg = config.write();
                                        loopbox::update_global_settings(&mut cfg, &suffix, &start, &end).map(|_| {
                                            domain_suffix_input.set(cfg.global.domain_suffix.clone());
                                            range_start_input.set(cfg.global.ip_range_start.to_string());
                                            range_end_input.set(cfg.global.ip_range_end.to_string());
                                        })
                                    };

                                    match result {
                                        Ok(()) => save_settings_group(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            "Network settings updated.",
                                            Some(previous),
                                            true,
                                        ),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Save Network"
                            }
                        }
                    }
                }

                // ── Agent API ────────────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⌘" }
                        div {
                            p { class: "settings-section-title", "Agent API" }
                            p { class: "settings-section-desc", "Local HTTP API for Codex, Claude, Cursor and other tools." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-toggles",
                            div { class: "settings-toggle-row",
                                div { class: "settings-toggle-info",
                                    span { class: "settings-toggle-label", "Enable Agent API" }
                                    span { class: "settings-toggle-desc", "Starts a localhost HTTP server with project/runtime/log/request endpoints." }
                                }
                                button {
                                    class: if agent_api_enabled_input() {
                                        "toggle-pill toggle-pill-on"
                                    } else {
                                        "toggle-pill"
                                    },
                                    onclick: move |_| {
                                        let next = !agent_api_enabled_input();
                                        agent_api_enabled_input.set(next);
                                        config.with_mut(|cfg| {
                                            cfg.global.agent_api.enabled = next;
                                        });
                                        save_settings_group(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            "Agent API updated.",
                                            None,
                                            false,
                                        );
                                    },
                                    span { class: "toggle-pill-dot" }
                                    if agent_api_enabled_input() { "Enabled" } else { "Disabled" }
                                }
                            }
                            div { class: "settings-toggle-row settings-toggle-row-last",
                                div { class: "settings-toggle-info",
                                    span { class: "settings-toggle-label", "Require Auth" }
                                    span { class: "settings-toggle-desc", "Require bearer token on API calls. Disable only for trusted local workflows." }
                                }
                                button {
                                    class: if agent_api_auth_enabled_input() {
                                        "toggle-pill toggle-pill-on"
                                    } else {
                                        "toggle-pill"
                                    },
                                    onclick: move |_| {
                                        let next = !agent_api_auth_enabled_input();
                                        agent_api_auth_enabled_input.set(next);
                                        config.with_mut(|cfg| {
                                            cfg.global.agent_api.auth_enabled = next;
                                        });
                                        save_settings_group(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            "Agent API auth updated.",
                                            None,
                                            false,
                                        );
                                    },
                                    span { class: "toggle-pill-dot" }
                                    if agent_api_auth_enabled_input() { "Enabled" } else { "Disabled" }
                                }
                            }
                        }
                        div { class: "settings-sub-divider" }
                        div { class: "settings-fields-3",
                            label { class: "field",
                                span { class: "field-label", "Agent API Port" }
                                input {
                                    class: "field-input",
                                    value: "{agent_api_port_input}",
                                    placeholder: "39393",
                                    oninput: move |evt| agent_api_port_input.set(evt.value()),
                                }
                            }
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let port_raw = agent_api_port_input();
                                    let parsed_port = match port_raw.trim().parse::<u16>() {
                                        Ok(value) if value > 0 => Ok(value),
                                        Ok(_) => Err("Agent API port must be greater than 0.".to_string()),
                                        Err(_) => Err("Agent API port must be a number between 1 and 65535.".to_string()),
                                    };

                                    match parsed_port {
                                        Ok(port) => {
                                            config.with_mut(|cfg| {
                                                cfg.global.agent_api.port = port;
                                            });
                                            agent_api_port_input.set(port.to_string());
                                            save_settings_group(
                                                config,
                                                notice,
                                                pending_auto_apply,
                                                "Agent API port updated.",
                                                None,
                                                false,
                                            );
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Save Agent API"
                            }
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Status" }
                                span { class: "settings-toggle-desc", "{agent_api_status_label}" }
                            }
                            div {}
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Base URL" }
                                span { class: "settings-toggle-desc", "{agent_api_base_url}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let value = agent_api_base_url.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied base URL."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy"
                            }
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "OpenAPI URL" }
                                span { class: "settings-toggle-desc", "{agent_api_openapi_url}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let value = agent_api_openapi_url.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied OpenAPI URL."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy"
                            }
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Discovery File" }
                                span { class: "settings-toggle-desc", "{agent_api_discovery}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let value = agent_api_discovery.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied discovery file path."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy"
                            }
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Token File" }
                                span { class: "settings-toggle-desc", "{agent_api_token}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let value = agent_api_token.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied token file path."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy"
                            }
                        }
                        div { class: "settings-sub-divider" }
                        label { class: "field",
                            span { class: "field-label", "Agent Prompt (copy into new agents)" }
                            textarea {
                                class: "field-input",
                                value: "{agent_api_prompt}",
                                rows: "8",
                                readonly: true,
                            }
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-outline",
                                onclick: {
                                    let value = agent_api_prompt.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied agent prompt."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy Agent Prompt"
                            }
                        }
                    }
                }

                // ── Updates ──────────────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⇪" }
                        div {
                            p { class: "settings-section-title", "Updates" }
                            p { class: "settings-section-desc", "In-app updates via Sparkle in macOS release builds." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Status" }
                                span { class: "settings-toggle-desc", "{updater_status_label}" }
                            }
                            div {}
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Feed URL" }
                                span { class: "settings-toggle-desc", "{updater_feed_url}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let value = updater_feed_url.clone();
                                    move |_| match copy_to_clipboard(&value) {
                                        Ok(()) => notice.set(Some(Notice::success("Copied feed URL."))),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Copy"
                            }
                        }
                        div { class: "settings-toggle-row",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Automatic Checks" }
                                span { class: "settings-toggle-desc", "{updater_auto_checks_label}" }
                            }
                            div {}
                        }
                        div { class: "settings-toggle-row settings-toggle-row-last",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Last Checked (UTC)" }
                                span { class: "settings-toggle-desc", "{updater_last_checked_label}" }
                            }
                            div {}
                        }
                        div { class: "settings-save-row",
                            button {
                                class: if updater_can_check { "btn btn-primary" } else { "btn btn-outline" },
                                onclick: move |_| {
                                    match loopbox::check_for_updates() {
                                        Ok(()) => {
                                            notice.set(Some(Notice::info("Checking for updates...")));
                                        }
                                        Err(err) => {
                                            notice.set(Some(Notice::error(format!(
                                                "Could not start in-app update check: {err}"
                                            ))));
                                        }
                                    }
                                },
                                "Check for Updates..."
                            }
                            button {
                                class: "btn btn-outline",
                                onclick: move |_| {
                                    let fallback_url = loopbox::latest_release_page_url();
                                    match webbrowser::open(&fallback_url) {
                                        Ok(_) => notice.set(Some(Notice::info("Opened releases page."))),
                                        Err(err) => {
                                            notice.set(Some(Notice::error(format!(
                                                "Failed to open releases page: {err}"
                                            ))));
                                        }
                                    }
                                },
                                "Open Releases Page"
                            }
                        }
                    }
                }

                SettingsFeatureSections {
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
                    config,
                    notice,
                    pending_auto_apply,
                }


            }
        }
    }
}
