// macOS native dialogs via osascript.
// Moved from app/pages/sandboxes/filesystem.rs.

use std::path::Path;
use std::process::Command;

pub fn select_directory_via_native_dialog(
    start_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let default_dir = start_dir
        .map(expand_posix_tilde)
        .filter(|path| Path::new(path).is_dir());
    let escaped_default = default_dir
        .as_ref()
        .map(|path| escape_applescript_string(path));
    let prompt = "Select project directory for Loopbox";
    let script = if let Some(default) = escaped_default {
        format!(
            r#"POSIX path of (choose folder with prompt "{prompt}" default location POSIX file "{default}")"#
        )
    } else {
        format!(r#"POSIX path of (choose folder with prompt "{prompt}")"#)
    };

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|err| format!("Failed to open native folder picker: {err}"))?;

    if output.status.success() {
        let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if chosen.is_empty() {
            return Ok(None);
        }
        return Ok(Some(chosen));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if stderr.contains("User canceled") || stderr.contains("(-128)") {
        return Ok(None);
    }
    Err(format!(
        "Native folder picker failed: {}",
        stderr.trim().trim_end_matches('.')
    ))
}

fn expand_posix_tilde(raw: &str) -> String {
    let path = raw.trim();
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut expanded = std::path::PathBuf::from(home);
            if path.len() > 2 {
                expanded.push(&path[2..]);
            }
            return expanded.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
