// Windows native folder picker via PowerShell and System.Windows.Forms.

use std::process::Command;

pub fn select_directory_via_native_dialog(
    start_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let initial_dir = start_dir
        .map(expand_windows_path)
        .filter(|path| std::path::Path::new(path).is_dir());

    let set_initial = if let Some(ref dir) = initial_dir {
        let escaped = dir.replace('\'', "''");
        format!("$f.SelectedPath = '{}'; ", escaped)
    } else {
        String::new()
    };

    let ps_command = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $f = New-Object System.Windows.Forms.FolderBrowserDialog; \
         $f.Description = 'Select project directory for Loopbox'; \
         {set_initial}\
         if ($f.ShowDialog() -eq 'OK') {{ $f.SelectedPath }} else {{ '' }}"
    );

    let output = Command::new("powershell")
        .arg("-Command")
        .arg(&ps_command)
        .output()
        .map_err(|err| format!("Failed to open native folder picker: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!(
            "Native folder picker failed: {}",
            stderr.trim().trim_end_matches('.')
        ));
    }

    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if chosen.is_empty() {
        Ok(None)
    } else {
        Ok(Some(chosen))
    }
}

fn expand_windows_path(raw: &str) -> String {
    let path = raw.trim();
    if path.starts_with('~') {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            let mut expanded = std::path::PathBuf::from(profile);
            let rest = path.strip_prefix('~').unwrap_or("");
            let rest = rest.strip_prefix('\\').or_else(|| rest.strip_prefix('/')).unwrap_or(rest);
            if !rest.is_empty() {
                expanded.push(rest);
            }
            return expanded.to_string_lossy().to_string();
        }
    }
    path.to_string()
}
