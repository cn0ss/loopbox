// Cross-platform hosts file management.
// The /etc/hosts format is identical on macOS, Linux, and Windows.
// Only the file path differs.

#[cfg(unix)]
pub fn hosts_file_path() -> &'static str {
    "/etc/hosts"
}

#[cfg(windows)]
pub fn hosts_file_path() -> &'static str {
    r"C:\Windows\System32\drivers\etc\hosts"
}
