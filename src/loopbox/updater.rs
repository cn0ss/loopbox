#[cfg(target_os = "macos")]
use std::ffi::CStr;

#[cfg(target_os = "macos")]
use std::os::raw::c_char;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn loopbox_updater_init() -> bool;
    fn loopbox_updater_can_check() -> bool;
    fn loopbox_updater_check_for_updates() -> bool;
    fn loopbox_updater_last_error() -> *const c_char;
    fn loopbox_updater_feed_url() -> *const c_char;
    fn loopbox_updater_automatic_checks_enabled(value_out: *mut bool) -> bool;
    fn loopbox_updater_last_check_utc() -> *const c_char;
}

#[cfg(target_os = "macos")]
const SKIP_UPDATER_INIT_ENV: &str = "LOOPBOX_SKIP_UPDATER_INIT";

pub fn init_updater() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) || std::env::var_os(SKIP_UPDATER_INIT_ENV).is_some() {
            return Ok(());
        }

        let ok = unsafe { loopbox_updater_init() };
        if ok {
            Ok(())
        } else {
            Err(updater_error_message("Failed to initialize updater."))
        }
    }
}

pub fn can_check_for_updates() -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        false
    }

    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) || std::env::var_os(SKIP_UPDATER_INIT_ENV).is_some() {
            return false;
        }

        let _ = init_updater();
        unsafe { loopbox_updater_can_check() }
    }
}

pub fn check_for_updates() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        Err("In-app updater is currently supported on macOS only.".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) || std::env::var_os(SKIP_UPDATER_INIT_ENV).is_some() {
            return Err("In-app updater is disabled for debug builds.".to_string());
        }

        init_updater()?;
        let ok = unsafe { loopbox_updater_check_for_updates() };
        if ok {
            Ok(())
        } else {
            Err(updater_error_message("Failed to start updater check."))
        }
    }
}

#[cfg(target_os = "macos")]
fn updater_error_message(fallback: &str) -> String {
    match c_string_from_ptr(unsafe { loopbox_updater_last_error() }) {
        Some(message) => message,
        None => fallback.to_string(),
    }
}

pub fn updater_feed_url() -> Option<String> {
    #[cfg(not(target_os = "macos"))]
    {
        None
    }

    #[cfg(target_os = "macos")]
    {
        c_string_from_ptr(unsafe { loopbox_updater_feed_url() })
    }
}

pub fn updater_automatic_checks_enabled() -> Option<bool> {
    #[cfg(not(target_os = "macos"))]
    {
        None
    }

    #[cfg(target_os = "macos")]
    {
        let mut value = false;
        let has_value =
            unsafe { loopbox_updater_automatic_checks_enabled(&mut value as *mut bool) };
        if has_value {
            Some(value)
        } else {
            None
        }
    }
}

pub fn updater_last_checked_utc() -> Option<String> {
    #[cfg(not(target_os = "macos"))]
    {
        None
    }

    #[cfg(target_os = "macos")]
    {
        c_string_from_ptr(unsafe { loopbox_updater_last_check_utc() })
    }
}

#[cfg(target_os = "macos")]
fn c_string_from_ptr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let raw = unsafe { CStr::from_ptr(ptr) };
    let value = raw.to_string_lossy().trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
