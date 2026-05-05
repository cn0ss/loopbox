use super::*;

#[component]
pub(super) fn SettingsFeatureSections(
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
    mut resource_metrics_enabled_input: Signal<bool>,
    mut resource_metrics_sample_interval_input: Signal<String>,
    mut resource_metrics_retention_days_input: Signal<String>,
    mut resource_metrics_max_storage_mb_input: Signal<String>,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    pending_auto_apply: Signal<Option<String>>,
) -> Element {
    let mut support_email_input = use_signal(String::new);
    let mut support_subject_input = use_signal(String::new);
    let mut support_text_input = use_signal(String::new);
    let support_form_valid = !support_email_input().trim().is_empty()
        && !support_subject_input().trim().is_empty()
        && !support_text_input().trim().is_empty();
    let resource_metrics_stats = loopbox::resource_metrics_disk_stats();
    let resource_metrics_storage_label = format!(
        "{} file(s), {}",
        resource_metrics_stats.total_files,
        crate::app::runtime_view::format_memory_bytes(Some(resource_metrics_stats.total_bytes))
    );
    let resource_metrics_dropped_label = format!(
        "{} dropped sample(s)",
        resource_metrics_stats.dropped_samples
    );

    rsx! {
        // ── Support ──────────────────────────────────────
        div { class: "settings-section",
            div { class: "settings-section-head",
                span { class: "settings-section-icon", "⚑" }
                div {
                    p { class: "settings-section-title", "Support" }
                    p { class: "settings-section-desc", "Send a support request from this Loopbox install." }
                }
            }
            div { class: "settings-section-body",
                div { class: "settings-toggle-row settings-toggle-row-last",
                    div { class: "settings-toggle-info",
                        span { class: "settings-toggle-label", "Availability" }
                        span { class: "settings-toggle-desc", "available" }
                    }
                    div {}
                }
                label { class: "field",
                    span { class: "field-label", "Email" }
                    input {
                        class: "field-input",
                        value: "{support_email_input}",
                        placeholder: "you@company.com",
                        oninput: move |evt| support_email_input.set(evt.value()),
                    }
                }
                label { class: "field",
                    span { class: "field-label", "Subject" }
                    input {
                        class: "field-input",
                        value: "{support_subject_input}",
                        placeholder: "What do you need help with?",
                        oninput: move |evt| support_subject_input.set(evt.value()),
                    }
                }
                label { class: "field field-wide",
                    span { class: "field-label", "Text" }
                    textarea {
                        class: "field-input field-textarea",
                        value: "{support_text_input}",
                        placeholder: "Describe your issue, steps to reproduce, and expected behavior.",
                        oninput: move |evt| support_text_input.set(evt.value()),
                    }
                }
                div { class: "settings-save-row",
                    button {
                        class: "btn btn-primary",
                        disabled: !support_form_valid,
                        onclick: move |_| {
                            let email = support_email_input();
                            let subject = support_subject_input();
                            let text = support_text_input();
                            match loopbox::submit_support_ticket(&email, &subject, &text) {
                                Ok(_) => {
                                    support_subject_input.set(String::new());
                                    support_text_input.set(String::new());
                                    notice.set(Some(Notice::success(
                                        "Support request submitted.".to_string(),
                                    )));
                                }
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Submit Support Request"
                    }
                }
                p { class: "settings-hint",
                    "Commercial use still requires a paid license through loopbox.tech/pricing; the app does not enforce it locally."
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
                                onclick: move |_| proxy_capture_mode_input.set(ProxyCaptureMode::Metadata),
                                "Metadata"
                            }
                            button {
                                class: if proxy_capture_mode_input() == ProxyCaptureMode::Headers {
                                    "seg-btn seg-btn-on"
                                } else {
                                    "seg-btn"
                                },
                                onclick: move |_| proxy_capture_mode_input.set(ProxyCaptureMode::Headers),
                                "Headers"
                            }
                            button {
                                class: if proxy_capture_mode_input() == ProxyCaptureMode::BodyPreview {
                                    "seg-btn seg-btn-on"
                                } else {
                                    "seg-btn"
                                },
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
                                        "Traffic capture settings updated. Capture mode was adjusted for this build."
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

        // ── Resource Metrics ──────────────────────────────
        div { class: "settings-section",
            div { class: "settings-section-head",
                span { class: "settings-section-icon", "▥" }
                div {
                    p { class: "settings-section-title", "Resource Metrics" }
                    p { class: "settings-section-desc", "Sampling and storage for service CPU and memory trends." }
                }
            }
            div { class: "settings-section-body",
                div { class: "settings-toggles",
                    div { class: "settings-toggle-row settings-toggle-row-last",
                        div { class: "settings-toggle-info",
                            span { class: "settings-toggle-label", "Collect Metrics" }
                            span { class: "settings-toggle-desc", "Sample active services while Loopbox or the Agent API is running." }
                        }
                        button {
                            class: if resource_metrics_enabled_input() {
                                "toggle-pill toggle-pill-on"
                            } else {
                                "toggle-pill"
                            },
                            onclick: move |_| {
                                resource_metrics_enabled_input.set(!resource_metrics_enabled_input());
                            },
                            span { class: "toggle-pill-dot" }
                            if resource_metrics_enabled_input() { "Enabled" } else { "Disabled" }
                        }
                    }
                }
                div { class: "settings-sub-divider" }
                div { class: "settings-fields-3",
                    label { class: "field",
                        span { class: "field-label", "Sample Interval (sec)" }
                        input {
                            class: "field-input",
                            value: "{resource_metrics_sample_interval_input}",
                            placeholder: "5",
                            oninput: move |evt| resource_metrics_sample_interval_input.set(evt.value()),
                        }
                    }
                    label { class: "field",
                        span { class: "field-label", "Retention (days)" }
                        input {
                            class: "field-input",
                            value: "{resource_metrics_retention_days_input}",
                            placeholder: "7",
                            oninput: move |evt| resource_metrics_retention_days_input.set(evt.value()),
                        }
                    }
                    label { class: "field",
                        span { class: "field-label", "Max Storage (MB)" }
                        input {
                            class: "field-input",
                            value: "{resource_metrics_max_storage_mb_input}",
                            placeholder: "250",
                            oninput: move |evt| resource_metrics_max_storage_mb_input.set(evt.value()),
                        }
                    }
                }
                p { class: "settings-hint",
                    "Interval: 2–60 seconds. Retention: 1–90 days. Storage: 25–5,000 MB. Current usage: {resource_metrics_storage_label}; {resource_metrics_dropped_label}."
                }
                div { class: "settings-save-row",
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            let enabled = resource_metrics_enabled_input();
                            let interval_raw = resource_metrics_sample_interval_input();
                            let retention_raw = resource_metrics_retention_days_input();
                            let storage_raw = resource_metrics_max_storage_mb_input();

                            let parsed_interval = match interval_raw.trim().parse::<u64>() {
                                Ok(value) if value > 0 => Ok(value.clamp(2, 60)),
                                Ok(_) => Err("Resource sample interval must be greater than 0.".to_string()),
                                Err(_) => Err("Resource sample interval must be a number between 2 and 60.".to_string()),
                            };
                            let parsed_retention = match retention_raw.trim().parse::<u16>() {
                                Ok(value) if value > 0 => Ok(value.clamp(1, 90)),
                                Ok(_) => Err("Resource retention days must be greater than 0.".to_string()),
                                Err(_) => Err("Resource retention days must be a number between 1 and 90.".to_string()),
                            };
                            let parsed_storage = match storage_raw.trim().parse::<usize>() {
                                Ok(value) if value > 0 => Ok(value.clamp(25, 5_000)),
                                Ok(_) => Err("Resource max storage must be greater than 0.".to_string()),
                                Err(_) => Err("Resource max storage must be a number between 25 and 5000.".to_string()),
                            };

                            let update_result = match (parsed_interval, parsed_retention, parsed_storage) {
                                (Ok(interval), Ok(retention), Ok(storage)) => {
                                    config.with_mut(|cfg| {
                                        cfg.global.resource_metrics.enabled = enabled;
                                        cfg.global.resource_metrics.sample_interval_secs = interval;
                                        cfg.global.resource_metrics.retention_days = retention;
                                        cfg.global.resource_metrics.max_storage_mb = storage;
                                    });
                                    resource_metrics_sample_interval_input.set(interval.to_string());
                                    resource_metrics_retention_days_input.set(retention.to_string());
                                    resource_metrics_max_storage_mb_input.set(storage.to_string());
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
                                    "Resource metrics settings updated.",
                                    None,
                                    false,
                                ),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Save Resource Metrics"
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
