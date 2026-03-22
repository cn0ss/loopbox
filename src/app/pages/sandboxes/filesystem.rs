use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectoryEntry {
    pub(super) name: String,
    pub(super) path: String,
}

pub(super) fn default_browser_path() -> String {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

pub(super) fn expand_tilde_path(raw: &str) -> String {
    let path = raw.trim();
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut expanded = PathBuf::from(home);
            if path.len() > 2 {
                expanded.push(&path[2..]);
            }
            return expanded.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub(super) fn ensure_directory_exists(path: &str) -> Result<String, String> {
    let expanded = expand_tilde_path(path);
    if expanded.is_empty() {
        return Err("Project directory is required.".to_string());
    }
    let candidate = PathBuf::from(&expanded);
    if !candidate.exists() {
        return Err(format!("Directory '{expanded}' does not exist."));
    }
    if !candidate.is_dir() {
        return Err(format!("'{expanded}' is not a directory."));
    }
    Ok(candidate.to_string_lossy().to_string())
}

pub(super) fn select_directory_via_native_dialog(
    start_dir: Option<&str>,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let default_dir = start_dir
            .map(expand_tilde_path)
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

        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|err| format!("Failed to open native folder picker: {err}"))?;

        if output.status.success() {
            let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if chosen.is_empty() {
                return Ok(None);
            }
            return ensure_directory_exists(&chosen).map(Some);
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

    #[cfg(not(target_os = "macos"))]
    {
        let _ = start_dir;
        Err("Native folder picker is currently supported on macOS only.".to_string())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn parent_directory(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
}

pub(super) fn list_child_directories(path: &str) -> Result<Vec<DirectoryEntry>, String> {
    let directory = ensure_directory_exists(path)?;
    let read_dir = fs::read_dir(&directory)
        .map_err(|err| format!("Failed to read directory '{directory}': {err}"))?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name == "." || name == ".." {
            continue;
        }

        entries.push(DirectoryEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
        });
    }

    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}
