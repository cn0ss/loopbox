#[macro_export]
macro_rules! define_loopbox_settings_paid_sections {
    () => {
        use super::*;

        #[component]
        pub(super) fn EeSettingsSections(
            mut proxy_capture_default_input: Signal<bool>,
            mut proxy_capture_mode_input: Signal<ProxyCaptureMode>,
            mut proxy_capture_text_only_input: Signal<bool>,
            mut proxy_request_body_preview_max_input: Signal<String>,
            mut proxy_response_body_preview_max_input: Signal<String>,
            mut proxy_max_events_input: Signal<String>,
            mut proxy_redact_headers_input: Signal<String>,
            mut proxy_redact_query_keys_input: Signal<String>,
            mut proxy_retention_days_input: Signal<String>,
            mut proxy_max_storage_mb_input: Signal<String>,
            mut proxy_writer_queue_size_input: Signal<String>,
            mut config: Signal<LoopboxConfig>,
            mut notice: Signal<Option<Notice>>,
            pending_auto_apply: Signal<Option<String>>,
        ) -> Element {
            let mut license_key_input = use_signal(String::new);
            let mut support_email_input = use_signal(String::new);
            let mut support_subject_input = use_signal(String::new);
            let mut support_text_input = use_signal(String::new);
            let license_tier_label = match loopbox::current_license_tier() {
                loopbox::LicenseTier::None => "Free (personal use)",
                loopbox::LicenseTier::Commercial => "Commercial (licensed)",
            };
            let license_activation_available = loopbox::license_activation_available();
            let traffic_capture_enabled = true;
            let edition_label = loopbox::edition_label();
            let priority_support_enabled = true;
            let support_form_valid = !support_email_input().trim().is_empty()
                && !support_subject_input().trim().is_empty()
                && !support_text_input().trim().is_empty();

            rsx! {
                // ── License ──────────────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "◈" }
                        div {
                            p { class: "settings-section-title", "License" }
                            p { class: "settings-section-desc", "Commercial license activation. Free for personal use." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-toggle-row settings-toggle-row-last",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Edition" }
                                span { class: "settings-toggle-desc", "{license_tier_label}" }
                            }
                            div {}
                        }
                        div { class: "settings-sub-divider" }
                        label { class: "field",
                            span { class: "field-label", "License Key" }
                            input {
                                class: "field-input",
                                value: "{license_key_input}",
                                placeholder: "lbx-com-...",
                                oninput: move |evt| license_key_input.set(evt.value()),
                            }
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let license_key = license_key_input();
                                    if license_key.trim().is_empty() {
                                        notice.set(Some(Notice::error("Enter a license key.".to_string())));
                                        return;
                                    }
                                    match loopbox::activate_license_key(&license_key) {
                                        Ok(()) => {
                                            license_key_input.set(String::new());
                                            notice.set(Some(Notice::success("License activated. Edition: Commercial.".to_string())));
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Activate License"
                            }
                        }
                        p { class: "settings-hint",
                            "Activate a commercial license key to use Loopbox for work. Get one at loopbox.tech/pricing."
                        }
                    }
                }

                // ── Priority Support ─────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⚑" }
                        div {
                            p { class: "settings-section-title", "Priority Support" }
                            p { class: "settings-section-desc", "Fast-track support for commercial license holders." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-toggle-row settings-toggle-row-last",
                            div { class: "settings-toggle-info",
                                span { class: "settings-toggle-label", "Availability" }
                                span { class: "settings-toggle-desc",
                                    if priority_support_enabled {
                                        "available"
                                    } else {
                                        "available in Ultimate"
                                    }
                                }
                            }
                            div {}
                        }
                        label { class: "field",
                            span { class: "field-label", "Email" }
                            input {
                                class: "field-input",
                                value: "{support_email_input}",
                                placeholder: "you@company.com",
                                disabled: !priority_support_enabled,
                                oninput: move |evt| support_email_input.set(evt.value()),
                            }
                        }
                        label { class: "field",
                            span { class: "field-label", "Subject" }
                            input {
                                class: "field-input",
                                value: "{support_subject_input}",
                                placeholder: "What do you need help with?",
                                disabled: !priority_support_enabled,
                                oninput: move |evt| support_subject_input.set(evt.value()),
                            }
                        }
                        label { class: "field field-wide",
                            span { class: "field-label", "Text" }
                            textarea {
                                class: "field-input field-textarea",
                                value: "{support_text_input}",
                                placeholder: "Describe your issue, steps to reproduce, and expected behavior.",
                                disabled: !priority_support_enabled,
                                oninput: move |evt| support_text_input.set(evt.value()),
                            }
                        }
                        div { class: "settings-save-row",
                            button {
                                class: if priority_support_enabled {
                                    "btn btn-primary"
                                } else {
                                    "btn btn-outline"
                                },
                                disabled: priority_support_enabled && !support_form_valid,
                                onclick: move |_| {
                                    if !priority_support_enabled {
                                        notice.set(Some(Notice::info(
                                            "Priority support is available in Ultimate.".to_string(),
                                        )));
                                        return;
                                    }

                                    let email = support_email_input();
                                    let subject = support_subject_input();
                                    let text = support_text_input();
                                    match loopbox::submit_priority_support_ticket(&email, &subject, &text) {
                                        Ok(_) => {
                                            support_subject_input.set(String::new());
                                            support_text_input.set(String::new());
                                            notice.set(Some(Notice::success(
                                                "Priority support ticket submitted.".to_string(),
                                            )));
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                if priority_support_enabled {
                                    "Submit Priority Ticket"
                                } else {
                                    "Submit Priority Ticket (Ultimate)"
                                }
                            }
                        }
                        p { class: "settings-hint",
                            if priority_support_enabled {
                                "Tickets are submitted directly in-app (no browser redirect)."
                            } else {
                                "Available in Ultimate. Upgrade to unlock in-app ticket support."
                            }
                        }
                    }
                }

                // ── Traffic Capture ───────────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⊙" }
                        div {
                            p { class: "settings-section-title", "Traffic Capture" }
                            p { class: "settings-section-desc", "Default behavior for recording HTTP traffic in sandboxes." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-toggles",
                            div { class: "settings-toggle-row",
                                div { class: "settings-toggle-info",
                                    span { class: "settings-toggle-label", "Capture by Default" }
                                    span { class: "settings-toggle-desc", "New sandboxes will have traffic capture enabled." }
                                }
                                button {
                                    class: if proxy_capture_default_input() {
                                        "toggle-pill toggle-pill-on"
                                    } else {
                                        "toggle-pill"
                                    },
                                    onclick: move |_| {
                                        proxy_capture_default_input.set(!proxy_capture_default_input());
                                    },
                                    span { class: "toggle-pill-dot" }
                                    if proxy_capture_default_input() { "Enabled" } else { "Disabled" }
                                }
                            }
                            div { class: "settings-toggle-row",
                                div { class: "settings-toggle-info",
                                    span { class: "settings-toggle-label", "Text Only" }
                                    span { class: "settings-toggle-desc", "Skip binary content types — images, archives, compiled assets." }
                                }
                                button {
                                    class: if proxy_capture_text_only_input() {
                                        "toggle-pill toggle-pill-on"
                                    } else {
                                        "toggle-pill"
                                    },
                                    onclick: move |_| {
                                        proxy_capture_text_only_input.set(!proxy_capture_text_only_input());
                                    },
                                    span { class: "toggle-pill-dot" }
                                    if proxy_capture_text_only_input() { "Enabled" } else { "Disabled" }
                                }
                            }
                            div { class: "settings-toggle-row settings-toggle-row-last",
                                div { class: "settings-toggle-info",
                                    span { class: "settings-toggle-label", "Capture Mode" }
                                    span { class: "settings-toggle-desc", "Detail level recorded for each request." }
                                }
                                div { class: "seg-control",
                                    button {
                                        class: if proxy_capture_mode_input() == ProxyCaptureMode::Metadata {
                                            "seg-btn seg-btn-on"
                                        } else {
                                            "seg-btn"
                                        },
                                        disabled: !traffic_capture_enabled,
                                        onclick: move |_| proxy_capture_mode_input.set(ProxyCaptureMode::Metadata),
                                        "Metadata"
                                    }
                                    button {
                                        class: if proxy_capture_mode_input() == ProxyCaptureMode::Headers {
                                            "seg-btn seg-btn-on"
                                        } else {
                                            "seg-btn"
                                        },
                                        disabled: !traffic_capture_enabled,
                                        onclick: move |_| proxy_capture_mode_input.set(ProxyCaptureMode::Headers),
                                        "Headers"
                                    }
                                    button {
                                        class: if proxy_capture_mode_input() == ProxyCaptureMode::BodyPreview {
                                            "seg-btn seg-btn-on"
                                        } else {
                                            "seg-btn"
                                        },
                                        disabled: !traffic_capture_enabled,
                                        onclick: move |_| proxy_capture_mode_input.set(ProxyCaptureMode::BodyPreview),
                                        "Body Preview"
                                    }
                                }
                            }
                        }
                        div { class: "settings-sub-divider" }
                        div { class: "settings-fields-3",
                            label { class: "field",
                                span { class: "field-label", "Max Events" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_max_events_input}",
                                    placeholder: "2000",
                                    oninput: move |evt| proxy_max_events_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "Req Body Preview (bytes)" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_request_body_preview_max_input}",
                                    placeholder: "65536",
                                    oninput: move |evt| proxy_request_body_preview_max_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "Resp Body Preview (bytes)" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_response_body_preview_max_input}",
                                    placeholder: "131072",
                                    oninput: move |evt| proxy_response_body_preview_max_input.set(evt.value()),
                                }
                            }
                        }
                        p { class: "settings-hint",
                            "Max events: 100–100,000. Body preview: 256–1,048,576 bytes."
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let traffic_default = proxy_capture_default_input();
                                    let requested_capture_mode = proxy_capture_mode_input();
                                    let capture_mode_default =
                                        loopbox::enforce_traffic_capture_mode(requested_capture_mode.clone());
                                    let capture_mode_adjusted =
                                        capture_mode_default != requested_capture_mode;
                                    let capture_text_only = proxy_capture_text_only_input();
                                    let max_events_raw = proxy_max_events_input();
                                    let request_body_preview_raw = proxy_request_body_preview_max_input();
                                    let response_body_preview_raw = proxy_response_body_preview_max_input();

                                    let parsed_max_events = match max_events_raw.trim().parse::<usize>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(100, 100_000)),
                                        Ok(_) => Err("Traffic max events must be greater than 0.".to_string()),
                                        Err(_) => Err("Traffic max events must be a number between 100 and 100000.".to_string()),
                                    };
                                    let parsed_request_body_preview = match request_body_preview_raw.trim().parse::<usize>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(256, 1024 * 1024)),
                                        Ok(_) => Err("Request body preview max must be greater than 0.".to_string()),
                                        Err(_) => Err("Request body preview max must be a number between 256 and 1048576.".to_string()),
                                    };
                                    let parsed_response_body_preview = match response_body_preview_raw.trim().parse::<usize>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(256, 1024 * 1024)),
                                        Ok(_) => Err("Response body preview max must be greater than 0.".to_string()),
                                        Err(_) => Err("Response body preview max must be a number between 256 and 1048576.".to_string()),
                                    };

                                    let update_result = match (
                                        parsed_max_events,
                                        parsed_request_body_preview,
                                        parsed_response_body_preview,
                                    ) {
                                        (Ok(max_events), Ok(request_body_preview_max), Ok(response_body_preview_max)) => {
                                            config.with_mut(|cfg| {
                                                cfg.global.proxy_traffic.capture_enabled_by_default = traffic_default;
                                                cfg.global.proxy_traffic.capture_mode_default =
                                                    capture_mode_default.clone();
                                                cfg.global.proxy_traffic.capture_text_only = capture_text_only;
                                                cfg.global.proxy_traffic.capture_body_preview =
                                                    cfg.global.proxy_traffic.capture_mode_default == ProxyCaptureMode::BodyPreview;
                                                cfg.global.proxy_traffic.max_events = max_events;
                                                cfg.global.proxy_traffic.request_body_preview_max_bytes = request_body_preview_max;
                                                cfg.global.proxy_traffic.response_body_preview_max_bytes = response_body_preview_max;
                                            });
                                            proxy_capture_mode_input.set(capture_mode_default);
                                            proxy_max_events_input.set(max_events.to_string());
                                            proxy_request_body_preview_max_input.set(request_body_preview_max.to_string());
                                            proxy_response_body_preview_max_input.set(response_body_preview_max.to_string());
                                            Ok(capture_mode_adjusted)
                                        }
                                        (Err(err), _, _) => Err(err),
                                        (_, Err(err), _) => Err(err),
                                        (_, _, Err(err)) => Err(err),
                                    };

                                    match update_result {
                                        Ok(capture_mode_adjusted) => {
                                            let success_message = if capture_mode_adjusted {
                                                "Traffic capture settings updated. Metadata mode is enforced in Core edition."
                                            } else {
                                                "Traffic capture settings updated."
                                            };
                                            save_settings_group(
                                                config,
                                                notice,
                                                pending_auto_apply,
                                                success_message,
                                                None,
                                                false,
                                            )
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Save Traffic Capture"
                            }
                        }
                    }
                }

                // ── Storage & Retention ───────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "⊟" }
                        div {
                            p { class: "settings-section-title", "Storage & Retention" }
                            p { class: "settings-section-desc", "How long traffic data is kept and how much disk space it may use." }
                        }
                    }
                    div { class: "settings-section-body",
                        div { class: "settings-fields-3",
                            label { class: "field",
                                span { class: "field-label", "Retention (days)" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_retention_days_input}",
                                    placeholder: "7",
                                    oninput: move |evt| proxy_retention_days_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "Max Storage (MB)" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_max_storage_mb_input}",
                                    placeholder: "500",
                                    oninput: move |evt| proxy_max_storage_mb_input.set(evt.value()),
                                }
                            }
                            label { class: "field",
                                span { class: "field-label", "Writer Queue Size" }
                                input {
                                    class: "field-input",
                                    value: "{proxy_writer_queue_size_input}",
                                    placeholder: "10000",
                                    oninput: move |evt| proxy_writer_queue_size_input.set(evt.value()),
                                }
                            }
                        }
                        p { class: "settings-hint",
                            "Retention: 1–90 days. Storage: 50–10,000 MB. Queue size: 100–100,000."
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let retention_days_raw = proxy_retention_days_input();
                                    let max_storage_mb_raw = proxy_max_storage_mb_input();
                                    let writer_queue_size_raw = proxy_writer_queue_size_input();

                                    let parsed_retention_days = match retention_days_raw.trim().parse::<u16>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(1, 90)),
                                        Ok(_) => Err("Retention days must be greater than 0.".to_string()),
                                        Err(_) => Err("Retention days must be a number between 1 and 90.".to_string()),
                                    };
                                    let parsed_max_storage_mb = match max_storage_mb_raw.trim().parse::<usize>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(50, 10_000)),
                                        Ok(_) => Err("Max storage must be greater than 0.".to_string()),
                                        Err(_) => Err("Max storage must be a number between 50 and 10000.".to_string()),
                                    };
                                    let parsed_writer_queue_size = match writer_queue_size_raw.trim().parse::<usize>() {
                                        Ok(value) if value > 0 => Ok(value.clamp(100, 100_000)),
                                        Ok(_) => Err("Writer queue size must be greater than 0.".to_string()),
                                        Err(_) => Err("Writer queue size must be a number between 100 and 100000.".to_string()),
                                    };

                                    let update_result = match (
                                        parsed_retention_days,
                                        parsed_max_storage_mb,
                                        parsed_writer_queue_size,
                                    ) {
                                        (Ok(retention_days), Ok(max_storage_mb), Ok(writer_queue_size)) => {
                                            config.with_mut(|cfg| {
                                                cfg.global.proxy_traffic.retention_days = retention_days;
                                                cfg.global.proxy_traffic.max_storage_mb = max_storage_mb;
                                                cfg.global.proxy_traffic.writer_queue_size = writer_queue_size;
                                            });
                                            proxy_retention_days_input.set(retention_days.to_string());
                                            proxy_max_storage_mb_input.set(max_storage_mb.to_string());
                                            proxy_writer_queue_size_input.set(writer_queue_size.to_string());
                                            Ok(())
                                        }
                                        (Err(err), _, _) => Err(err),
                                        (_, Err(err), _) => Err(err),
                                        (_, _, Err(err)) => Err(err),
                                    };

                                    match update_result {
                                        Ok(()) => save_settings_group(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            "Storage settings updated.",
                                            None,
                                            false,
                                        ),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Save Storage & Retention"
                            }
                        }
                    }
                }

                // ── Privacy & Redaction ───────────────────────────
                div { class: "settings-section",
                    div { class: "settings-section-head",
                        span { class: "settings-section-icon", "◈" }
                        div {
                            p { class: "settings-section-title", "Privacy & Redaction" }
                            p { class: "settings-section-desc", "Sensitive values matching these keys are masked in captured traffic." }
                        }
                    }
                    div { class: "settings-section-body",
                        label { class: "field",
                            span { class: "field-label", "Redact Headers" }
                            input {
                                class: "field-input",
                                value: "{proxy_redact_headers_input}",
                                placeholder: "authorization, cookie, set-cookie, x-api-key, proxy-authorization",
                                oninput: move |evt| proxy_redact_headers_input.set(evt.value()),
                            }
                        }
                        label { class: "field",
                            span { class: "field-label", "Redact Query Keys" }
                            input {
                                class: "field-input",
                                value: "{proxy_redact_query_keys_input}",
                                placeholder: "token, key, secret, password, code",
                                oninput: move |evt| proxy_redact_query_keys_input.set(evt.value()),
                            }
                        }
                        p { class: "settings-hint",
                            "Comma-separated lists. Keys are matched case-insensitively and shown as [REDACTED]."
                        }
                        div { class: "settings-save-row",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    let redact_headers = parse_redaction_list(&proxy_redact_headers_input());
                                    let redact_query_keys = parse_redaction_list(&proxy_redact_query_keys_input());
                                    let parsed_redact_headers = if redact_headers.is_empty() {
                                        Err("Redact headers list must contain at least one key.".to_string())
                                    } else {
                                        Ok(redact_headers)
                                    };
                                    let parsed_redact_query_keys = if redact_query_keys.is_empty() {
                                        Err("Redact query keys list must contain at least one key.".to_string())
                                    } else {
                                        Ok(redact_query_keys)
                                    };

                                    let update_result = match (parsed_redact_headers, parsed_redact_query_keys) {
                                        (Ok(headers), Ok(query_keys)) => {
                                            config.with_mut(|cfg| {
                                                cfg.global.proxy_traffic.redact_headers = headers.clone();
                                                cfg.global.proxy_traffic.redact_query_keys = query_keys.clone();
                                            });
                                            proxy_redact_headers_input.set(headers.join(", "));
                                            proxy_redact_query_keys_input.set(query_keys.join(", "));
                                            Ok(())
                                        }
                                        (Err(err), _) => Err(err),
                                        (_, Err(err)) => Err(err),
                                    };

                                    match update_result {
                                        Ok(()) => save_settings_group(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            "Privacy settings updated.",
                                            None,
                                            false,
                                        ),
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Save Privacy & Redaction"
                            }
                        }
                    }
                }
            }
        }
    };
}

#[macro_export]
macro_rules! define_loopbox_sandboxes_project_detail_paid {
    () => {
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

        pub(super) fn project_detail_service_runtime_label(service: &ServiceConfig) -> Option<&'static str> {
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
        pub(super) fn ProjectDetailEeServiceEditFields(
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
        pub(super) fn ProjectDetailEeTrafficTab(
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
                let mut events =
                    loopbox::proxy_traffic_events_for_project(&pn_traffic, 300).unwrap_or_default();
                if let Some(proj) = cfg.projects.get(&pn_traffic) {
                    if let Some(selected_service) =
                        effective_log_selection(proj.services.as_slice(), filter)
                    {
                        events.retain(|event| event.service_name == selected_service);
                    }
                }
                events
            });

            let traffic_snapshot = traffic_events();
            let selected_traffic_service =
                effective_log_selection(project.services.as_slice(), traffic_filter());
            let capture_enabled = loopbox::project_proxy_traffic_enabled(&config(), &project_name);
            let capture_mode = loopbox::project_proxy_traffic_capture_mode(&config(), &project_name);
            let traffic_disk_stats = loopbox::proxy_traffic_disk_stats();
            let traffic_capture_enabled = true;
            let har_export_enabled = true;
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
                                        if let Err(err) = loopbox::sync_reverse_proxy(&config()) {
                                            eprintln!("Loopbox reverse proxy sync warning: {err}");
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
                                        if let Err(err) = loopbox::sync_reverse_proxy(&config()) {
                                            eprintln!("Loopbox reverse proxy sync warning: {err}");
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
                                if har_export_enabled { "Export" } else { "Export (Pro)" }
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

                    if !traffic_capture_enabled {
                        p { class: "text-dim",
                            "Traffic tab is read-only in Core. Upgrade to Pro or Ultimate to enable capture controls and exports."
                        }
                    }

                    div { class: "traffic-split",
                        div { class: "traffic-list-pane",
                            if traffic_snapshot.is_empty() {
                                div { class: "traffic-list-empty",
                                    if traffic_capture_enabled {
                                        p { "No captured traffic yet." }
                                        p { class: "text-dim", "Enable capture and open a service URL." }
                                    } else {
                                        p { "Traffic capture is a paid add-on." }
                                        p { class: "text-dim", "Upgrade to Pro or Ultimate to record and inspect HTTP/gRPC traffic." }
                                    }
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
                                        span { class: format!("traffic-status-code {}", traffic_status_class(&event)), "{traffic_status_label(event)}" }
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

        fn format_request_body_render(event: &ProxyTrafficEvent, mode: TrafficBodyViewMode) -> TrafficBodyRender {
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
    };
}

#[macro_export]
macro_rules! define_loopbox_sandboxes_wizard_paid {
    () => {
        use super::*;
        use crate::loopbox::ServiceRuntimeKind;

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

        fn service_entry_is_ready(entry: &ServiceEntry) -> bool {
            if service_entry_requires_command(entry) {
                !entry.command.trim().is_empty()
            } else {
                !entry.container_image.trim().is_empty()
            }
        }

        pub(super) fn wizard_service_entry_is_ready(entry: &ServiceEntry) -> bool {
            service_entry_is_ready(entry)
        }

        pub(super) fn wizard_service_step_subtitle() -> &'static str {
            "Edit services. For process runtime, assign commands. For container runtime, set image and optional Docker options."
        }

        pub(super) fn wizard_service_requirement_hint() -> &'static str {
            "Each named service needs a command (process) or image (container)."
        }

        pub(super) fn wizard_blank_service_entry_base() -> ServiceEntry {
            ServiceEntry {
                name: String::new(),
                ports: vec![blank_service_port_entry()],
                port: String::new(),
                protocol: "http1".to_string(),
                runtime: "process".to_string(),
                command: String::new(),
                workdir: String::new(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: true,
            }
        }

        pub(super) fn wizard_discovered_service_entry(
            name: String,
            port: Option<u16>,
            suggestion: &loopbox::DiscoverySuggestion,
        ) -> ServiceEntry {
            ServiceEntry {
                name,
                ports: vec![ServicePortEntry {
                    port: port.map(|value| value.to_string()).unwrap_or_default(),
                    protocol: "http1".to_string(),
                    health_path: String::new(),
                }],
                port: port.map(|value| value.to_string()).unwrap_or_default(),
                protocol: "http1".to_string(),
                runtime: "process".to_string(),
                command: suggestion.command.clone(),
                workdir: suggestion.workdir.clone(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: true,
            }
        }

        #[component]
        pub(super) fn WizardEeServiceRuntimeFields(
            service_index: usize,
            entry: ServiceEntry,
            mut add_form: Signal<AddProjectInput>,
        ) -> Element {
            let selected_runtime = service_entry_runtime(&entry);
            let requires_command = !matches!(selected_runtime, ServiceRuntimeKind::Container);

            rsx! {
                label { class: "field",
                    span { "Runtime" }
                    select {
                        value: "{service_runtime_value(selected_runtime)}",
                        onchange: move |evt: Event<FormData>| {
                            let selected_runtime = parse_service_runtime(&evt.value());
                            let runtime = service_runtime_value(selected_runtime);
                            add_form.write().services[service_index].runtime = runtime.to_string();
                        },
                        option { value: "process", "Process" }
                        option {
                            value: "container",
                            "Container"
                        }
                    }
                }
                if requires_command {
                    label { class: "field field-wide",
                        span { "Command" }
                        input {
                            value: "{entry.command}",
                            placeholder: "pnpm dev",
                            oninput: move |evt: Event<FormData>| {
                                add_form.write().services[service_index].command =
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
                                add_form.write().services[service_index].container_image = evt.value();
                            },
                        }
                    }
                    label { class: "field field-wide",
                        span { "Container Args" }
                        input {
                            value: "{entry.container_args}",
                            placeholder: "-c shared_buffers=256MB, -c max_connections=200",
                            oninput: move |evt: Event<FormData>| {
                                add_form.write().services[service_index].container_args = evt.value();
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
                                add_form.write().services[service_index].container_env = evt.value();
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
                                add_form.write().services[service_index].container_volumes = evt.value();
                            },
                        }
                    }
                    button {
                        class: if entry.container_auto_remove { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                        onclick: move |_| {
                            let current = add_form().services[service_index].container_auto_remove;
                            add_form.write().services[service_index].container_auto_remove = !current;
                        },
                        if entry.container_auto_remove { "Container auto-remove: on" } else { "Container auto-remove: off" }
                    }
                }
            }
        }
    };
}

#[macro_export]
macro_rules! define_loopbox_agent_api_audit_paid {
    () => {
        use super::*;

        pub(super) fn sidebar_show_agent_api_audit_tab() -> bool {
            true
        }

        #[component]
        pub(super) fn AgentApiAuditPage(
            mut notice: Signal<Option<Notice>>,
            mut runtime_tick: Signal<u64>,
        ) -> Element {
            let _tick = runtime_tick();
            let mut selected_event_key = use_signal(|| None::<String>);
            let events = crate::loopbox::agent_api_audit_events(500).unwrap_or_default();
            let selected_event = selected_event_key()
                .and_then(|selected_key| {
                    events
                        .iter()
                        .find(|event| {
                            format!("{}-{}", event.id, event.started_at_unix_ms) == selected_key
                        })
                        .cloned()
                })
                .or_else(|| events.first().cloned());

            rsx! {
                div { class: "page",
                    div { class: "page-header",
                        div { class: "page-header-left",
                            h1 { class: "page-title", "Agent API Audit" }
                            p { class: "text-dim", "All local Agent API request/response records, including headers and bodies." }
                        }
                        div { class: "panel-actions",
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: move |_| {
                                    runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                },
                                "Refresh"
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: move |_| {
                                    match crate::loopbox::clear_agent_api_audit_events() {
                                        Ok(removed) => {
                                            selected_event_key.set(None);
                                            notice.set(Some(Notice::info(format!(
                                                "Cleared {removed} Agent API audit event(s)."
                                            ))));
                                            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                "Clear"
                            }
                        }
                    }

                    if events.is_empty() {
                        div { class: "panel",
                            p { "No Agent API requests captured yet." }
                            p { class: "text-dim", "Use the local Agent API and click Refresh." }
                        }
                    } else {
                        div { class: "traffic-split",
                            div { class: "traffic-list-pane",
                                div { class: "traffic-list-header",
                                    span { class: "traffic-list-count", "{events.len()} requests" }
                                }
                                for event in &events {
                                    div {
                                        key: "agent-api-audit-{event.id}-{event.started_at_unix_ms}",
                                        class: if selected_event_key()
                                            == Some(format!("{}-{}", event.id, event.started_at_unix_ms)) {
                                            "traffic-row traffic-row-selected"
                                        } else {
                                            "traffic-row"
                                        },
                                        onclick: {
                                            let event_key =
                                                format!("{}-{}", event.id, event.started_at_unix_ms);
                                            move |_| selected_event_key.set(Some(event_key.clone()))
                                        },
                                        span {
                                            class: format!("traffic-method {}", agent_api_method_class(&event.method)),
                                            "{event.method}"
                                        }
                                        span { class: "traffic-path", "{event.path}" }
                                        span {
                                            class: format!("traffic-status-code {}", agent_api_status_class(event.status_code)),
                                            "{event.status_code}"
                                        }
                                        span { class: "traffic-dur", "{event.duration_ms}ms" }
                                    }
                                }
                            }

                            div { class: "traffic-detail-pane",
                                if let Some(event) = selected_event {
                                    div { class: "traffic-detail-header",
                                        span {
                                            class: format!("traffic-detail-method {}", agent_api_method_class(&event.method)),
                                            "{event.method}"
                                        }
                                        span { class: "traffic-detail-path", "{event.path}" }
                                        span {
                                            class: format!("traffic-status-code {}", agent_api_status_class(event.status_code)),
                                            "{event.status_code}"
                                        }
                                    }

                                    div { class: "traffic-detail-meta",
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "id" }
                                            span { class: "traffic-detail-value", "{event.id}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "started_at_unix_ms" }
                                            span { class: "traffic-detail-value", "{event.started_at_unix_ms}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "duration" }
                                            span { class: "traffic-detail-value", "{event.duration_ms}ms" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "query" }
                                            span { class: "traffic-detail-value",
                                                {event.query.clone().unwrap_or_else(|| "-".to_string())}
                                            }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "matched_path" }
                                            span { class: "traffic-detail-value",
                                                {event.matched_path.clone().unwrap_or_else(|| "-".to_string())}
                                            }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "request_version" }
                                            span { class: "traffic-detail-value", "{event.request_version}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "response_version" }
                                            span { class: "traffic-detail-value", "{event.response_version}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "auth_enabled" }
                                            span { class: "traffic-detail-value", "{event.auth_enabled}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "authorization_header_present" }
                                            span { class: "traffic-detail-value", "{event.authorization_header_present}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "request_bytes" }
                                            span { class: "traffic-detail-value", "{event.request_body_bytes}" }
                                        }
                                        div { class: "traffic-detail-field",
                                            span { class: "traffic-detail-label", "response_bytes" }
                                            span { class: "traffic-detail-value", "{event.response_body_bytes}" }
                                        }
                                    }

                                    div { class: "traffic-detail-section",
                                        div { class: "traffic-section-title",
                                            span { "Request Headers" }
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
                                            span { "Request Body" }
                                            span { class: "traffic-meta-chip",
                                                "{agent_api_body_encoding_label(&event.request_body_encoding)}"
                                                if event.request_body_truncated { " (truncated)" }
                                            }
                                        }
                                        pre { class: "traffic-detail-body",
                                            {agent_api_body_for_pre(
                                                &event.request_body,
                                                &event.request_headers,
                                                &event.request_body_encoding,
                                            )}
                                        }
                                    }

                                    div { class: "traffic-detail-section",
                                        div { class: "traffic-section-title",
                                            span { "Response Headers" }
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

                                    div { class: "traffic-detail-section",
                                        div { class: "traffic-section-title",
                                            span { "Response Body" }
                                            span { class: "traffic-meta-chip",
                                                "{agent_api_body_encoding_label(&event.response_body_encoding)}"
                                                if event.response_body_truncated { " (truncated)" }
                                            }
                                        }
                                        pre { class: "traffic-detail-body",
                                            {agent_api_body_for_pre(
                                                &event.response_body,
                                                &event.response_headers,
                                                &event.response_body_encoding,
                                            )}
                                        }
                                    }
                                } else {
                                    div { class: "traffic-detail-empty", "Select a request to inspect." }
                                }
                            }
                        }
                    }
                }
            }
        }

        fn agent_api_method_class(method: &str) -> &'static str {
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

        fn agent_api_status_class(status_code: u16) -> &'static str {
            if status_code < 300 {
                "tstatus-2xx"
            } else if status_code < 400 {
                "tstatus-3xx"
            } else if status_code < 500 {
                "tstatus-4xx"
            } else {
                "tstatus-5xx"
            }
        }

        fn agent_api_body_encoding_label(
            encoding: &crate::loopbox::AgentApiAuditBodyEncoding,
        ) -> &'static str {
            match encoding {
                crate::loopbox::AgentApiAuditBodyEncoding::Utf8 => "utf8",
                crate::loopbox::AgentApiAuditBodyEncoding::Hex => "hex",
            }
        }

        fn format_agent_api_body_display(
            body: &str,
            headers: &[crate::loopbox::AgentApiAuditHeader],
            encoding: &crate::loopbox::AgentApiAuditBodyEncoding,
        ) -> String {
            if body.is_empty() {
                return String::new();
            }
            if !matches!(encoding, crate::loopbox::AgentApiAuditBodyEncoding::Utf8) {
                return body.to_string();
            }

            let content_type = agent_api_header_content_type(headers);
            if !agent_api_is_json_body(content_type.as_deref(), body) {
                return body.to_string();
            }

            match serde_json::from_str::<serde_json::Value>(body) {
                Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string()),
                Err(_) => body.to_string(),
            }
        }

        fn agent_api_body_for_pre(
            body: &str,
            headers: &[crate::loopbox::AgentApiAuditHeader],
            encoding: &crate::loopbox::AgentApiAuditBodyEncoding,
        ) -> String {
            let rendered = format_agent_api_body_display(body, headers, encoding);
            if rendered.is_empty() {
                "(empty)".to_string()
            } else {
                rendered
            }
        }

        fn agent_api_header_content_type(
            headers: &[crate::loopbox::AgentApiAuditHeader],
        ) -> Option<String> {
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

        fn agent_api_is_json_body(content_type: Option<&str>, body: &str) -> bool {
            if content_type.is_some_and(|ct| ct == "application/json" || ct.ends_with("+json")) {
                return true;
            }
            let trimmed = body.trim();
            (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        }
    };
}
