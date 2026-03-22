use super::*;

#[allow(dead_code)]
pub(super) fn parse_redaction_list(raw: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.split(',')
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

pub(super) fn save_settings_group(
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut pending_auto_apply: Signal<Option<String>>,
    success_prefix: &str,
    previous_config: Option<LoopboxConfig>,
    allow_system_apply: bool,
) {
    let current = config();
    match loopbox::save_config(&current) {
        Ok(path) => {
            let saved_message = format!("{success_prefix} Saved {}.", path.display());
            let should_apply_system = allow_system_apply
                && !current.projects.is_empty()
                && previous_config
                    .as_ref()
                    .is_some_and(|prev| system_setup_reapply_needed(prev, &current));

            if should_apply_system {
                pending_auto_apply.set(Some(saved_message.clone()));
                notice.set(Some(Notice::info(format!(
                    "{saved_message} Scheduling system setup in background."
                ))));
            } else {
                notice.set(Some(Notice::success(saved_message)));
            }
        }
        Err(err) => notice.set(Some(Notice::error(err))),
    }
}

pub(super) fn system_setup_reapply_needed(
    previous: &LoopboxConfig,
    current: &LoopboxConfig,
) -> bool {
    if loopbox::managed_hosts_block(previous) != loopbox::managed_hosts_block(current) {
        return true;
    }
    loopbox::proxy_redirect_required(previous) != loopbox::proxy_redirect_required(current)
}

pub(super) fn agent_api_bootstrap_prompt(
    base_url: &str,
    openapi_url: &str,
    discovery_path: &str,
    auth_enabled: bool,
    token_path: &str,
) -> String {
    loopbox::agent_api_bootstrap_prompt_for_values(
        base_url,
        openapi_url,
        discovery_path,
        auth_enabled,
        token_path,
    )
}
