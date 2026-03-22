pub fn ensure_installed_in_applications() -> Result<(), String> {
    // On Windows, skip the install-to-Applications flow.
    // The app runs from wherever the user placed it.
    Ok(())
}
