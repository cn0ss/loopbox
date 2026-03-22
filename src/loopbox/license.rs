pub use super::internal::license::{BuildChannel, LicenseTier};

pub fn build_channel() -> BuildChannel {
    super::internal::license::build_channel()
}

pub fn license_activation_available() -> bool {
    super::internal::license::license_activation_available()
}

pub fn init_license_state_at_startup() -> Result<(), String> {
    super::internal::license::init_license_state_at_startup()
}

pub fn start_periodic_license_revalidation() -> Result<(), String> {
    super::internal::license::start_periodic_license_revalidation()
}

pub fn activate_license_key(key: &str) -> Result<(), String> {
    super::internal::license::activate_license_key(key)
}

pub fn current_license_tier() -> LicenseTier {
    super::internal::license::current_license_tier()
}

pub fn license_status_label() -> String {
    super::internal::license::license_status_label()
}
