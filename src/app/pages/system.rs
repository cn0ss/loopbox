use crate::app::models::{Notice, Page, SetupStatus};
use crate::app::utils::{
    apply_setup_result, set_notice_error, set_notice_info, set_notice_success,
};
use crate::loopbox::{self, LoopboxConfig};
use dioxus::html::input_data::keyboard_types::{Key, Modifiers};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SystemTab {
    Setup,
    HostsFile,
    Platform,
}

// ── Hosts syntax highlighting ──

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn highlight_hosts_line(line: &str) -> String {
    let trimmed = line.trim_start();

    if trimmed.is_empty() {
        return String::new();
    }

    // Comment lines
    if trimmed.starts_with('#') {
        return format!("<span class=\"syn-comment\">{}</span>", html_escape(line));
    }

    // IP + hostnames: first token is IP (key color), rest are hostnames (val color)
    let leading_ws = &line[..line.len() - trimmed.len()];
    let mut parts = trimmed.splitn(2, [' ', '\t']);
    let ip = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");

    let mut out = String::new();
    out.push_str(&html_escape(leading_ws));
    out.push_str(&format!(
        "<span class=\"syn-key\">{}</span>",
        html_escape(ip)
    ));
    if !rest.is_empty() {
        out.push_str(&format!(
            "<span class=\"syn-val\"> {}</span>",
            html_escape(rest.trim_start())
        ));
    }
    out
}

fn highlight_hosts_content(content: &str) -> String {
    content
        .split('\n')
        .map(highlight_hosts_line)
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Search helpers ──

fn hosts_search_match_count(content: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    let q = query.to_lowercase();
    let c = content.to_lowercase();
    let mut count = 0;
    let mut pos = 0;
    while let Some(found) = c[pos..].find(&q) {
        count += 1;
        pos += found + 1;
    }
    count
}

fn hosts_search_match_offset(content: &str, query: &str, index: usize) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let q = query.to_lowercase();
    let c = content.to_lowercase();
    let mut i = 0;
    let mut pos = 0;
    while let Some(found) = c[pos..].find(&q) {
        if i == index {
            let start = pos + found;
            return Some((start, start + query.len()));
        }
        i += 1;
        pos += found + 1;
    }
    None
}

// Helper: run trusted JS snippet in webview via Dioxus document::eval API.
// document::eval is the Dioxus desktop webview interop, not JavaScript's eval().
// Only static trusted JS strings are passed here.
fn run_webview_js(js: &str) {
    let js = js.to_string();
    spawn(async move {
        let _ = document::eval(&js).await;
    });
}

fn focus_hosts_search_at(start: usize, end: usize) {
    let js = format!(
        "var t=document.getElementById('hosts-editor-input');if(t){{t.focus();t.setSelectionRange({start},{end});var lh=parseFloat(getComputedStyle(t).lineHeight)||16;var ln=t.value.substring(0,{start}).split('\\n').length-1;t.scrollTop=ln*lh-t.clientHeight/3;}}"
    );
    run_webview_js(&js);
}

fn platform_support_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS primary"
    } else if cfg!(target_os = "windows") {
        "Windows experimental"
    } else {
        "unsupported platform"
    }
}

fn platform_support_summary() -> &'static str {
    if cfg!(target_os = "macos") {
        "Loopback aliases, hosts management, pf redirects, Sparkle updates, and persistent integrated terminal sessions are available on macOS."
    } else if cfg!(target_os = "windows") {
        "Hosts management, loopback aliases, netsh portproxy redirects, and standard process runtime are available. Integrated terminal sessions and auto-update are not yet available on Windows."
    } else {
        "Loopbox currently ships platform implementations for macOS and experimental Windows builds."
    }
}

fn platform_capability_rows() -> Vec<(&'static str, &'static str)> {
    if cfg!(target_os = "macos") {
        vec![
            ("Loopback aliases", "lo0 aliases"),
            ("Domain-only HTTP", "pf redirect"),
            ("Hosts file", "/etc/hosts"),
            ("Integrated terminal", "persistent PTY"),
            ("Updates", "Sparkle"),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            ("Loopback aliases", "netsh interface ipv4"),
            ("Domain-only HTTP", "netsh portproxy"),
            ("Hosts file", r"C:\Windows\System32\drivers\etc\hosts"),
            ("Integrated terminal", "not available"),
            ("Updates", "manual download"),
        ]
    } else {
        vec![
            ("Loopback aliases", "not available"),
            ("Domain-only HTTP", "not available"),
            ("Hosts file", "platform dependent"),
            ("Integrated terminal", "not available"),
            ("Updates", "not available"),
        ]
    }
}

// ════════════════════════════════════════════
// Hosts Editor Component
// ════════════════════════════════════════════

#[component]
fn HostsEditor(
    hosts_path: String,
    hosts_is_loaded: bool,
    hosts_dirty: bool,
    hosts_outside_danger: bool,
    mut hosts_content: Signal<String>,
    mut hosts_original: Signal<String>,
    mut hosts_loaded: Signal<bool>,
    mut notice: Signal<Option<Notice>>,
) -> Element {
    let mut search_open = use_signal(|| false);
    let mut search_query = use_signal(String::new);
    let mut search_index = use_signal(|| 0_usize);
    let reloaded_hosts_notice = format!("Reloaded {}", hosts_path);
    let loaded_hosts_notice = format!("Loaded {}", hosts_path);

    let content = hosts_content();
    let original = hosts_original();
    let dirty = hosts_is_loaded && content != original;
    let outside_danger = dirty && loopbox::has_changes_outside_managed_block(&original, &content);

    let line_count = content.split('\n').count().max(1);
    let char_count = content.len();
    let byte_count = content.len();
    let s_open = search_open();
    let s_query = search_query();
    let s_idx = search_index();

    // Set up scroll sync + tab handling
    use_effect(move || {
        if hosts_loaded() {
            run_webview_js(
                r#"setTimeout(function() {
                    var ta = document.getElementById('hosts-editor-input');
                    var hl = document.getElementById('hosts-editor-highlight');
                    var gt = document.getElementById('hosts-editor-gutter');
                    if (!ta || !hl || !gt) return;
                    if (ta._lbScroll) ta.removeEventListener('scroll', ta._lbScroll);
                    ta._lbScroll = function() {
                        hl.scrollTop = ta.scrollTop;
                        hl.scrollLeft = ta.scrollLeft;
                        gt.scrollTop = ta.scrollTop;
                    };
                    ta.addEventListener('scroll', ta._lbScroll);
                    if (ta._lbTab) ta.removeEventListener('keydown', ta._lbTab);
                    ta._lbTab = function(e) {
                        if (e.key === 'Tab') {
                            e.preventDefault();
                            e.stopPropagation();
                            var s = ta.selectionStart, end = ta.selectionEnd;
                            ta.value = ta.value.substring(0, s) + '    ' + ta.value.substring(end);
                            ta.selectionStart = ta.selectionEnd = s + 4;
                            ta.dispatchEvent(new Event('input', { bubbles: true }));
                        }
                    };
                    ta.addEventListener('keydown', ta._lbTab);
                }, 30);"#,
            );
        }
    });

    rsx! {
        section { class: "panel",
            div { class: "panel-header",
                    div {
                        h2 { "Hosts Editor" }
                        div { class: "hosts-path",
                            span { class: "hosts-path-icon", "\u{25B8}" }
                            span { "{hosts_path}" }
                        }
                    }
                div { class: "panel-badges",
                    if dirty {
                        span { class: "dirty-badge", "\u{25CF} unsaved" }
                    }
                    span { class: "panel-badge", "system file" }
                }
            }

            if hosts_is_loaded {
                if outside_danger {
                    div { class: "danger-banner",
                        span { class: "danger-banner-icon", "\u{26A0}" }
                        p {
                            "You are editing lines outside the managed loopbox block. "
                            "These are system entries \u{2014} changes here can break DNS resolution."
                        }
                    }
                }

                div { class: if outside_danger { "code-editor code-editor-danger" } else { "code-editor" },

                    // ── Toolbar ──
                    div { class: "code-editor-toolbar",
                        div { class: "code-editor-toolbar-left",
                            span { class: "code-editor-file-badge", "{hosts_path}" }
                            if dirty {
                                span { class: "code-editor-dirty-dot" }
                                span { class: "code-editor-dirty-label", "modified" }
                            }
                        }
                        div { class: "code-editor-toolbar-right",
                            button {
                                class: "code-editor-toolbar-btn",
                                title: "Find (Ctrl/Cmd+F)",
                                onclick: move |_| {
                                    let opening = !s_open;
                                    search_open.set(opening);
                                    if !opening {
                                        search_query.set(String::new());
                                        search_index.set(0);
                                    }
                                },
                                if s_open { "\u{2715} Find" } else { "\u{2315} Find" }
                            }
                            button {
                                class: "btn btn-primary btn-sm",
                                disabled: !dirty,
                                onclick: move |_| {
                                    match loopbox::save_hosts_file(&hosts_content()) {
                                        Ok(msg) => {
                                            hosts_original.set(hosts_content());
                                            set_notice_success(notice, msg);
                                        }
                                        Err(err) => set_notice_error(notice, err),
                                    }
                                },
                                "Save"
                            }
                            button {
                                class: "btn btn-outline btn-sm",
                                disabled: !dirty,
                                onclick: move |_| {
                                    hosts_content.set(hosts_original());
                                },
                                "Revert"
                            }
                            button {
                                class: "btn btn-outline btn-sm",
                                onclick: move |_| {
                                    match loopbox::read_hosts_file() {
                                        Ok(new_content) => {
                                            hosts_content.set(new_content.clone());
                                            hosts_original.set(new_content);
                                            set_notice_info(notice, reloaded_hosts_notice.clone());
                                        }
                                        Err(err) => set_notice_error(notice, err),
                                    }
                                },
                                "Reload"
                            }
                        }
                    }

                    // ── Search Panel ──
                    if s_open {
                        {{
                            let match_count = hosts_search_match_count(&content, &s_query);
                            let display_idx = if match_count > 0 { s_idx + 1 } else { 0 };
                            rsx! {
                                div { class: "code-editor-search",
                                    input {
                                        class: "code-editor-search-input",
                                        r#type: "text",
                                        placeholder: "Find\u{2026}",
                                        value: "{s_query}",
                                        oninput: move |evt| {
                                            search_query.set(evt.value());
                                            search_index.set(0);
                                        },
                                        onkeydown: move |evt| {
                                            if evt.key() == Key::Enter {
                                                let total = hosts_search_match_count(&hosts_content(), &search_query());
                                                if total > 0 {
                                                    let next = (search_index() + 1) % total;
                                                    search_index.set(next);
                                                    if let Some((start, end)) = hosts_search_match_offset(&hosts_content(), &search_query(), next) {
                                                        focus_hosts_search_at(start, end);
                                                    }
                                                }
                                            }
                                            if evt.key() == Key::Escape {
                                                search_open.set(false);
                                                search_query.set(String::new());
                                                search_index.set(0);
                                            }
                                        },
                                    }
                                    span {
                                        class: if match_count > 0 { "code-editor-search-count code-editor-search-count-active" } else { "code-editor-search-count" },
                                        if s_query.is_empty() {
                                            ""
                                        } else {
                                            "{display_idx} of {match_count}"
                                        }
                                    }
                                    button {
                                        class: "code-editor-search-btn",
                                        disabled: match_count == 0,
                                        title: "Previous match",
                                        onclick: move |_| {
                                            let total = hosts_search_match_count(&hosts_content(), &search_query());
                                            if total > 0 {
                                                let prev = if search_index() == 0 { total - 1 } else { search_index() - 1 };
                                                search_index.set(prev);
                                                if let Some((start, end)) = hosts_search_match_offset(&hosts_content(), &search_query(), prev) {
                                                    focus_hosts_search_at(start, end);
                                                }
                                            }
                                        },
                                        "\u{2191}"
                                    }
                                    button {
                                        class: "code-editor-search-btn",
                                        disabled: match_count == 0,
                                        title: "Next match",
                                        onclick: move |_| {
                                            let total = hosts_search_match_count(&hosts_content(), &search_query());
                                            if total > 0 {
                                                let next = (search_index() + 1) % total;
                                                search_index.set(next);
                                                if let Some((start, end)) = hosts_search_match_offset(&hosts_content(), &search_query(), next) {
                                                    focus_hosts_search_at(start, end);
                                                }
                                            }
                                        },
                                        "\u{2193}"
                                    }
                                    button {
                                        class: "code-editor-search-btn",
                                        title: "Close search (Esc)",
                                        onclick: move |_| {
                                            search_open.set(false);
                                            search_query.set(String::new());
                                            search_index.set(0);
                                        },
                                        "\u{2715}"
                                    }
                                }
                            }
                        }}
                    }

                    // ── Editor Body ──
                    div { class: "code-editor-body",
                        div {
                            class: "code-editor-gutter",
                            id: "hosts-editor-gutter",
                            for i in 1..=line_count {
                                div {
                                    class: "code-editor-line-num",
                                    key: "{i}",
                                    "{i}"
                                }
                            }
                        }
                        div { class: "code-editor-content",
                            pre {
                                class: "code-editor-highlight",
                                id: "hosts-editor-highlight",
                                dangerous_inner_html: "{highlight_hosts_content(&content)}",
                            }
                            textarea {
                                class: "code-editor-input",
                                id: "hosts-editor-input",
                                value: "{content}",
                                oninput: move |evt| hosts_content.set(evt.value()),
                                onkeydown: move |evt: KeyboardEvent| {
                                    if (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL))
                                        && evt.key() == Key::Character("s".to_string())
                                    {
                                        evt.prevent_default();
                                        let is_dirty = hosts_content() != hosts_original();
                                        if is_dirty {
                                            match loopbox::save_hosts_file(&hosts_content()) {
                                                Ok(msg) => {
                                                    hosts_original.set(hosts_content());
                                                    set_notice_success(notice, msg);
                                                }
                                                Err(err) => set_notice_error(notice, err),
                                            }
                                        }
                                    }
                                    if (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL))
                                        && evt.key() == Key::Character("f".to_string())
                                    {
                                        evt.prevent_default();
                                        search_open.set(true);
                                        run_webview_js("setTimeout(function(){var e=document.querySelector('#hosts-editor-input').closest('.code-editor').querySelector('.code-editor-search-input');if(e)e.focus();},30);");
                                    }
                                    if evt.key() == Key::Escape && search_open() {
                                        search_open.set(false);
                                        search_query.set(String::new());
                                        search_index.set(0);
                                    }
                                },
                                spellcheck: "false",
                            }
                        }
                    }

                    // ── Status Bar ──
                    div { class: "code-editor-status",
                        span { class: "code-editor-status-item", "{line_count} lines" }
                        span { class: "code-editor-status-item", "{char_count} chars" }
                        span { class: "code-editor-status-item", "{byte_count} bytes" }
                        span { class: "code-editor-status-item", "UTF-8" }
                        if dirty {
                            span { class: "code-editor-status-item code-editor-status-modified", "\u{25CF} modified" }
                        }
                    }
                }
            } else {
                div { class: "empty-state",
                    p { class: "empty-state-text", "Load the hosts file to begin editing." }
                }
                div { class: "form-actions",
                    button {
                        class: "btn btn-outline",
                        onclick: move |_| {
                            match loopbox::read_hosts_file() {
                                Ok(new_content) => {
                                    hosts_content.set(new_content.clone());
                                    hosts_original.set(new_content);
                                    hosts_loaded.set(true);
                                    set_notice_info(notice, loaded_hosts_notice.clone());
                                }
                                Err(err) => set_notice_error(notice, err),
                            }
                        },
                        "Load File"
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_system_page(
    page: Page,
    setup_alias_count: usize,
    setup_lines_count: usize,
    setup_hosts_count: usize,
    can_apply_setup: bool,
    hosts_is_loaded: bool,
    hosts_dirty: bool,
    hosts_outside_danger: bool,
    _hosts_line_count: usize,
    _hosts_byte_count: usize,
    hosts_preview: String,
    apply_script_preview: String,
    _hosts_snapshot: String,
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    setup_status: Signal<Option<SetupStatus>>,
    mut show_setup_script: Signal<bool>,
    hosts_content: Signal<String>,
    hosts_original: Signal<String>,
    hosts_loaded: Signal<bool>,
) -> Element {
    let hosts_path = crate::platform::hosts::hosts_file_path().to_string();
    let system_setup_summary = format!(
        "Apply loopback network setup, refresh the managed {} block, and sync domain-only URL redirect rules.",
        hosts_path
    );
    let managed_hosts_label = format!("Managed {} Block", hosts_path);
    let mut active_tab = use_signal(|| SystemTab::Setup);
    let tab = active_tab();

    rsx! {
        if page == Page::System {
            div { class: "page system-page",
                div { class: "page-header",
                    div { class: "page-header-left",
                        div { class: "page-header-stack",
                            span { class: "page-eyebrow", "Host" }
                            div {
                                style: "display:flex; align-items:baseline; gap:14px; flex-wrap:wrap;",
                                h1 { class: "page-title", "system" }
                                span { class: "status-badge status-badge--warn", "admin required" }
                            }
                            div { class: "page-meta",
                                span { class: "page-meta-item",
                                    "platform" strong { "\u{00a0}{platform_support_label()}" }
                                }
                                span { class: "page-meta-sep", "·" }
                                span { class: "page-meta-item",
                                    "{setup_alias_count}" "\u{00a0}loopback ips"
                                }
                                span { class: "page-meta-sep", "·" }
                                span { class: "page-meta-item",
                                    "{setup_hosts_count}" "\u{00a0}hostnames across" "\u{00a0}{setup_lines_count}" "\u{00a0}sandboxes"
                                }
                            }
                        }
                    }
                }

                div { class: "tab-bar system-tabs",
                    button {
                        class: if tab == SystemTab::Setup { "active" } else { "" },
                        onclick: move |_| active_tab.set(SystemTab::Setup),
                        "setup"
                    }
                    button {
                        class: if tab == SystemTab::HostsFile { "active" } else { "" },
                        onclick: move |_| active_tab.set(SystemTab::HostsFile),
                        "hosts file"
                    }
                    button {
                        class: if tab == SystemTab::Platform { "active" } else { "" },
                        onclick: move |_| active_tab.set(SystemTab::Platform),
                        "platform"
                    }
                }

                if tab == SystemTab::Setup {
                    section { class: "panel system-action-panel",
                        div { class: "panel-header",
                            div {
                                h2 { "System Setup" }
                                p { class: "panel-subtitle", "{system_setup_summary}" }
                            }
                            if let Some(last_setup) = setup_status() {
                                span {
                                    class: format!("status-badge status-badge--{}",
                                        match last_setup.kind.class_name() {
                                            c if c.contains("error") => "error",
                                            c if c.contains("warn") => "warn",
                                            _ => "ok",
                                        }
                                    ),
                                    "{last_setup.action} · {last_setup.timestamp}"
                                }
                            }
                        }

                        div { class: "form-actions system-actions",
                            button {
                                class: "btn btn-primary",
                                disabled: !can_apply_setup,
                                onclick: move |_| {
                                    apply_setup_result(
                                        loopbox::apply_system_setup(&config()),
                                        "Applied",
                                        "Apply Failed",
                                        notice,
                                        setup_status,
                                    );
                                },
                                "Apply system setup"
                            }
                            button {
                                class: "btn btn-outline",
                                onclick: move |_| {
                                    let current = show_setup_script();
                                    show_setup_script.set(!current);
                                },
                                if show_setup_script() {
                                    "Hide generated script"
                                } else {
                                    "Preview generated script"
                                }
                            }
                            div { class: "system-actions-spacer" }
                            button {
                                class: "btn btn-danger",
                                onclick: move |_| {
                                    apply_setup_result(
                                        loopbox::revert_system_setup(&config()),
                                        "Reverted",
                                        "Revert Failed",
                                        notice,
                                        setup_status,
                                    );
                                },
                                "Revert setup"
                            }
                        }

                        if !can_apply_setup {
                            p { class: "panel-help system-help-warn",
                                "Add at least one sandbox before running system setup."
                            }
                        } else {
                            p { class: "panel-help",
                                "You'll be prompted for elevated privileges when applying."
                            }
                        }

                        if let Some(last_setup) = setup_status() {
                            p { class: "panel-help",
                                style: "font-family: var(--font-ui); color: var(--text-secondary);",
                                "{last_setup.message}"
                            }
                        }

                        if show_setup_script() {
                            h3 { class: "panel-section-title", "Generated Script" }
                            textarea {
                                class: "code-box",
                                readonly: true,
                                value: "{apply_script_preview}",
                            }
                        }

                        h3 { class: "panel-section-title", "{managed_hosts_label}" }
                        textarea {
                            class: "code-box code-box-sm",
                            readonly: true,
                            value: "{hosts_preview}",
                        }
                    }
                }

                if tab == SystemTab::HostsFile {
                    HostsEditor {
                        hosts_path,
                        hosts_is_loaded,
                        hosts_dirty,
                        hosts_outside_danger,
                        hosts_content,
                        hosts_original,
                        hosts_loaded,
                        notice,
                    }
                }

                if tab == SystemTab::Platform {
                    section { class: "panel platform-support-panel",
                        div { class: "panel-header",
                            div {
                                h2 { "Platform Capabilities" }
                                p { class: "panel-subtitle", "{platform_support_summary()}" }
                            }
                        }

                        div { class: "platform-capability-grid",
                            for (label, value) in platform_capability_rows() {
                                div { class: "platform-capability-row", key: "{label}",
                                    span { class: "platform-capability-label", "{label}" }
                                    span { class: "platform-capability-value", "{value}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
