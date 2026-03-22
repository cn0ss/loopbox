/// Cross-platform device identification for license binding.

pub fn resolve_device_identifier() -> Option<String> {
    if let Ok(override_id) = std::env::var("LOOPBOX_LICENSE_DEVICE_ID") {
        let trimmed = override_id.trim().to_ascii_lowercase();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    if let Some(raw) = read_platform_device_id() {
        let normalized = raw.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn read_platform_device_id() -> Option<String> {
    let output = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("IOPlatformUUID") {
            let rhs = line.split('=').nth(1)?.trim();
            let start = rhs.find('"')?;
            let remainder = &rhs[(start + 1)..];
            let end = remainder.find('"')?;
            return Some(remainder[..end].to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_platform_device_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(contents) = std::fs::read_to_string(path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn read_platform_device_id() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.contains("MachineGuid") {
            continue;
        }
        if let Some(value) = line.split_whitespace().last() {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}
