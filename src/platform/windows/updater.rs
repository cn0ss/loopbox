pub fn init_updater() -> Result<(), String> {
    Ok(())
}
pub fn can_check_for_updates() -> bool {
    false
}
pub fn check_for_updates() -> Result<(), String> {
    Err(
        "Auto-update is not yet available on Windows. Download updates from loopbox.tech."
            .to_string(),
    )
}
pub fn updater_feed_url() -> Option<String> {
    None
}
pub fn updater_automatic_checks_enabled() -> Option<bool> {
    None
}
pub fn updater_last_checked_utc() -> Option<String> {
    None
}
