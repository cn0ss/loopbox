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
                .find(|event| format!("{}-{}", event.id, event.started_at_unix_ms) == selected_key)
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
