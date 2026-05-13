use crate::loopbox;
use dioxus::prelude::*;
use std::collections::VecDeque;
use std::sync::Mutex;

// Helper: run trusted JS snippet in webview via Dioxus document::eval API.
// This is the same pattern used in sandboxes.rs — document::eval is Dioxus's
// webview interop, not JavaScript's eval(). Only static trusted JS is passed.
fn run_webview_js(js: &str) {
    let js = js.to_string();
    spawn(async move {
        let _ = document::eval(&js).await;
    });
}

// ── Static config queue for popup windows ──

pub(crate) struct LogWindowConfig {
    pub project_name: String,
    pub service_name: String,
}

static WINDOW_QUEUE: Mutex<VecDeque<LogWindowConfig>> = Mutex::new(VecDeque::new());

pub(crate) fn push_config(config: LogWindowConfig) {
    WINDOW_QUEUE.lock().unwrap().push_back(config);
}

fn take_config() -> Option<LogWindowConfig> {
    WINDOW_QUEUE.lock().unwrap().pop_front()
}

// ── Standalone log popout component ──

#[allow(non_snake_case)]
pub(crate) fn LogPopoutWindow() -> Element {
    let initial = take_config().unwrap_or(LogWindowConfig {
        project_name: "unknown".into(),
        service_name: "unknown".into(),
    });

    let project_name = use_signal(|| initial.project_name);
    let mut log_filter = use_signal(|| Some(initial.service_name));
    let mut tick = use_signal(|| 0_u64);

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            tick.with_mut(|t| *t = t.wrapping_add(1));
        }
    });

    let pn = project_name();
    let filter = log_filter();

    // Get service list from config
    let cfg = loopbox::load_config().unwrap_or_default();
    let services: Vec<String> = cfg
        .projects
        .get(&pn)
        .map(|p| p.services.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default();

    // Resolve effective selection
    let selected_service = match &filter {
        Some(name) if services.contains(name) => Some(name.clone()),
        _ => services.first().cloned(),
    };

    // Fetch logs
    let _tick = tick();
    let mut logs: Vec<(String, String)> = Vec::new();
    if let Some(ref svc) = selected_service {
        if let Ok(lines) = loopbox::service_logs(&pn, svc) {
            for line in lines {
                logs.push((svc.clone(), line));
            }
        }
    }

    let log_attached = selected_service
        .as_ref()
        .and_then(|svc| loopbox::service_log_attached(&pn, svc).ok())
        .unwrap_or(false);

    rsx! {
        super::AtlasStylesheets {}

        div { class: "log-popout-root",
            div { class: "log-toolbar",
                div { class: "log-toolbar-filters",
                    for svc in &services {
                        button {
                            key: "{svc}",
                            class: if selected_service.as_ref() == Some(svc) {
                                "btn btn-sm btn-toggle-on"
                            } else {
                                "btn btn-sm btn-outline"
                            },
                            onclick: {
                                let name = svc.clone();
                                move |_| log_filter.set(Some(name.clone()))
                            },
                            "{svc}"
                        }
                    }
                }
                div { class: "log-toolbar-right",
                    if let Some(_svc) = selected_service.as_ref() {
                        span {
                            class: if log_attached { "log-status log-status-attached" } else { "log-status" },
                            if log_attached { "attached" } else { "detached" }
                        }
                    }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let pn = pn.clone();
                            let selected = selected_service.clone();
                            move |_| {
                                if let Some(ref svc) = selected {
                                    let _ = loopbox::clear_service_logs(&pn, svc);
                                    tick.with_mut(|t| *t = t.wrapping_add(1));
                                }
                            }
                        },
                        "Clear"
                    }
                }
            }

            div { class: "log-viewer-wrap",
                div { id: "log-viewer-popout", class: "log-viewer",
                    if logs.is_empty() {
                        div { class: "log-empty",
                            p { "No log output yet." }
                            p { class: "log-empty-hint", "Logs auto-refresh every second." }
                        }
                    } else {
                        for (idx, (service_name, line)) in logs.iter().enumerate() {
                            div { class: log_line_outer_class(line), key: "{service_name}-{idx}",
                                span { class: "log-svc", "{service_name}" }
                                span { class: log_line_text_class(line), "{strip_log_prefix(line)}" }
                            }
                        }
                    }
                }
                button {
                    id: "log-jump-popout",
                    class: "log-jump-btn",
                    onclick: move |_| {
                        run_webview_js(
                            "var el=document.getElementById('log-viewer-popout');\
                             if(el){el.scrollTop=el.scrollHeight;el._tailing=true;\
                             var b=document.getElementById('log-jump-popout');\
                             if(b)b.style.display='none';}"
                        );
                    },
                    span { class: "log-jump-btn-arrow", "\u{2193}" }
                    "Jump to latest"
                }
            }
            {
                let log_count = logs.len();
                run_webview_js(&format!(
                    "(function(){{\
                        var el=document.getElementById('log-viewer-popout');\
                        if(!el)return;\
                        if(!el._scrollSetup){{\
                            el._scrollSetup=true;el._tailing=true;\
                            el.addEventListener('scroll',function(){{\
                                el._tailing=el.scrollTop+el.clientHeight>=el.scrollHeight-50;\
                                var b=document.getElementById('log-jump-popout');\
                                if(b)b.style.display=el._tailing?'none':'flex';\
                            }});\
                        }}\
                        if(el._tailing)el.scrollTop=el.scrollHeight;\
                    }})();/* t={} */",
                    log_count
                ));
                rsx! {}
            }
        }
    }
}

fn log_line_outer_class(line: &str) -> &'static str {
    if line.starts_with("[stderr]") {
        "log-line log-line-err"
    } else {
        "log-line"
    }
}

fn log_line_text_class(line: &str) -> &'static str {
    if line.starts_with("[stderr]") {
        "log-text log-text-err"
    } else {
        "log-text"
    }
}

fn strip_log_prefix(line: &str) -> String {
    let unprefixed = if let Some(rest) = line.strip_prefix("[stdout] ") {
        rest
    } else if let Some(rest) = line.strip_prefix("[stderr] ") {
        rest
    } else {
        line
    };
    strip_terminal_control_sequences(unprefixed)
}

fn strip_terminal_control_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0_usize;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        if (0x40..=0x7e).contains(&c) {
                            break;
                        }
                    }
                }
                b']' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == 0x07 {
                            i += 1;
                            break;
                        }
                        if c == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => {
                    i += 1;
                }
            }
            continue;
        }

        if byte < 0x20 && byte != b'\t' {
            i += 1;
            continue;
        }
        if byte == 0x7f {
            i += 1;
            continue;
        }

        output.push(byte);
        i += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}
