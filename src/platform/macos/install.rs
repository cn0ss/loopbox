// macOS app installation: .app bundle, /Applications, ditto.
// Moved from loopbox/install.rs.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SKIP_INSTALL_PROMPT_ENV: &str = "LOOPBOX_SKIP_INSTALL_PROMPT";

pub fn ensure_installed_in_applications() -> Result<(), String> {
    if cfg!(debug_assertions) || std::env::var_os(SKIP_INSTALL_PROMPT_ENV).is_some() {
        return Ok(());
    }

    let Some(source_bundle) = current_app_bundle_path() else {
        return Ok(());
    };

    if is_inside_applications(&source_bundle) {
        return Ok(());
    }

    let app_name = source_bundle
        .file_name()
        .ok_or_else(|| "Failed to determine app bundle name.".to_string())?
        .to_string_lossy()
        .to_string();
    let destination_bundle = Path::new("/Applications").join(&app_name);

    if destination_bundle.exists() {
        let should_open_existing = ask_user_choice(
            "Loopbox is already installed in Applications. Open the installed copy now?",
            "Open",
        )?;
        if should_open_existing {
            open_bundle(&destination_bundle)?;
            std::process::exit(0);
        }
        return Ok(());
    }

    let should_move = ask_user_choice(
        "Loopbox runs best from Applications. Move it now and relaunch from there?",
        "Move",
    )?;
    if !should_move {
        return Ok(());
    }

    match move_bundle_to_applications(&source_bundle, &destination_bundle) {
        Ok(()) => {
            open_bundle(&destination_bundle)?;
            std::process::exit(0);
        }
        Err(err) if is_user_cancelled(&err) => Ok(()),
        Err(err) => Err(err),
    }
}

fn current_app_bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }
    let bundle_dir = contents_dir.parent()?;
    if bundle_dir.extension()? != "app" {
        return None;
    }

    fs::canonicalize(bundle_dir)
        .ok()
        .or_else(|| Some(bundle_dir.to_path_buf()))
}

fn is_inside_applications(bundle: &Path) -> bool {
    if bundle.starts_with("/Applications") {
        return true;
    }

    if let Some(home) = std::env::var_os("HOME") {
        let user_applications = Path::new(&home).join("Applications");
        if bundle.starts_with(user_applications) {
            return true;
        }
    }

    false
}

fn ask_user_choice(message: &str, action_label: &str) -> Result<bool, String> {
    let escaped_message = escape_applescript_string(message);
    let escaped_action = escape_applescript_string(action_label);
    let script = format!(
        "display dialog \"{escaped_message}\" buttons {{\"Not now\", \"{escaped_action}\"}} default button \"{escaped_action}\" with icon note"
    );

    match run_osascript_inline(&script) {
        Ok(stdout) => Ok(stdout.contains(&format!("button returned:{action_label}"))),
        Err(err) if is_user_cancelled(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn move_bundle_to_applications(source: &Path, destination: &Path) -> Result<(), String> {
    let script = format!(
        "#!/bin/bash\nset -euo pipefail\nsrc={}\ndst={}\n/usr/bin/ditto \"$src\" \"$dst\"\n/usr/bin/xattr -dr com.apple.quarantine \"$dst\" >/dev/null 2>&1 || true\n",
        shell_quote(source.to_string_lossy().as_ref()),
        shell_quote(destination.to_string_lossy().as_ref())
    );
    let script_path = write_temp_script(&script)?;
    let result = run_privileged_script_via_osascript(&script_path);
    let _ = fs::remove_file(&script_path);
    result
}

fn write_temp_script(script_content: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let script_path = std::env::temp_dir().join(format!("loopbox-install-{nonce}.sh"));
    fs::write(&script_path, script_content).map_err(|err| {
        format!(
            "Failed to write temporary install script {}: {err}",
            script_path.display()
        )
    })?;
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700)).map_err(|err| {
        format!(
            "Failed to set permissions on temporary install script {}: {err}",
            script_path.display()
        )
    })?;
    Ok(script_path)
}

fn run_privileged_script_via_osascript(script_path: &Path) -> Result<(), String> {
    let path_literal = script_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let applescript = format!(
        "do shell script \"bash \" & quoted form of POSIX path of \"{path_literal}\" with administrator privileges"
    );
    run_osascript_inline(&applescript).map(|_| ())
}

fn run_osascript_inline(script: &str) -> Result<String, String> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|err| format!("Failed to invoke macOS dialog: {err}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            Err(stderr)
        } else if !stdout.is_empty() {
            Err(stdout)
        } else {
            Err(format!("osascript exited with {}", output.status))
        }
    }
}

fn open_bundle(bundle_path: &Path) -> Result<(), String> {
    let output = Command::new("/usr/bin/open")
        .arg(bundle_path)
        .output()
        .map_err(|err| format!("Failed to relaunch app from Applications: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            Err(format!("Failed to relaunch app: {stderr}"))
        } else if !stdout.is_empty() {
            Err(format!("Failed to relaunch app: {stdout}"))
        } else {
            Err(format!("Failed to relaunch app. Exit: {}", output.status))
        }
    }
}

fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_user_cancelled(err: &str) -> bool {
    err.contains("User canceled")
        || err.contains("User cancelled")
        || err.contains("(-128)")
        || err.to_lowercase().contains("cancelled")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
