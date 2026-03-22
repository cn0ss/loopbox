// Windows Terminal / cmd.exe integration.

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run_terminal_script(shell_script: &str) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let bat_path = std::env::temp_dir().join(format!("loopbox-terminal-{nonce}.bat"));

    fs::write(&bat_path, shell_script).map_err(|err| {
        format!(
            "Failed to write temp batch file {}: {err}",
            bat_path.display()
        )
    })?;

    let bat_str = bat_path.to_string_lossy().to_string();

    // Try Windows Terminal first (wt.exe).
    let wt_result = Command::new("wt.exe")
        .arg("new-tab")
        .arg("cmd.exe")
        .arg("/K")
        .arg(&bat_str)
        .spawn();

    if let Ok(_child) = wt_result {
        return Ok(());
    }

    // Fallback to cmd.exe via `start`.
    let output = Command::new("cmd.exe")
        .arg("/C")
        .arg("start")
        .arg("cmd.exe")
        .arg("/K")
        .arg(&bat_str)
        .output()
        .map_err(|err| format!("Failed to launch cmd.exe terminal: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            Err(format!("Failed to open terminal: {stderr}"))
        } else if !stdout.is_empty() {
            Err(format!("Failed to open terminal: {stdout}"))
        } else {
            Err(format!("Failed to open terminal. Exit: {}", output.status))
        }
    }
}
