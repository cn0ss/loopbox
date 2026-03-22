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
    match crate::platform::dialog::select_directory_via_native_dialog(start_dir)? {
        Some(chosen) => ensure_directory_exists(&chosen).map(Some),
        None => Ok(None),
    }
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
