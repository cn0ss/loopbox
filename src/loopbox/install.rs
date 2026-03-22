pub fn ensure_installed_in_applications() -> Result<(), String> {
    crate::platform::install::ensure_installed_in_applications()
}
