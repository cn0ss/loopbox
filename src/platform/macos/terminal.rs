// macOS Terminal.app integration via AppleScript.
// Moved from runtime/terminal.rs.

use std::process::Command;

pub fn run_terminal_script(shell_script: &str) -> Result<(), String> {
    let escaped = escape_applescript_string(shell_script);
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg("tell application \"Terminal\"")
        .arg("-e")
        .arg("activate")
        .arg("-e")
        .arg(format!("do script \"{escaped}\""))
        .arg("-e")
        .arg("end tell")
        .output()
        .map_err(|err| format!("Failed to launch macOS Terminal: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            Err(format!("Failed to open Terminal: {stderr}"))
        } else if !stdout.is_empty() {
            Err(format!("Failed to open Terminal: {stdout}"))
        } else {
            Err(format!("Failed to open Terminal. Exit: {}", output.status))
        }
    }
}

fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
