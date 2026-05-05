use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run_privileged_script(script_path: &Path) -> Result<(), String> {
    let path_str = script_path.to_string_lossy().replace('\'', "''");

    let ps_command = format!(
        "Start-Process -FilePath 'cmd.exe' -ArgumentList '/c \"{}\"' -Verb RunAs -Wait",
        path_str
    );

    let output = Command::new("powershell")
        .arg("-Command")
        .arg(&ps_command)
        .output()
        .map_err(|err| format!("Failed to invoke Windows UAC elevation prompt: {err}"))?;

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
    let escaped_message = message.replace('\'', "''");
    let _ = action_label; // Windows MessageBox uses generic Yes/No buttons

    let ps_command = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         [System.Windows.Forms.MessageBox]::Show('{}', 'Loopbox', 'YesNo')",
        escaped_message
    );

    let output = Command::new("powershell")
        .arg("-Command")
        .arg(&ps_command)
        .output()
        .map_err(|err| format!("Failed to show dialog: {err}"))?;

    if !output.status.success() {
        return Err(format_output_error(
            "Failed to show confirmation dialog.",
            &output,
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout == "Yes")
}

pub fn write_temp_setup_script(script: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let script_path = std::env::temp_dir().join(format!("loopbox-apply-{nonce}.bat"));

    fs::write(&script_path, script).map_err(|err| {
        format!(
            "Failed to write temp script {}: {err}",
            script_path.display()
        )
    })?;

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
