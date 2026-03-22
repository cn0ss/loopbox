use crate::app::models::{Notice, Page};
use crate::loopbox::{AgentApiServerInfo, LoopboxConfig};
use dioxus::prelude::*;

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
        use crate::platform::tray::MacOsMenuBarTray;
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
