use crate::app::models::{Notice, SetupStatus};
use crate::loopbox::{self, LoopboxConfig};
use dioxus::prelude::*;

pub(super) fn preview_project_name(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase().replace(' ', "-");
    let cleaned: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if cleaned.is_empty() {
        "sandbox".to_string()
    } else {
        cleaned
    }
}

pub(super) fn preview_service_name(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect()
}

pub(super) fn preview_suffix(raw: &str) -> String {
    let cleaned = raw.trim().trim_start_matches('.').to_lowercase();
    if cleaned.is_empty() {
        "localhost".to_string()
    } else {
        cleaned
    }
}

pub(super) fn persist_config_and_apply(
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    mut pending_auto_apply: Signal<Option<String>>,
    success_prefix: String,
    previous_config: Option<LoopboxConfig>,
) {
    let current = config();
    match loopbox::save_config(&current) {
        Ok(path) => {
            let saved_message = format!("{success_prefix} Saved {}.", path.display());
            let should_apply_system = !current.projects.is_empty()
                && previous_config
                    .as_ref()
                    .is_some_and(|prev| sandboxes_system_setup_reapply_needed(prev, &current));

            if should_apply_system {
                pending_auto_apply.set(Some(saved_message.clone()));
                set_notice_info(
                    notice,
                    format!("{saved_message} Scheduling system setup in background."),
                );
            } else {
                set_notice_success(notice, saved_message);
            }
        }
        Err(err) => set_notice_error(notice, err),
    }
}

fn sandboxes_system_setup_reapply_needed(
    previous: &LoopboxConfig,
    current: &LoopboxConfig,
) -> bool {
    if loopbox::managed_hosts_block(previous) != loopbox::managed_hosts_block(current) {
        return true;
    }
    loopbox::proxy_redirect_required(previous) != loopbox::proxy_redirect_required(current)
}

pub(super) fn apply_setup_result(
    result: Result<String, String>,
    success_action: &str,
    failure_action: &str,
    notice: Signal<Option<Notice>>,
    mut setup_status: Signal<Option<SetupStatus>>,
) {
    match result {
        Ok(message) => {
            setup_status.set(Some(SetupStatus::success(success_action, message.clone())));
            set_notice_success(notice, message);
        }
        Err(err) => {
            setup_status.set(Some(SetupStatus::error(failure_action, err.clone())));
            set_notice_error(notice, err);
        }
    }
}

pub(super) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| format!("Clipboard init failed: {err}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|err| format!("Clipboard write failed: {err}"))
}

pub(super) fn set_notice_success(mut notice: Signal<Option<Notice>>, message: impl Into<String>) {
    notice.set(Some(Notice::success(message)));
}

pub(super) fn set_notice_error(mut notice: Signal<Option<Notice>>, message: impl Into<String>) {
    notice.set(Some(Notice::error(message)));
}

pub(super) fn set_notice_info(mut notice: Signal<Option<Notice>>, message: impl Into<String>) {
    notice.set(Some(Notice::info(message)));
}
