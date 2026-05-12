use crate::loopbox::{self, TerminalClientMessage, TerminalKeyAction, TerminalMods};
use dioxus::html::input_data::keyboard_types::{Key, Modifiers};
use dioxus::prelude::*;
use std::collections::VecDeque;
use std::sync::Mutex;

fn run_webview_js(js: &str) {
    let js = js.to_string();
    spawn(async move {
        let _ = document::eval(&js).await;
    });
}

pub(crate) struct TerminalWindowConfig {
    pub project_name: String,
    pub service_name: String,
}

static WINDOW_QUEUE: Mutex<VecDeque<TerminalWindowConfig>> = Mutex::new(VecDeque::new());

pub(crate) fn push_config(config: TerminalWindowConfig) {
    WINDOW_QUEUE.lock().unwrap().push_back(config);
}

fn take_config() -> Option<TerminalWindowConfig> {
    WINDOW_QUEUE.lock().unwrap().pop_front()
}

#[allow(non_snake_case)]
pub(crate) fn TerminalPopoutWindow() -> Element {
    let initial = take_config().unwrap_or(TerminalWindowConfig {
        project_name: "unknown".to_string(),
        service_name: "unknown".to_string(),
    });

    let project_name = use_signal(|| initial.project_name);
    let service_name = use_signal(|| initial.service_name);
    let mut tick = use_signal(|| 0_u64);

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            tick.with_mut(|value| *value = value.wrapping_add(1));
        }
    });

    let pn = project_name();
    let svc = service_name();
    let frame_resource = use_resource(move || {
        let pn = project_name();
        let svc = service_name();
        let _ = tick();
        async move {
            tokio::task::spawn_blocking(move || loopbox::terminal_session_snapshot(&pn, &svc))
                .await
                .unwrap_or_else(|err| Err(format!("Terminal snapshot task failed: {err}")))
        }
    });

    let status_label = match frame_resource() {
        Some(Ok(frame)) => {
            let label = if frame.title.is_empty() {
                "attached".to_string()
            } else {
                frame.title.clone()
            };
            draw_terminal_frame(&frame);
            label
        }
        Some(Err(err)) => err,
        None => "connecting".to_string(),
    };

    let main_css_href = super::main_css_href();

    rsx! {
        document::Stylesheet { href: main_css_href }
        document::Stylesheet { href: "https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600&display=swap" }

        div { class: "terminal-popout-root",
            div { class: "terminal-toolbar",
                div { class: "terminal-toolbar-title",
                    span { class: "terminal-service-name", "{svc}" }
                    span { class: "terminal-project-name", "{pn}" }
                }
                span { class: "terminal-status", "{status_label}" }
            }
            div {
                id: "loopbox-terminal-shell",
                class: "terminal-canvas-shell",
                tabindex: "0",
                onmounted: move |_| run_webview_js(TERMINAL_BOOTSTRAP_JS),
                onkeydown: {
                    let pn = pn.clone();
                    let svc = svc.clone();
                    move |evt: KeyboardEvent| {
                        evt.prevent_default();
                        if is_paste_shortcut(&evt) {
                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                if let Ok(text) = clipboard.get_text() {
                                    let _ = loopbox::send_terminal_client_message(
                                        &pn,
                                        &svc,
                                        TerminalClientMessage::Paste { text },
                                    );
                                }
                            }
                            return;
                        }
                        let (code, text) = terminal_key_payload(evt.key());
                        let mods = TerminalMods {
                            ctrl: evt.modifiers().contains(Modifiers::CONTROL),
                            alt: evt.modifiers().contains(Modifiers::ALT),
                            shift: evt.modifiers().contains(Modifiers::SHIFT),
                            meta: evt.modifiers().contains(Modifiers::META),
                        };
                        let _ = loopbox::send_terminal_client_message(
                            &pn,
                            &svc,
                            TerminalClientMessage::Key {
                                code,
                                text,
                                mods,
                                action: TerminalKeyAction::Press,
                            },
                        );
                    }
                },
                canvas { id: "loopbox-terminal-canvas", class: "terminal-canvas" }
            }
        }
    }
}

fn is_paste_shortcut(evt: &KeyboardEvent) -> bool {
    (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL))
        && evt.key() == Key::Character("v".to_string())
}

fn terminal_key_payload(key: Key) -> (String, Option<String>) {
    match key {
        Key::Character(value) => ("Character".to_string(), Some(value)),
        other => (format!("{other:?}"), None),
    }
}

fn draw_terminal_frame(frame: &loopbox::TerminalFrame) {
    let Ok(frame_json) = serde_json::to_string(frame) else {
        return;
    };
    run_webview_js(&format!(
        "window.loopboxDrawTerminal && window.loopboxDrawTerminal({frame_json});"
    ));
}

const TERMINAL_BOOTSTRAP_JS: &str = r#"
(function() {
  if (window.loopboxTerminalBootstrapped) {
    var existingShell = document.getElementById('loopbox-terminal-shell');
    if (existingShell) existingShell.focus();
    return;
  }
  window.loopboxTerminalBootstrapped = true;
  window.loopboxDrawTerminal = function(frame) {
    window.loopboxLastTerminalFrame = frame;
    var canvas = document.getElementById('loopbox-terminal-canvas');
    var shell = document.getElementById('loopbox-terminal-shell');
    if (!canvas || !shell || !frame) return;
    if (document.activeElement !== shell) shell.focus();
    var rect = shell.getBoundingClientRect();
    var dpr = window.devicePixelRatio || 1;
    var width = Math.max(320, Math.floor(rect.width));
    var height = Math.max(220, Math.floor(rect.height));
    if (canvas.width !== Math.floor(width * dpr) || canvas.height !== Math.floor(height * dpr)) {
      canvas.width = Math.floor(width * dpr);
      canvas.height = Math.floor(height * dpr);
      canvas.style.width = width + 'px';
      canvas.style.height = height + 'px';
    }
    var ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = '#0b0f14';
    ctx.fillRect(0, 0, width, height);
    var fontSize = 13;
    var lineHeight = 18;
    var xPad = 12;
    var yPad = 16;
    ctx.font = '500 ' + fontSize + 'px JetBrains Mono, SFMono-Regular, Menlo, monospace';
    ctx.textBaseline = 'top';
    ctx.fillStyle = '#d5dde8';
    var lines = frame.lines || [];
    for (var i = 0; i < lines.length; i++) {
      var y = yPad + i * lineHeight;
      if (y > height - lineHeight) break;
      ctx.fillText(lines[i] || '', xPad, y);
    }
    var cellWidth = Math.max(7, Math.ceil(ctx.measureText('M').width));
    var cursorX = xPad + (frame.cursor_x || 0) * cellWidth;
    var cursorY = yPad + (frame.cursor_y || 0) * lineHeight;
    if (cursorY >= yPad && cursorY < height - lineHeight) {
      ctx.fillStyle = 'rgba(213,221,232,0.22)';
      ctx.fillRect(cursorX, cursorY, cellWidth, lineHeight);
      ctx.strokeStyle = 'rgba(213,221,232,0.75)';
      ctx.strokeRect(cursorX + 0.5, cursorY + 0.5, cellWidth - 1, lineHeight - 1);
    }
  };
  window.addEventListener('resize', function() {
    if (window.loopboxLastTerminalFrame) {
      window.loopboxDrawTerminal(window.loopboxLastTerminalFrame);
    }
  });
  var shell = document.getElementById('loopbox-terminal-shell');
  if (shell) shell.focus();
})();
"#;
