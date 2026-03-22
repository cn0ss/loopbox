use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn run_privileged_script(script_path: &Path) -> Result<(), String> {
    let path_literal = script_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let applescript = format!(
        "do shell script \"bash \" & quoted form of POSIX path of \"{path_literal}\" with administrator privileges"
    );

    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(applescript)
        .output()
        .map_err(|err| format!("Failed to invoke macOS privilege prompt: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format_output_error(
            "System setup failed or was cancelled.",
            &output,
        ))
    }
}

pub fn ask_user_confirmation(message: &str, action_label: &str) -> Result<bool, String> {
    let escaped_message = message
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let escaped_label = action_label
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    let script = format!(
        "display dialog \"{escaped_message}\" buttons {{\"Cancel\", \"{escaped_label}\"}} default button \"{escaped_label}\""
    );

    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|err| format!("Failed to show dialog: {err}"))?;

    Ok(output.status.success())
}

pub fn write_temp_setup_script(script: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let script_path = std::env::temp_dir().join(format!("loopbox-apply-{nonce}.sh"));

    fs::write(&script_path, script).map_err(|err| {
        format!(
            "Failed to write temp script {}: {err}",
            script_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700)).map_err(|err| {
            format!(
                "Failed to set permissions on temp script {}: {err}",
                script_path.display()
            )
        })?;
    }

    Ok(script_path)
}

fn format_output_error(prefix: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !stderr.is_empty() {
        format!("{prefix} {stderr}")
    } else if !stdout.is_empty() {
        format!("{prefix} {stdout}")
    } else {
        format!("{prefix} Exit status: {}", output.status)
    }
}
