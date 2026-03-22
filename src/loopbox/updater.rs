pub fn init_updater() -> Result<(), String> {
    crate::platform::updater::init_updater()
}

pub fn can_check_for_updates() -> bool {
    crate::platform::updater::can_check_for_updates()
}

pub fn check_for_updates() -> Result<(), String> {
    crate::platform::updater::check_for_updates()
}

pub fn updater_feed_url() -> Option<String> {
    crate::platform::updater::updater_feed_url()
}

pub fn updater_automatic_checks_enabled() -> Option<bool> {
    crate::platform::updater::updater_automatic_checks_enabled()
}

pub fn updater_last_checked_utc() -> Option<String> {
    crate::platform::updater::updater_last_checked_utc()
}
