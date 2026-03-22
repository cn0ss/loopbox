use crate::app::models::{Notice, Page};
use crate::loopbox::{self, AgentApiServerInfo, LoopboxConfig, ServiceRuntimeState};
use dioxus::prelude::*;
#[cfg(target_os = "macos")]
use std::{cell::RefCell, time::Duration};

#[cfg(target_os = "macos")]
use dioxus::desktop::{
    trayicon::{
        menu::{Menu, MenuItem, PredefinedMenuItem},
        Icon, TrayIcon, TrayIconBuilder,
    },
    use_tray_menu_event_handler, use_window, DesktopContext,
};

#[cfg(target_os = "macos")]
const TRAY_OPEN_ID: &str = "loopbox-tray-open";
#[cfg(target_os = "macos")]
const TRAY_SHOW_SANDBOXES_ID: &str = "loopbox-tray-show-sandboxes";
#[cfg(target_os = "macos")]
const TRAY_SHOW_RUNTIME_ID: &str = "loopbox-tray-show-runtime";
#[cfg(target_os = "macos")]
const TRAY_SHOW_AGENT_API_ID: &str = "loopbox-tray-show-agent-api";
#[cfg(target_os = "macos")]
const TRAY_START_ALL_ID: &str = "loopbox-tray-start-all";
#[cfg(target_os = "macos")]
const TRAY_STOP_ALL_ID: &str = "loopbox-tray-stop-all";
#[cfg(target_os = "macos")]
const TRAY_REFRESH_INTERVAL_SECS: u64 = 5;

#[component]
pub(super) fn MenuBarTrayController(
    config: Signal<LoopboxConfig>,
    agent_api_info: Option<AgentApiServerInfo>,
    current_page: Signal<Page>,
    selected_project: Signal<Option<String>>,
    notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    #[cfg(target_os = "macos")]
    {
        return rsx! {
            MacOsMenuBarTray {
                config,
                agent_api_info,
                current_page,
                selected_project,
                notice,
                runtime_tick,
            }
        };
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            config,
            agent_api_info,
            current_page,
            selected_project,
            notice,
            runtime_tick,
        );
        rsx! {}
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct TrayHandles {
    _tray: TrayIcon,
    status_primary: MenuItem,
    status_secondary: MenuItem,
    start_all: MenuItem,
    stop_all: MenuItem,
    agent_api: MenuItem,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TrayRuntimeSummary {
    sandboxes: usize,
    services: usize,
    running: usize,
    starting: usize,
    unhealthy: usize,
    crashed: usize,
    stopped: usize,
    agent_api_running: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayPresentation {
    primary: String,
    secondary: String,
    title: String,
    tooltip: String,
    start_enabled: bool,
    stop_enabled: bool,
    agent_api_enabled: bool,
}

#[cfg(target_os = "macos")]
#[component]
fn MacOsMenuBarTray(
    config: Signal<LoopboxConfig>,
    agent_api_info: Option<AgentApiServerInfo>,
    current_page: Signal<Page>,
    selected_project: Signal<Option<String>>,
    notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    let window = use_window();
    let tray = use_hook(initialize_tray_handles);
    let mut tray_refresh = use_signal(|| 0_u64);
    let last_applied = use_hook(|| RefCell::new(None::<TrayPresentation>));

    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_secs(TRAY_REFRESH_INTERVAL_SECS)).await;
            tray_refresh.with_mut(|tick| *tick = tick.wrapping_add(1));
        }
    });

    let summary_resource = use_resource(move || {
        let cfg = config();
        let info = agent_api_info.clone();
        let _refresh = tray_refresh();
        async move {
            tokio::task::spawn_blocking(move || summarize_runtime(&cfg, info.as_ref()))
                .await
                .unwrap_or_default()
        }
    });

    if let Some(summary) = summary_resource() {
        let presentation = tray_presentation(&summary);
        let mut last_applied = last_applied.borrow_mut();
        if last_applied.as_ref() != Some(&presentation) {
            apply_summary_to_tray(&tray, &presentation);
            *last_applied = Some(presentation);
        }
    }

    let mut current_page = current_page;
    let mut selected_project = selected_project;
    let mut notice = notice;
    let mut runtime_tick = runtime_tick;
    let mut tray_refresh_for_menu = tray_refresh;

    use_tray_menu_event_handler(move |event| match event.id().0.as_str() {
        TRAY_OPEN_ID | TRAY_SHOW_SANDBOXES_ID => {
            selected_project.set(None);
            current_page.set(Page::Sandboxes);
            focus_main_window(&window);
        }
        TRAY_SHOW_RUNTIME_ID => {
            selected_project.set(None);
            current_page.set(Page::Runtime);
            focus_main_window(&window);
        }
        TRAY_SHOW_AGENT_API_ID => {
            selected_project.set(None);
            current_page.set(Page::AgentApiAudit);
            focus_main_window(&window);
        }
        TRAY_START_ALL_ID => {
            let message = match start_all_sandboxes(&config()) {
                Ok(message) => Notice::success(message),
                Err(err) => Notice::error(err),
            };
            notice.set(Some(message));
            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
            tray_refresh_for_menu.with_mut(|tick| *tick = tick.wrapping_add(1));
        }
        TRAY_STOP_ALL_ID => {
            let message = match stop_all_sandboxes(&config()) {
                Ok(message) => Notice::info(message),
                Err(err) => Notice::error(err),
            };
            notice.set(Some(message));
            runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
            tray_refresh_for_menu.with_mut(|tick| *tick = tick.wrapping_add(1));
        }
        _ => {}
    });

    rsx! {}
}

#[cfg(target_os = "macos")]
fn initialize_tray_handles() -> TrayHandles {
    let menu = Menu::new();
    let status_primary = MenuItem::new("Loopbox", false, None);
    let status_secondary = MenuItem::new("Loading runtime status...", false, None);
    let open = MenuItem::with_id(TRAY_OPEN_ID, "Open Loopbox", true, None);
    let show_sandboxes = MenuItem::with_id(TRAY_SHOW_SANDBOXES_ID, "Show Sandboxes", true, None);
    let show_runtime = MenuItem::with_id(TRAY_SHOW_RUNTIME_ID, "Show Runtime", true, None);
    let agent_api = MenuItem::with_id(TRAY_SHOW_AGENT_API_ID, "Show Agent API Audit", true, None);
    let start_all = MenuItem::with_id(TRAY_START_ALL_ID, "Start All Sandboxes", true, None);
    let stop_all = MenuItem::with_id(TRAY_STOP_ALL_ID, "Stop All Sandboxes", true, None);
    let separator_one = PredefinedMenuItem::separator();
    let separator_two = PredefinedMenuItem::separator();
    let separator_three = PredefinedMenuItem::separator();
    let quit = PredefinedMenuItem::quit(None);

    menu.append_items(&[
        &status_primary,
        &status_secondary,
        &separator_one,
        &open,
        &show_sandboxes,
        &show_runtime,
        &agent_api,
        &separator_two,
        &start_all,
        &stop_all,
        &separator_three,
        &quit,
    ])
    .expect("failed to assemble Loopbox tray menu");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(loopbox_tray_icon())
        .with_icon_as_template(true)
        .build()
        .expect("failed to create Loopbox tray icon");

    TrayHandles {
        _tray: tray,
        status_primary,
        status_secondary,
        start_all,
        stop_all,
        agent_api,
    }
}

#[cfg(target_os = "macos")]
fn apply_summary_to_tray(tray: &TrayHandles, presentation: &TrayPresentation) {
    tray.status_primary.set_text(&presentation.primary);
    tray.status_secondary.set_text(&presentation.secondary);
    tray.start_all.set_enabled(presentation.start_enabled);
    tray.stop_all.set_enabled(presentation.stop_enabled);
    tray.agent_api.set_enabled(presentation.agent_api_enabled);
    if presentation.title.is_empty() {
        tray._tray.set_title(None::<&str>);
    } else {
        tray._tray.set_title(Some(presentation.title.as_str()));
    }
    let _ = tray._tray.set_tooltip(Some(&presentation.tooltip));
}

#[cfg(target_os = "macos")]
fn summarize_runtime(
    config: &LoopboxConfig,
    agent_api_info: Option<&AgentApiServerInfo>,
) -> TrayRuntimeSummary {
    let mut summary = TrayRuntimeSummary {
        sandboxes: config.projects.len(),
        agent_api_running: agent_api_info.is_some_and(|info| info.running),
        ..TrayRuntimeSummary::default()
    };

    for (project_name, project) in &config.projects {
        for service in &project.services {
            summary.services += 1;
            match loopbox::service_runtime_status(config, project_name, &service.name) {
                Ok(status) => match status.state {
                    ServiceRuntimeState::Running => summary.running += 1,
                    ServiceRuntimeState::Starting => summary.starting += 1,
                    ServiceRuntimeState::Unhealthy => summary.unhealthy += 1,
                    ServiceRuntimeState::Crashed => summary.crashed += 1,
                    ServiceRuntimeState::Stopped => summary.stopped += 1,
                },
                Err(_) => summary.crashed += 1,
            }
        }
    }

    summary
}

#[cfg(target_os = "macos")]
fn tray_presentation(summary: &TrayRuntimeSummary) -> TrayPresentation {
    TrayPresentation {
        primary: primary_status_text(summary),
        secondary: secondary_status_text(summary),
        title: tray_title(summary),
        tooltip: tray_tooltip(summary),
        start_enabled: summary.sandboxes > 0,
        stop_enabled: summary.running + summary.starting + summary.unhealthy > 0,
        agent_api_enabled: true,
    }
}

#[cfg(target_os = "macos")]
fn primary_status_text(summary: &TrayRuntimeSummary) -> String {
    if summary.sandboxes == 0 {
        return "No sandboxes configured".to_string();
    }
    if summary.unhealthy > 0 || summary.crashed > 0 {
        return format!(
            "{} running · {} unhealthy · {} crashed",
            summary.running, summary.unhealthy, summary.crashed
        );
    }
    if summary.starting > 0 {
        return format!(
            "{} running · {} starting",
            summary.running, summary.starting
        );
    }
    format!("{} running · {} stopped", summary.running, summary.stopped)
}

#[cfg(target_os = "macos")]
fn secondary_status_text(summary: &TrayRuntimeSummary) -> String {
    if summary.sandboxes == 0 {
        return "Add a sandbox in Loopbox to control it from the menu bar.".to_string();
    }
    format!(
        "{} services across {} sandboxes · Agent API {}",
        summary.services,
        summary.sandboxes,
        if summary.agent_api_running {
            "on"
        } else {
            "off"
        }
    )
}

#[cfg(target_os = "macos")]
fn tray_title(summary: &TrayRuntimeSummary) -> String {
    if summary.unhealthy > 0 || summary.crashed > 0 {
        return "!".to_string();
    }
    if summary.running > 0 {
        return summary.running.to_string();
    }
    String::new()
}

#[cfg(target_os = "macos")]
fn tray_tooltip(summary: &TrayRuntimeSummary) -> String {
    if summary.sandboxes == 0 {
        return "Loopbox: no sandboxes configured".to_string();
    }
    format!(
        "Loopbox: {} running, {} starting, {} unhealthy, {} crashed, Agent API {}",
        summary.running,
        summary.starting,
        summary.unhealthy,
        summary.crashed,
        if summary.agent_api_running {
            "on"
        } else {
            "off"
        }
    )
}

#[cfg(target_os = "macos")]
fn start_all_sandboxes(config: &LoopboxConfig) -> Result<String, String> {
    if config.projects.is_empty() {
        return Err("No sandboxes configured.".to_string());
    }

    let mut started = 0_usize;
    let mut errors = Vec::new();
    for project_name in config.projects.keys() {
        match loopbox::start_project_all(config, project_name) {
            Ok(_) => started += 1,
            Err(err) => errors.push(format!("{project_name}: {err}")),
        }
    }

    if errors.is_empty() {
        Ok(format!("Started {started} sandbox(es) from the menu bar."))
    } else {
        Err(format!(
            "Failed to start some sandboxes: {}",
            errors.join(" | ")
        ))
    }
}

#[cfg(target_os = "macos")]
fn stop_all_sandboxes(config: &LoopboxConfig) -> Result<String, String> {
    if config.projects.is_empty() {
        return Err("No sandboxes configured.".to_string());
    }

    let mut stopped = 0_usize;
    let mut errors = Vec::new();
    for project_name in config.projects.keys() {
        match loopbox::stop_project_all(config, project_name) {
            Ok(_) => stopped += 1,
            Err(err) => errors.push(format!("{project_name}: {err}")),
        }
    }

    if errors.is_empty() {
        Ok(format!("Stopped {stopped} sandbox(es) from the menu bar."))
    } else {
        Err(format!(
            "Failed to stop some sandboxes: {}",
            errors.join(" | ")
        ))
    }
}

#[cfg(target_os = "macos")]
fn focus_main_window(window: &DesktopContext) {
    window.set_visible(true);
    window.set_focus();
}

#[cfg(target_os = "macos")]
fn loopbox_tray_icon() -> Icon {
    const SIZE: usize = 18;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let is_box_left = (3..=5).contains(&x) && (3..=14).contains(&y);
            let is_box_top = (3..=12).contains(&x) && (3..=5).contains(&y);
            let is_box_bottom = (3..=12).contains(&x) && (12..=14).contains(&y);
            let is_box_right = (10..=12).contains(&x) && (7..=10).contains(&y);
            let is_dot = (13..=14).contains(&x) && (3..=4).contains(&y);
            if is_box_left || is_box_top || is_box_bottom || is_box_right || is_dot {
                let idx = (y * SIZE + x) * 4;
                rgba[idx] = 0xff;
                rgba[idx + 1] = 0xff;
                rgba[idx + 2] = 0xff;
                rgba[idx + 3] = 0xff;
            }
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).expect("valid Loopbox tray icon pixels")
}
