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

#[cfg(unix)]
pub fn replace_hosts_file_script(content_path: &str) -> String {
    let hosts_path = hosts_file_path();
    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n/bin/cp \"{content_path}\" \"{hosts_path}\"\nrm -f \"{content_path}\"\n"
    )
}

#[cfg(windows)]
pub fn replace_hosts_file_script(content_path: &str) -> String {
    let hosts_path = hosts_file_path();
    format!(
        "@echo off\r\ncopy /Y \"{content_path}\" \"{hosts_path}\" >nul\r\ndel /Q \"{content_path}\" >nul 2>&1\r\n"
    )
}
