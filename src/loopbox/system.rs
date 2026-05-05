use super::projects::{effective_hosts_for_project, validate_project_ip};
use super::{
    reverse_proxy_fallback_port, service_ports, LoopboxConfig, HOSTS_BLOCK_BEGIN, HOSTS_BLOCK_END,
};
use crate::platform;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

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
    platform::networking::apply_networking_script(
        &config.projects,
        &managed_hosts_block(config),
        reverse_proxy_fallback_port(),
    )
}

pub fn apply_system_setup(config: &LoopboxConfig) -> Result<String, String> {
    validate_setup_config(config)?;
    run_setup_script(apply_script(config), apply_success_message(config))
}

pub fn revert_script(config: &LoopboxConfig) -> String {
    platform::networking::revert_networking_script(&config.projects)
}

pub fn revert_system_setup(config: &LoopboxConfig) -> Result<String, String> {
    run_setup_script(revert_script(config), revert_success_message(config))
}

pub fn read_hosts_file() -> Result<String, String> {
    let path = platform::hosts::hosts_file_path();
    fs::read_to_string(path).map_err(|err| format!("Failed to read {path}: {err}"))
}

pub fn save_hosts_file(content: &str) -> Result<String, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let hosts_path = platform::hosts::hosts_file_path();
    let content_path = env::temp_dir().join(format!("loopbox-hosts-{nonce}.tmp"));

    fs::write(&content_path, content)
        .map_err(|err| format!("Failed to write temporary file: {err}"))?;

    let content_path_str = content_path.to_string_lossy().to_string();
    let script = platform::hosts::replace_hosts_file_script(&content_path_str);

    let result = run_setup_script(script, format!("Saved {} successfully.", hosts_path));
    let _ = fs::remove_file(&content_path);
    result
}

pub fn has_changes_outside_managed_block(original: &str, modified: &str) -> bool {
    let (orig_before, orig_after) = content_outside_managed_block(original);
    let (mod_before, mod_after) = content_outside_managed_block(modified);
    orig_before != mod_before || orig_after != mod_after
}

pub fn proxy_redirect_required(config: &LoopboxConfig) -> bool {
    config.projects.values().any(|project| {
        project
            .services
            .iter()
            .any(|service| !service_ports(service).is_empty())
    })
}

pub fn proxy_redirect_configured(config: &LoopboxConfig) -> bool {
    if !proxy_redirect_required(config) {
        return true;
    }
    platform::networking::proxy_redirect_configured_on_system(
        &config.projects,
        reverse_proxy_fallback_port(),
    )
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
    let script_path = platform::privilege::write_temp_setup_script(&script_for_privileged_runner)?;
    let run_result = platform::privilege::run_privileged_script(&script_path);
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

fn apply_success_message(config: &LoopboxConfig) -> String {
    let alias_count = config.projects.len();
    let managed_host_count = managed_service_hosts(config).len();

    format!(
        "Setup complete. Ensured {alias_count} loopback address assignment(s), wrote {managed_host_count} managed hostname mapping(s), configured redirect rules, and refreshed DNS cache."
    )
}

fn revert_success_message(_config: &LoopboxConfig) -> String {
    "Revert complete. Removed loopbox-managed loopback addresses, hosts block, redirect rules, and refreshed DNS cache."
        .to_string()
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
