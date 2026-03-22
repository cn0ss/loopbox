use super::projects::{effective_hosts_for_project, validate_project_ip};
use super::{
    reverse_proxy_fallback_port, service_ports, LoopboxConfig, HOSTS_BLOCK_BEGIN, HOSTS_BLOCK_END,
};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const PF_ANCHOR_NAME: &str = "com.apple/loopbox";
const PF_ANCHOR_PATH: &str = "/etc/pf.anchors/loopbox";
const PROXY_REDIRECT_SOURCE_PORT: u16 = 80;

pub fn managed_hosts_block(config: &LoopboxConfig) -> String {
    let mut lines = vec![HOSTS_BLOCK_BEGIN.to_string()];
    for (project_name, project) in &config.projects {
        let all_hosts =
            effective_hosts_for_project(project_name, project, &config.global.domain_suffix);
        if !all_hosts.is_empty() {
            lines.push(format!("{} {}", project.ip.trim(), all_hosts.join(" ")));
        }
    }
    lines.push(HOSTS_BLOCK_END.to_string());
    lines.join("\n")
}

pub fn apply_script(config: &LoopboxConfig) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str("set -euo pipefail\n\n");
    script.push_str("# 1) Ensure loopback aliases exist\n");

    if config.projects.is_empty() {
        script.push_str("# No projects configured yet.\n");
    } else {
        for project in config.projects.values() {
            script.push_str(&format!(
                "if ! /sbin/ifconfig lo0 | /usr/bin/grep -Fq \"inet {} \"; then\n",
                project.ip
            ));
            script.push_str(&format!(
                "  sudo /sbin/ifconfig lo0 alias {} up\n",
                project.ip
            ));
            script.push_str("fi\n");
        }
    }

    script.push_str("\n# 2) Rewrite the managed loopbox block in /etc/hosts\n");
    script.push_str("tmp_file=\"$(mktemp)\"\n");
    script.push_str("/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str("inside==0 {print}' /etc/hosts > \"$tmp_file\"\n");
    script.push_str("cat >> \"$tmp_file\" <<'LOOPBOX_HOSTS'\n");
    script.push_str(&managed_hosts_block(config));
    script.push_str("\nLOOPBOX_HOSTS\n");
    script.push_str("sudo /bin/cp \"$tmp_file\" /etc/hosts\n");
    script.push_str("rm -f \"$tmp_file\"\n");

    script.push_str(
        "\n# 3) Configure pf redirect for domain-only HTTP access (:80 -> proxy fallback)\n",
    );
    script.push_str("proxy_pf_tmp=\"$(mktemp)\"\n");
    script.push_str("cat > \"$proxy_pf_tmp\" <<'LOOPBOX_PF'\n");
    script.push_str(&managed_proxy_pf_anchor(config));
    script.push_str("\nLOOPBOX_PF\n");
    script.push_str(&format!(
        "sudo /bin/cp \"$proxy_pf_tmp\" {}\n",
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("rm -f \"$proxy_pf_tmp\"\n");
    script.push_str(&format!(
        "if /usr/bin/grep -Eq '^rdr ' {}; then\n",
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("  sudo /sbin/pfctl -E >/dev/null 2>&1 || true\n");
    script.push_str(&format!(
        "  sudo /sbin/pfctl -a {} -f {}\n",
        shell_quote(PF_ANCHOR_NAME),
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("else\n");
    script.push_str(&format!(
        "  sudo /sbin/pfctl -a {} -F all >/dev/null 2>&1 || true\n",
        shell_quote(PF_ANCHOR_NAME)
    ));
    script.push_str("fi\n");

    script.push_str("\n# 4) Refresh macOS DNS cache after hosts changes\n");
    script.push_str("sudo /usr/bin/dscacheutil -flushcache >/dev/null 2>&1 || true\n");
    script.push_str("sudo /usr/bin/killall -HUP mDNSResponder >/dev/null 2>&1 || true\n");
    script
}

pub fn apply_system_setup(config: &LoopboxConfig) -> Result<String, String> {
    validate_setup_config(config)?;
    run_setup_script(apply_script(config), apply_success_message(config))
}

pub fn revert_script(config: &LoopboxConfig) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str("set -euo pipefail\n\n");
    script.push_str("# 1) Remove loopback aliases from current config and legacy hosts entries\n");
    for ip in config.projects.values().map(|project| project.ip.trim()) {
        script.push_str(&format!(
            "sudo /sbin/ifconfig lo0 -alias \"{}\" >/dev/null 2>&1 || true\n",
            ip
        ));
    }
    script.push_str("legacy_ips=\"$(/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str(
        "inside==1 && $1 ~ /^127\\./ && $1 != \"127.0.0.1\" {print $1}' /etc/hosts | /usr/bin/sort -u)\"\n",
    );
    script.push_str("while IFS= read -r ip; do\n");
    script.push_str("  [ -n \"$ip\" ] || continue\n");
    script.push_str("  sudo /sbin/ifconfig lo0 -alias \"$ip\" >/dev/null 2>&1 || true\n");
    script.push_str("done <<< \"$legacy_ips\"\n");

    script.push_str("\n# 2) Remove the managed loopbox block from /etc/hosts\n");
    script.push_str("tmp_file=\"$(mktemp)\"\n");
    script.push_str("/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str("inside==0 {print}' /etc/hosts > \"$tmp_file\"\n");
    script.push_str("sudo /bin/cp \"$tmp_file\" /etc/hosts\n");
    script.push_str("rm -f \"$tmp_file\"\n");

    script.push_str("\n# 3) Remove loopbox pf anchor redirect rules\n");
    script.push_str(&format!(
        "sudo /sbin/pfctl -a {} -F all >/dev/null 2>&1 || true\n",
        shell_quote(PF_ANCHOR_NAME)
    ));
    script.push_str(&format!(
        "sudo /bin/rm -f {}\n",
        shell_quote(PF_ANCHOR_PATH)
    ));

    script.push_str("\n# 4) Refresh macOS DNS cache after hosts changes\n");
    script.push_str("sudo /usr/bin/dscacheutil -flushcache >/dev/null 2>&1 || true\n");
    script.push_str("sudo /usr/bin/killall -HUP mDNSResponder >/dev/null 2>&1 || true\n");
    script
}

pub fn revert_system_setup(config: &LoopboxConfig) -> Result<String, String> {
    run_setup_script(revert_script(config), revert_success_message(config))
}

pub fn read_hosts_file() -> Result<String, String> {
    fs::read_to_string("/etc/hosts").map_err(|err| format!("Failed to read /etc/hosts: {err}"))
}

pub fn save_hosts_file(content: &str) -> Result<String, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let content_path = env::temp_dir().join(format!("loopbox-hosts-{nonce}.tmp"));

    fs::write(&content_path, content)
        .map_err(|err| format!("Failed to write temporary file: {err}"))?;

    let content_path_str = content_path.to_string_lossy().to_string();
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n/bin/cp \"{}\" /etc/hosts\nrm -f \"{}\"\n",
        content_path_str, content_path_str
    );

    let result = run_setup_script(script, "Saved /etc/hosts successfully.".to_string());
    let _ = fs::remove_file(&content_path);
    result
}

pub fn has_changes_outside_managed_block(original: &str, modified: &str) -> bool {
    let (orig_before, orig_after) = content_outside_managed_block(original);
    let (mod_before, mod_after) = content_outside_managed_block(modified);
    orig_before != mod_before || orig_after != mod_after
}

pub fn proxy_redirect_required(config: &LoopboxConfig) -> bool {
    !proxy_redirect_target_ips(config).is_empty()
}

pub fn proxy_redirect_configured(config: &LoopboxConfig) -> bool {
    if !proxy_redirect_required(config) {
        return true;
    }

    let Ok(anchor_content) = fs::read_to_string(PF_ANCHOR_PATH) else {
        return false;
    };
    let fallback_port = reverse_proxy_fallback_port();
    let has_rdr = anchor_content.lines().any(|line| {
        let clean = line.trim();
        clean.starts_with("rdr ")
            && clean.contains(" on lo0 ")
            && clean.contains(&format!(" port {PROXY_REDIRECT_SOURCE_PORT}"))
            && clean.contains("-> 127.0.0.1")
            && clean.contains(&format!(" port {fallback_port}"))
    });
    if !has_rdr {
        return false;
    }
    for ip in proxy_redirect_target_ips(config) {
        if !anchor_content.contains(&ip) {
            return false;
        }
    }
    true
}

fn managed_proxy_pf_anchor(config: &LoopboxConfig) -> String {
    let mut lines = vec![
        "# loopbox managed pf anchor".to_string(),
        "# Enables domain-only access by redirecting sandbox-ip:80 to the internal proxy fallback port.".to_string(),
    ];

    let ips = proxy_redirect_target_ips(config);
    if ips.is_empty() {
        lines.push("# no networked services configured".to_string());
        return lines.join("\n");
    }

    let fallback_port = reverse_proxy_fallback_port();
    let ip_list = ips.join(", ");
    lines.push(format!(
        "rdr pass on lo0 inet proto tcp from any to {{ {ip_list} }} port {PROXY_REDIRECT_SOURCE_PORT} -> 127.0.0.1 port {fallback_port}"
    ));
    lines.join("\n")
}

fn proxy_redirect_target_ips(config: &LoopboxConfig) -> Vec<String> {
    let mut ips = BTreeSet::new();
    for project in config.projects.values() {
        if !project
            .services
            .iter()
            .any(|service| !service_ports(service).is_empty())
        {
            continue;
        }
        let candidate = project.ip.trim();
        if candidate.parse::<IpAddr>().is_ok() {
            ips.insert(candidate.to_string());
        }
    }
    ips.into_iter().collect()
}

fn content_outside_managed_block(content: &str) -> (String, String) {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut inside = false;
    let mut past_block = false;

    for line in content.lines() {
        if line == HOSTS_BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if line == HOSTS_BLOCK_END {
            inside = false;
            past_block = true;
            continue;
        }
        if inside {
            continue;
        }
        if past_block {
            after.push(line);
        } else {
            before.push(line);
        }
    }

    (before.join("\n"), after.join("\n"))
}

fn validate_setup_config(config: &LoopboxConfig) -> Result<(), String> {
    for (name, project) in &config.projects {
        if let Err(err) = validate_project_ip(&config.global, project.ip.trim()) {
            return Err(format!(
                "Refusing to run privileged setup: project '{name}' has invalid IP '{}': {err}",
                project.ip.trim()
            ));
        }
    }
    Ok(())
}

fn run_setup_script(script: String, success_message: String) -> Result<String, String> {
    let script_for_privileged_runner = strip_redundant_sudo_commands(&script);
    let script_path = write_temp_setup_script(&script_for_privileged_runner)?;
    let run_result = run_privileged_script(&script_path);
    let cleanup_result = fs::remove_file(&script_path);

    match (run_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(success_message),
        (Ok(()), Err(err)) => Ok(format!(
            "{success_message} Temporary script cleanup failed ({}): {err}",
            script_path.display()
        )),
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(cleanup_err)) => Err(format!(
            "{err} Temporary script cleanup failed ({}): {cleanup_err}",
            script_path.display()
        )),
    }
}

fn strip_redundant_sudo_commands(script: &str) -> String {
    let mut output = String::new();
    for (index, line) in script.lines().enumerate() {
        if index > 0 {
            output.push('\n');
        }

        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("sudo ") {
            let indent_len = line.len().saturating_sub(trimmed.len());
            output.push_str(&line[..indent_len]);
            output.push_str(rest);
        } else {
            output.push_str(line);
        }
    }
    if script.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn write_temp_setup_script(script: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let script_path = env::temp_dir().join(format!("loopbox-apply-{nonce}.sh"));

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

fn run_privileged_script(script_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        run_script_with_osascript(script_path)
    }

    #[cfg(not(target_os = "macos"))]
    {
        run_script_with_sudo(script_path)
    }
}

#[cfg(target_os = "macos")]
fn run_script_with_osascript(script_path: &Path) -> Result<(), String> {
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

#[cfg(not(target_os = "macos"))]
fn run_script_with_sudo(script_path: &Path) -> Result<(), String> {
    let output = Command::new("sudo")
        .arg("/bin/bash")
        .arg(script_path)
        .output()
        .map_err(|err| format!("Failed to run setup with sudo: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format_output_error("System setup failed.", &output))
    }
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

fn apply_success_message(config: &LoopboxConfig) -> String {
    let alias_count = config.projects.len();
    let managed_host_count = managed_service_hosts(config).len();
    let redirect_ips = proxy_redirect_target_ips(config).len();

    format!(
        "Setup complete. Ensured {alias_count} loopback alias(es), wrote {managed_host_count} managed hostname mapping(s), configured pf redirect for {redirect_ips} IP(s), and refreshed DNS cache."
    )
}

fn revert_success_message(_config: &LoopboxConfig) -> String {
    "Revert complete. Removed loopbox-managed aliases, hosts block, pf redirect anchor, and refreshed DNS cache."
        .to_string()
}

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn managed_service_hosts(config: &LoopboxConfig) -> Vec<String> {
    let mut hosts = BTreeSet::new();
    for (project_name, project) in &config.projects {
        for host in effective_hosts_for_project(project_name, project, &config.global.domain_suffix)
        {
            let clean = host.trim().to_lowercase();
            if !clean.is_empty() {
                hosts.insert(clean);
            }
        }
    }
    hosts.into_iter().collect()
}

#[cfg(test)]
mod tests;
