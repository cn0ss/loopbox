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
    let mut search_input = use_signal(String::new);
    let mut status_filter = use_signal(|| AgentApiStatusFilter::All);
    let raw_events = crate::loopbox::agent_api_audit_events(500).unwrap_or_default();
    let total_count = raw_events.len();
    let ok_count = raw_events.iter().filter(|e| e.status_code < 400).count();
    let warn_count = raw_events
        .iter()
        .filter(|e| (400..500).contains(&e.status_code))
        .count();
    let err_count = raw_events.iter().filter(|e| e.status_code >= 500).count();

    let search = search_input();
    let search_trimmed = search.trim().to_ascii_lowercase();
    let filter = status_filter();
    let events: Vec<_> = raw_events
        .iter()
        .filter(|event| {
            let pass_status = match filter {
                AgentApiStatusFilter::All => true,
                AgentApiStatusFilter::Ok => event.status_code < 400,
                AgentApiStatusFilter::Client => (400..500).contains(&event.status_code),
                AgentApiStatusFilter::Server => event.status_code >= 500,
            };
            if !pass_status {
                return false;
            }
            if search_trimmed.is_empty() {
                return true;
            }
            event.method.to_ascii_lowercase().contains(&search_trimmed)
                || event.path.to_ascii_lowercase().contains(&search_trimmed)
                || event.status_code.to_string().contains(&search_trimmed)
        })
        .cloned()
        .collect();
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
                    div { class: "page-header-stack",
                        span { class: "page-eyebrow", "Inspect" }
                        h1 { class: "page-title", "agent api audit" }
                        div { class: "page-meta",
                            span { class: "page-meta-item",
                                "total" strong { "\u{00a0}{total_count}" }
                            }
                            span { class: "page-meta-sep", "·" }
                            span { class: "status-badge status-badge--ok status-badge--count",
                                "{ok_count} ok"
                            }
                            span { class: "status-badge status-badge--warn status-badge--count",
                                "{warn_count} 4xx"
                            }
                            span { class: "status-badge status-badge--error status-badge--count",
                                "{err_count} 5xx"
                            }
                        }
                    }
                }
                div { class: "page-actions",
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: move |_| {
                            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                        },
                        "Refresh"
                    }
                    button {
                        class: "btn btn-sm btn-danger",
                        disabled: total_count == 0,
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

            if total_count == 0 {
                div { class: "empty-state",
                    span { class: "empty-state-icon", "⌘" }
                    h2 { class: "empty-state-title", "no agent api requests" }
                    p { class: "empty-state-desc",
                        "Once an agent (Codex, Claude, Cursor, …) hits the local Agent API, every request and response will appear here with full headers and bodies."
                    }
                }
            } else {
                div { class: "split-layout",
                    aside { class: "split-pane split-pane-list",
                        div { class: "split-pane-list-header",
                            div {
                                style: "display:flex; flex-direction:column; gap:10px; width:100%;",
                                div {
                                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px; width:100%;",
                                    span { class: "split-pane-list-header-title",
                                        "{events.len()} of {total_count}"
                                    }
                                    div {
                                        style: "display:flex; gap:4px;",
                                        for option in [AgentApiStatusFilter::All, AgentApiStatusFilter::Ok, AgentApiStatusFilter::Client, AgentApiStatusFilter::Server] {
                                            button {
                                                key: "{option.label()}",
                                                class: if option == filter { "agent-api-chip is-active" } else { "agent-api-chip" },
                                                onclick: move |_| status_filter.set(option),
                                                "{option.label()}"
                                            }
                                        }
                                    }
                                }
                                input {
                                    class: "agent-api-search",
                                    r#type: "text",
                                    value: "{search}",
                                    placeholder: "Filter method, path, status…",
                                    oninput: move |evt| search_input.set(evt.value()),
                                }
                            }
                        }
                        div { class: "split-pane-list-body",
                            if events.is_empty() {
                                div { class: "split-pane-detail--empty",
                                    "No requests match the current filter."
                                }
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
                    }

                    section { class: "split-pane split-pane-detail",
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
                            div { class: "split-pane-detail--empty",
                                "Select a request to inspect its headers and body."
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgentApiStatusFilter {
    All,
    Ok,
    Client,
    Server,
}

impl AgentApiStatusFilter {
    fn label(self) -> &'static str {
        match self {
            AgentApiStatusFilter::All => "all",
            AgentApiStatusFilter::Ok => "ok",
            AgentApiStatusFilter::Client => "4xx",
            AgentApiStatusFilter::Server => "5xx",
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
