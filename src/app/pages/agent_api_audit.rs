use crate::app::models::{Notice, Page};
use dioxus::prelude::*;

mod paid;

use paid::AgentApiAuditPage;

pub(in crate::app) fn sidebar_show_agent_api_audit_tab() -> bool {
    paid::sidebar_show_agent_api_audit_tab()
}

pub(in crate::app) fn render_agent_api_audit_page(
    page: Page,
    notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    rsx! {
        if page == Page::AgentApiAudit {
            AgentApiAuditPage {
                notice,
                runtime_tick,
            }
        }
    }
}
