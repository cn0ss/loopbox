use crate::app::models::{Notice, Page};
use crate::loopbox::{self, LoopboxConfig};
use dioxus::html::input_data::keyboard_types::{Key, Modifiers};
use dioxus::prelude::*;
use serde_json::Value;

pub(in crate::app) fn render_agents_page(
    page: Page,
    config: Signal<LoopboxConfig>,
    _selected_project: Signal<Option<String>>,
    mut notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    let mut composer = use_signal(String::new);
    let mut composer_diagnosis_session_id = use_signal(|| None::<String>);
    let mut show_events = use_signal(|| false);
    let mut auto_connect_attempted = use_signal(|| false);
    let mut last_page_was_agents = use_signal(|| false);

    let is_agents_page = page == Page::Agents;
    use_effect(move || {
        if is_agents_page && !last_page_was_agents() {
            auto_connect_attempted.set(false);
        }
        last_page_was_agents.set(is_agents_page);
    });

    if !is_agents_page {
        return rsx! {};
    }

    let _ = runtime_tick();
    let snapshot = loopbox::codex_agents_snapshot(&config());
    let should_auto_connect =
        snapshot.enabled && !snapshot.running && !snapshot.starting && !auto_connect_attempted();
    use_effect(move || {
        if should_auto_connect {
            auto_connect_attempted.set(true);
            if let Err(err) = loopbox::codex_agents_start(&config()) {
                notice.set(Some(Notice::error(err)));
            }
        }
    });
    let prefilled_prompt = snapshot.prefilled_prompt.clone();
    let prefilled_diagnosis_session_id = snapshot.prefilled_diagnosis_session_id.clone();
    use_effect(move || {
        if let Some(prompt) = prefilled_prompt.clone() {
            composer.set(prompt);
            composer_diagnosis_session_id.set(prefilled_diagnosis_session_id.clone());
        }
    });

    let composer_value = composer();
    let diagnosis_session_id_snapshot = composer_diagnosis_session_id();
    let show_event_log = show_events();
    let can_send = snapshot.enabled && !composer_value.trim().is_empty();
    let can_interrupt = snapshot.active_turn_id.is_some();
    let engine_label = if snapshot.running {
        "running"
    } else if snapshot.starting {
        "starting"
    } else {
        "stopped"
    };
    let turn_label = snapshot
        .turn_status
        .as_deref()
        .unwrap_or(if snapshot.active_thread_id.is_some() {
            "idle"
        } else {
            "new"
        })
        .to_string();
    let model_summary = selected_model_summary(&snapshot.models);
    let model_count = snapshot.models.len();
    let thread_count = snapshot.threads.len();
    let event_count = snapshot.event_log.len();
    let diagnostics_count = snapshot.stderr_tail.len();
    let tool_count = snapshot.loopbox_mcp_tools.len();
    let missing_tool_count = snapshot.loopbox_mcp_missing_tools.len();
    let missing_tool_label = snapshot.loopbox_mcp_missing_tools.join(", ");
    let tool_health_label = if tool_count == 0 {
        "Not loaded".to_string()
    } else if missing_tool_count > 0 {
        format!("{missing_tool_count} missing")
    } else {
        format!("{tool_count} ready")
    };
    let tool_health_class = if tool_count == 0 {
        "agents-tool-health"
    } else if missing_tool_count > 0 {
        "agents-tool-health is-warn"
    } else {
        "agents-tool-health is-ok"
    };
    let primary_action_icon = if snapshot.running || snapshot.starting {
        "↻"
    } else {
        "▶"
    };
    let primary_action_reloads_tools = snapshot.running || snapshot.starting;
    let primary_action_label = if snapshot.running || snapshot.starting {
        "Reconnect"
    } else {
        "Start"
    };
    let transcript_items = snapshot
        .transcript
        .iter()
        .filter_map(agent_item_view)
        .collect::<Vec<_>>();
    let transcript_item_ids = transcript_items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let thread_rows = snapshot
        .threads
        .iter()
        .map(|thread| AgentThreadRow {
            id: thread.id.clone(),
            title: thread.title.clone(),
            meta: thread_meta(thread),
            is_active: snapshot.active_thread_id.as_deref() == Some(thread.id.as_str()),
        })
        .collect::<Vec<_>>();

    rsx! {
        div { class: "page agents-page",
            div { class: "agents-commandbar",
                div { class: "agents-commandbar-main",
                    div { class: "agents-brand-mark",
                        CodexGlyph {}
                    }
                    h1 { class: "page-title agents-title", "Agents" }
                }
                div { class: "agents-status-strip",
                    span { class: "agents-context-token",
                        span { class: "agents-inline-icon", "⌘" }
                        strong { "Global Loopbox" }
                    }
                    span {
                        class: if snapshot.running { "agents-status-token is-ok" } else if snapshot.starting { "agents-status-token is-warn" } else { "agents-status-token" },
                        "{engine_label}"
                    }
                    span { class: "agents-status-token", "{turn_label}" }
                }
                details { class: "agents-actions-menu",
                    summary { class: "btn btn-sm btn-outline agents-menu-trigger",
                        span { class: "agents-button-icon", "⋯" }
                        "Actions"
                    }
                    div { class: "agents-actions-popover",
                        button {
                            class: "agents-menu-item",
                            onclick: move |_| {
                                let result = if primary_action_reloads_tools {
                                    loopbox::codex_agents_reload_tools(&config())
                                } else {
                                    loopbox::codex_agents_start(&config())
                                };
                                match result {
                                    Ok(()) if primary_action_reloads_tools => {
                                        notice.set(Some(Notice::info("Refreshing Codex connection.")))
                                    }
                                    Ok(()) => {
                                        notice.set(Some(Notice::success("Codex app-server started.")))
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            span { class: "agents-button-icon", "{primary_action_icon}" }
                            "{primary_action_label}"
                        }
                        button {
                            class: "agents-menu-item",
                            onclick: move |_| match loopbox::codex_agents_reload_tools(&config()) {
                                Ok(()) => notice.set(Some(Notice::info("Reloading Loopbox MCP tools."))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            },
                            span { class: "agents-button-icon", "⟳" }
                            "Reload tools"
                        }
                        button {
                            class: "agents-menu-item",
                            disabled: !can_interrupt,
                            onclick: move |_| match loopbox::codex_agents_interrupt_turn() {
                                Ok(()) => notice.set(Some(Notice::info("Interrupt requested."))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            },
                            span { class: "agents-button-icon", "Ⅱ" }
                            "Interrupt"
                        }
                        button {
                            class: "agents-menu-item agents-menu-item-danger",
                            disabled: !snapshot.running,
                            onclick: move |_| match loopbox::codex_agents_stop() {
                                Ok(()) => notice.set(Some(Notice::info("Codex app-server stopped."))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            },
                            span { class: "agents-button-icon", "■" }
                            "Stop"
                        }
                    }
                }
            }

            div { class: "agents-workbench",
                aside { class: "agents-rail",
                    section { class: "agents-rail-section agents-chats-section",
                        div { class: "agents-section-head",
                            h2 {
                                span { class: "agents-section-icon", "◌" }
                                "Chats"
                                span { class: "agents-count-badge", "{thread_count}" }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: move |_| match loopbox::codex_agents_new_chat(&config()) {
                                    Ok(()) => notice.set(Some(Notice::info("New Codex chat ready."))),
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                },
                                span { class: "agents-button-icon", "+" }
                                "New"
                            }
                        }
                        if thread_rows.is_empty() {
                            p { class: "agents-rail-empty", "No saved chats yet" }
                        } else {
                            div { class: "agents-chat-list",
                                for thread in thread_rows.iter() {
                                    button {
                                        key: "{thread.id}",
                                        class: if thread.is_active { "agents-chat-row is-active" } else { "agents-chat-row" },
                                        onclick: {
                                            let thread_id = thread.id.clone();
                                            move |_| match loopbox::codex_agents_resume_thread(&config(), &thread_id) {
                                                Ok(()) => notice.set(Some(Notice::info("Loaded Codex chat."))),
                                                Err(err) => notice.set(Some(Notice::error(err))),
                                            }
                                        },
                                        strong { "{thread.title}" }
                                        small { "{thread.meta}" }
                                    }
                                }
                            }
                        }
                    }

                    section { class: "agents-rail-section",
                        h2 {
                            span { class: "agents-section-icon", "◈" }
                            "Session"
                        }
                        dl { class: "agents-kv",
                            div {
                                dt { "Binary" }
                                dd { code { "{snapshot.codex_binary}" } }
                            }
                            div {
                                dt { "Scope" }
                                dd { "Global" }
                            }
                            div {
                                dt { "Model" }
                                dd { "{model_summary}" }
                            }
                            div {
                                dt { "Account" }
                                dd { class: "agents-kv-truncate", "{snapshot.auth.label}" }
                            }
                        }
                        if let Some(thread_id) = snapshot.active_thread_id.as_ref() {
                            div { class: "agents-thread-id",
                                span { "Thread" }
                                code { "{thread_id}" }
                            }
                        }
                    }

                    section { class: "agents-rail-section",
                        h2 {
                            span { class: "agents-section-icon", "◎" }
                            "Models"
                        }
                        if snapshot.models.is_empty() {
                            p { class: "agents-rail-empty", "Unavailable" }
                        } else {
                            details { class: "agents-rail-details",
                                summary {
                                    span { "{model_count} available" }
                                    span { class: "agents-chevron", ">" }
                                }
                                div { class: "agents-model-list",
                                    for model in snapshot.models.iter() {
                                        div {
                                            key: "{model.id}",
                                            class: if model.is_default { "agents-model-row is-default" } else { "agents-model-row" },
                                            span { "{model.display_name}" }
                                            if let Some(effort) = model.default_effort.as_ref() {
                                                small { "{effort}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    section { class: "agents-rail-section",
                        h2 {
                            span { class: "agents-section-icon", "∿" }
                            "Diagnostics"
                        }
                        div { class: "{tool_health_class}",
                            span {
                                span { class: "agents-section-icon", "ƒ" }
                                "Loopbox tools"
                            }
                            strong { "{tool_health_label}" }
                        }
                        if !missing_tool_label.is_empty() {
                            p { class: "agents-tool-warning",
                                "Missing: {missing_tool_label}"
                            }
                        }
                        button {
                            class: "agents-rail-toggle",
                            onclick: move |_| show_events.set(!show_event_log),
                            span {
                                span { class: "agents-section-icon", "≡" }
                                "Events"
                            }
                            span { class: "agents-count-badge", "{event_count}" }
                        }
                        if show_event_log {
                            div { class: "agents-event-log",
                                for (index, event) in snapshot.event_log.iter().rev().take(40).enumerate() {
                                    div { key: "{index}", "{event}" }
                                }
                            }
                        }
                        if diagnostics_count > 0 {
                            details { class: "agents-rail-details agents-diagnostics-rail",
                                summary {
                                    span { "Process output" }
                                    span { class: "agents-count-badge", "{diagnostics_count}" }
                                }
                                pre {
                                    for line in snapshot.stderr_tail.iter() {
                                        "{line}\n"
                                    }
                                }
                            }
                        }
                    }
                }

                section { class: "agents-conversation",
                    if !snapshot.enabled {
                        div { class: "agents-alert is-warn",
                            "Codex Agents are disabled in Loopbox config."
                        }
                    }
                    if snapshot.auth.requires_auth && !snapshot.auth.signed_in {
                        div { class: "agents-alert is-warn",
                            "Codex needs authentication. Sign in with the Codex CLI, then restart."
                        }
                    }
                    div { class: "agents-scroll",
                        if transcript_items.is_empty() {
                            div { class: "agents-empty",
                                div { class: "agents-empty-mark",
                                    span { ">" }
                                }
                                h2 { "Ready" }
                                div { class: "agents-quick-prompts",
                                    for prompt in quick_prompts() {
                                        button {
                                            key: "{prompt.label}",
                                            class: "agents-prompt-chip",
                                            onclick: {
                                                let prompt = prompt.prompt.clone();
                                                move |_| composer.set(prompt.clone())
                                            },
                                            span { class: "agents-prompt-icon", "{prompt.icon}" }
                                            "{prompt.label}"
                                        }
                                    }
                                }
                            }
                        } else {
                            div { class: "agents-transcript",
                                for item in transcript_items.iter() {
                                    div {
                                        key: "{item.id}",
                                        class: "{item.class_name}",
                                        div { class: "agents-avatar", "{item.icon}" }
                                        div { class: "agents-message-shell",
                                            div { class: "agents-message-meta",
                                                strong { "{item.title}" }
                                                if !item.status.is_empty() {
                                                    span { class: "agents-message-status", "{item.status}" }
                                                }
                                            }
                                            if item.is_tool {
                                                div { class: "agents-tool-summary", "{item.summary}" }
                                                if !item.detail.is_empty() {
                                                    details { class: "agents-tool-details",
                                                        summary {
                                                            span { "Payload" }
                                                            span { "⌄" }
                                                        }
                                                        pre { "{item.detail}" }
                                                    }
                                                }
                                                for request in snapshot.pending_requests.iter().filter(|request| request.item_id.as_deref() == Some(item.id.as_str())) {
                                                    {render_approval_card(request, notice, "agents-approval-card is-inline")}
                                                }
                                            } else if item.text.trim().is_empty() && item.raw_json.trim().is_empty() {
                                                div { class: "agents-message-text agents-message-muted", "No output" }
                                            } else if item.body_mono {
                                                pre { class: "agents-message-text agents-message-text-mono", "{item.text}" }
                                            } else {
                                                div { class: "agents-message-text", "{item.text}" }
                                            }
                                        }
                                    }
                                }
                                for request in snapshot.pending_requests.iter().filter(|request| request.item_id.as_ref().map(|item_id| !transcript_item_ids.contains(item_id)).unwrap_or(true)) {
                                    div {
                                        key: "approval-fallback-{request.request_id}",
                                        class: "agents-message agents-message-approval",
                                        div { class: "agents-avatar", "!" }
                                        div { class: "agents-message-shell",
                                            div { class: "agents-message-meta",
                                                strong { "Approval" }
                                                span { class: "agents-message-status", "pending" }
                                            }
                                            {render_approval_card(request, notice, "agents-approval-card is-inline")}
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !snapshot.errors.is_empty() {
                        div { class: "agents-error-list",
                            for (index, error) in snapshot.errors.iter().rev().take(3).enumerate() {
                                div { key: "{index}", "{error}" }
                            }
                        }
                    }

                    div { class: "agents-composer",
                        textarea {
                            class: "agents-composer-input",
                            value: "{composer_value}",
                            placeholder: "Ask about sandboxes, logs, traffic, or runtime...",
                            oninput: move |evt| composer.set(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter && (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL)) {
                                    let text = composer().trim().to_string();
                                    if text.is_empty() {
                                        return;
                                    }
                                    let result = if let Some(session_id) = composer_diagnosis_session_id() {
                                        loopbox::codex_agents_send_diagnosis_message(&config(), session_id, text)
                                    } else {
                                        loopbox::codex_agents_send_message(&config(), None, text)
                                    };
                                    match result {
                                        Ok(()) => {
                                            composer.set(String::new());
                                            composer_diagnosis_session_id.set(None);
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                }
                            },
                        }
                        div { class: "agents-composer-actions",
                            if diagnosis_session_id_snapshot.is_some() {
                                span { class: "agents-composer-context", "Diagnosis" }
                            } else {
                                span { "Cmd/Ctrl Enter" }
                            }
                            button {
                                class: "btn btn-primary",
                                disabled: !can_send,
                                onclick: move |_| {
                                    let text = composer().trim().to_string();
                                    if text.is_empty() {
                                        return;
                                    }
                                    let result = if let Some(session_id) = composer_diagnosis_session_id() {
                                        loopbox::codex_agents_send_diagnosis_message(&config(), session_id, text)
                                    } else {
                                        loopbox::codex_agents_send_message(&config(), None, text)
                                    };
                                    match result {
                                        Ok(()) => {
                                            composer.set(String::new());
                                            composer_diagnosis_session_id.set(None);
                                        }
                                        Err(err) => notice.set(Some(Notice::error(err))),
                                    }
                                },
                                span { class: "agents-button-icon", "↵" }
                                "Send"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CodexGlyph() -> Element {
    rsx! {
        svg {
            class: "agents-codex-glyph",
            fill: "currentColor",
            fill_rule: "evenodd",
            view_box: "0 0 24 24",
            xmlns: "http://www.w3.org/2000/svg",
            title { "Codex" }
            path {
                clip_rule: "evenodd",
                d: "M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z",
            }
        }
    }
}

fn render_approval_card(
    request: &loopbox::CodexAgentPendingRequest,
    mut notice: Signal<Option<Notice>>,
    class_name: &'static str,
) -> Element {
    let accept_request_id = request.request_id.clone();
    let decline_request_id = request.request_id.clone();

    rsx! {
        div { class: "{class_name}", key: "{request.request_id}",
            div { class: "agents-approval-copy",
                strong { "{request.title}" }
                p { "{request.body}" }
            }
            div { class: "agents-approval-actions",
                button {
                    class: "btn btn-sm btn-primary",
                    onclick: move |_| match loopbox::codex_agents_accept_request(&accept_request_id) {
                        Ok(()) => notice.set(Some(Notice::success("Approved Codex request."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Accept"
                }
                button {
                    class: "btn btn-sm btn-outline",
                    onclick: move |_| match loopbox::codex_agents_decline_request(&decline_request_id) {
                        Ok(()) => notice.set(Some(Notice::info("Declined Codex request."))),
                        Err(err) => notice.set(Some(Notice::error(err))),
                    },
                    "Decline"
                }
            }
        }
    }
}

#[derive(Clone)]
struct AgentThreadRow {
    id: String,
    title: String,
    meta: String,
    is_active: bool,
}

#[derive(Clone)]
struct QuickPrompt {
    label: String,
    prompt: String,
    icon: String,
}

fn quick_prompts() -> Vec<QuickPrompt> {
    vec![
        QuickPrompt {
            label: "Summarize".to_string(),
            prompt: "Summarize all Loopbox sandboxes.".to_string(),
            icon: "◇".to_string(),
        },
        QuickPrompt {
            label: "Doctor".to_string(),
            prompt: "Run Loopbox doctor and explain the highest priority issue.".to_string(),
            icon: "✚".to_string(),
        },
        QuickPrompt {
            label: "Runtime".to_string(),
            prompt: "Which services are running right now?".to_string(),
            icon: "◉".to_string(),
        },
        QuickPrompt {
            label: "Create sandbox".to_string(),
            prompt: "Create a new Loopbox sandbox. Ask me for the sandbox name, absolute project directory, services, commands, working directories if needed, ports, protocols, and health paths. Then validate with loopbox_validate_project_config and create it with loopbox_create_project after approval.".to_string(),
            icon: "+".to_string(),
        },
    ]
}

#[derive(Clone)]
struct AgentItemView {
    id: String,
    title: String,
    status: String,
    text: String,
    raw_json: String,
    summary: String,
    detail: String,
    class_name: String,
    icon: String,
    is_tool: bool,
    body_mono: bool,
}

fn agent_item_view(item: &loopbox::CodexTranscriptItem) -> Option<AgentItemView> {
    if item.kind == "reasoning" && item.text.trim().is_empty() {
        return None;
    }

    let is_tool = matches!(
        item.kind.as_str(),
        "mcpToolCall" | "dynamicToolCall" | "commandExecution" | "fileChange" | "webSearch"
    );
    let is_working = item.status == "inProgress" || item.status == "sending";
    let class_name = format!(
        "agents-message agents-message-{}{}",
        item.kind,
        if is_working { " is-working" } else { "" }
    );
    let body_mono = !matches!(item.kind.as_str(), "agentMessage" | "userMessage" | "plan");
    let summary = if is_tool {
        tool_summary(item)
    } else {
        String::new()
    };
    let detail = if is_tool {
        tool_detail(item)
    } else if item.text.trim().is_empty() {
        item.raw_json.clone()
    } else {
        String::new()
    };

    Some(AgentItemView {
        id: item.id.clone(),
        title: item.title.clone(),
        status: item.status.clone(),
        text: if item.text.trim().is_empty() && !item.raw_json.trim().is_empty() {
            item.raw_json.clone()
        } else {
            item.text.clone()
        },
        raw_json: item.raw_json.clone(),
        summary,
        detail,
        class_name,
        icon: item_icon(&item.kind).to_string(),
        is_tool,
        body_mono,
    })
}

fn item_icon(kind: &str) -> &'static str {
    match kind {
        "userMessage" => "↵",
        "agentMessage" => "✦",
        "mcpToolCall" | "dynamicToolCall" => "ƒ",
        "commandExecution" => "$",
        "fileChange" => "Δ",
        "webSearch" => "⌕",
        "plan" => "◇",
        "reasoning" => "∴",
        _ => "·",
    }
}

fn tool_summary(item: &loopbox::CodexTranscriptItem) -> String {
    let Ok(raw) = serde_json::from_str::<Value>(&item.raw_json) else {
        return status_summary(item);
    };
    let tool_name = raw
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or(item.title.as_str());

    if item.status == "inProgress" {
        return format!("Calling {tool_name}...");
    }
    if let Some(error) = raw.get("error").filter(|value| !value.is_null()) {
        return format!("Tool failed: {}", compact_value(error));
    }

    let structured = raw
        .pointer("/result/structuredContent")
        .or_else(|| raw.pointer("/result/structured_content"))
        .or_else(|| raw.get("structuredContent"));

    match (tool_name, structured) {
        ("loopbox_overview", Some(value)) => overview_summary(value),
        ("loopbox_runtime", Some(value)) => runtime_summary(value),
        ("loopbox_logs", Some(value)) => collection_summary(value, "lines", "log line"),
        ("loopbox_requests", Some(value)) => collection_summary(value, "events", "request"),
        ("loopbox_resources", Some(value)) => {
            collection_summary(value, "samples", "resource sample")
        }
        (_, Some(_)) => "Completed with structured output.".to_string(),
        _ => status_summary(item),
    }
}

fn tool_detail(item: &loopbox::CodexTranscriptItem) -> String {
    let Ok(raw) = serde_json::from_str::<Value>(&item.raw_json) else {
        return item.text.clone();
    };
    let mut sections = Vec::new();
    if let Some(arguments) = raw.get("arguments") {
        sections.push(format!("Arguments\n{}", pretty_value(arguments)));
    }
    if let Some(structured) = raw
        .pointer("/result/structuredContent")
        .or_else(|| raw.pointer("/result/structured_content"))
    {
        sections.push(format!("Structured output\n{}", pretty_value(structured)));
    } else if let Some(result) = raw.get("result") {
        sections.push(format!("Result\n{}", pretty_value(result)));
    }
    if sections.is_empty() {
        pretty_value(&raw)
    } else {
        sections.join("\n\n")
    }
}

fn overview_summary(value: &Value) -> String {
    let project_count = value
        .get("projects")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let running = value
        .get("projects")
        .and_then(Value::as_array)
        .map(|projects| {
            projects
                .iter()
                .filter_map(|project| project.get("runningCount").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let warnings = value
        .get("doctor")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);

    format!(
        "Scanned {project_count} sandbox{}, found {running} running service{}, {warnings} doctor warning{}.",
        plural(project_count),
        plural(running as usize),
        plural(warnings)
    )
}

fn runtime_summary(value: &Value) -> String {
    let mut running = Vec::new();
    if let Some(projects) = value.get("projects").and_then(Value::as_object) {
        for (project, services) in projects {
            let Some(services) = services.as_array() else {
                continue;
            };
            for service in services {
                let state = service.pointer("/status/state").and_then(Value::as_str);
                if state == Some("running") || state == Some("starting") {
                    if let Some(name) = service.get("service").and_then(Value::as_str) {
                        running.push(format!("{project}/{name}"));
                    }
                }
            }
        }
    }

    if running.is_empty() {
        "Runtime checked: no services are running.".to_string()
    } else {
        format!("Runtime checked: {} running.", running.join(", "))
    }
}

fn collection_summary(value: &Value, key: &str, label: &str) -> String {
    let count = value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    format!("Returned {count} {label}{}.", plural(count))
}

fn status_summary(item: &loopbox::CodexTranscriptItem) -> String {
    match item.status.as_str() {
        "completed" => "Completed.".to_string(),
        "failed" => "Failed.".to_string(),
        "declined" => "Declined.".to_string(),
        "inProgress" => "Running...".to_string(),
        status if !status.is_empty() => status.to_string(),
        _ => "No output.".to_string(),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn compact_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn pretty_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn selected_model_summary(models: &[loopbox::CodexAgentModel]) -> String {
    if models.is_empty() {
        return "Not loaded".to_string();
    }

    let model = models
        .iter()
        .find(|model| model.is_default)
        .unwrap_or(&models[0]);
    match model.default_effort.as_deref() {
        Some(effort) => format!("{} ({effort})", model.display_name),
        None => model.display_name.clone(),
    }
}

fn thread_meta(thread: &loopbox::CodexAgentThreadSummary) -> String {
    let preview = thread.preview.trim();
    if !preview.is_empty() {
        return truncate_chars(preview, 56);
    }
    thread
        .updated_at
        .or(thread.created_at)
        .map(|timestamp| format!("timestamp {timestamp}"))
        .unwrap_or_else(|| short_thread_id(&thread.id))
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id.chars().take(12).collect::<String>()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push('…');
    }
    output
}
