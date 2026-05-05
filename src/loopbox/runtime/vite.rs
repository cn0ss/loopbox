use super::*;

pub(super) fn apply_bind_hints_to_command(service: &ServiceConfig, bind_ip: &str) -> String {
    let raw_command = service.command.trim();
    if raw_command.is_empty() {
        return service.command.clone();
    }
    let lower = raw_command.to_lowercase();
    let parsed_invocation = parse_script_invocation(raw_command);

    let bind_port = primary_service_port(service);

    if is_expo_like_command(&lower) {
        return append_expo_port_flags(raw_command, bind_port);
    }
    if let Some(invocation) = parsed_invocation.as_ref() {
        if let Some(script_command) = script_command_for_name(&service.workdir, &invocation.script)
        {
            if is_expo_like_command(&script_command.to_lowercase()) {
                return append_expo_flags_to_script_invocation(
                    raw_command,
                    &invocation.manager,
                    bind_port,
                );
            }
        }
    }
    if has_host_flag(&lower) {
        return raw_command.to_string();
    }

    if is_direct_vite_command(&lower) {
        return append_vite_bind_flags(raw_command, bind_port, bind_ip);
    }
    if is_direct_astro_command(&lower) {
        return append_astro_bind_flags(raw_command, bind_port, bind_ip);
    }

    let Some(invocation) = parsed_invocation else {
        return raw_command.to_string();
    };
    let Some((script_command, origin)) = resolve_vite_script_command(&service.workdir, &invocation)
    else {
        if let Some(script_command) = script_command_for_name(&service.workdir, &invocation.script)
        {
            let script_lower = script_command.to_lowercase();
            if is_direct_astro_command(&script_lower) {
                let astro_command = append_astro_bind_flags(&script_command, bind_port, bind_ip);
                return wrap_script_command_for_manager(&invocation.manager, &astro_command);
            }
        }
        return raw_command.to_string();
    };

    match origin {
        ViteScriptOrigin::LocalScript => {
            let vite_command = append_vite_bind_flags(&script_command, bind_port, bind_ip);
            wrap_script_command_for_manager(&invocation.manager, &vite_command)
        }
        ViteScriptOrigin::WorkspaceScript => {
            if let Some(rewritten) = rewrite_workspace_script_invocation_to_vite_exec(
                &invocation,
                &script_command,
                bind_port,
                bind_ip,
            ) {
                return rewritten;
            }
            if has_host_flag(&script_command.to_lowercase()) {
                raw_command.to_string()
            } else {
                append_vite_flags_to_script_invocation(
                    raw_command,
                    &invocation.manager,
                    bind_port,
                    bind_ip,
                )
            }
        }
        ViteScriptOrigin::NestedScriptInvocation { manager } => {
            if let Some(nested_invocation) = parse_script_invocation(&script_command) {
                if let Some((nested_script, nested_origin)) =
                    resolve_vite_script_command(&service.workdir, &nested_invocation)
                {
                    if matches!(nested_origin, ViteScriptOrigin::WorkspaceScript) {
                        if let Some(rewritten) = rewrite_workspace_script_invocation_to_vite_exec(
                            &nested_invocation,
                            &nested_script,
                            bind_port,
                            bind_ip,
                        ) {
                            return rewritten;
                        }
                    }
                }
            }

            let lower_nested = script_command.to_lowercase();
            if is_direct_vite_command(&lower_nested) {
                append_vite_bind_flags(&script_command, bind_port, bind_ip)
            } else if has_host_flag(&lower_nested) {
                script_command
            } else {
                append_vite_flags_to_script_invocation(
                    &script_command,
                    &manager,
                    bind_port,
                    bind_ip,
                )
            }
        }
    }
}

fn is_expo_like_command(command_lower: &str) -> bool {
    command_lower.contains("expo")
        || command_lower.contains("react-native")
        || command_lower.contains("metro")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScriptManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScriptInvocation {
    pub(super) manager: ScriptManager,
    pub(super) script: String,
    pub(super) workspace_filter: Option<String>,
}

pub(super) fn parse_script_invocation(command: &str) -> Option<ScriptInvocation> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let head = tokens[0].to_lowercase();
    match head.as_str() {
        "npm" => {
            if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("run") {
                Some(ScriptInvocation {
                    manager: ScriptManager::Npm,
                    script: tokens[2].to_lowercase(),
                    workspace_filter: None,
                })
            } else {
                None
            }
        }
        "pnpm" => parse_pnpm_script_invocation(&tokens),
        "yarn" => {
            if tokens.len() >= 2 {
                Some(ScriptInvocation {
                    manager: ScriptManager::Yarn,
                    script: tokens[1].to_lowercase(),
                    workspace_filter: None,
                })
            } else {
                None
            }
        }
        "bun" => {
            if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("run") {
                Some(ScriptInvocation {
                    manager: ScriptManager::Bun,
                    script: tokens[2].to_lowercase(),
                    workspace_filter: None,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_pnpm_script_invocation(tokens: &[&str]) -> Option<ScriptInvocation> {
    if tokens.len() < 2 {
        return None;
    }

    let workspace_filter = parse_pnpm_workspace_filter(tokens);
    let mut index = 1_usize;
    while index < tokens.len() {
        let token = tokens[index];
        if token == "--" {
            break;
        }
        if token.eq_ignore_ascii_case("run") {
            index += 1;
            while index < tokens.len() {
                let candidate = tokens[index];
                if candidate == "--" {
                    return None;
                }
                if candidate.starts_with('-') {
                    index += if pnpm_option_takes_value(candidate)
                        && !candidate.contains('=')
                        && index + 1 < tokens.len()
                    {
                        2
                    } else {
                        1
                    };
                    continue;
                }
                return Some(ScriptInvocation {
                    manager: ScriptManager::Pnpm,
                    script: candidate.to_lowercase(),
                    workspace_filter: workspace_filter.clone(),
                });
            }
            return None;
        }
        if token.starts_with('-') {
            index += if pnpm_option_takes_value(token)
                && !token.contains('=')
                && index + 1 < tokens.len()
            {
                2
            } else {
                1
            };
            continue;
        }
        return Some(ScriptInvocation {
            manager: ScriptManager::Pnpm,
            script: token.to_lowercase(),
            workspace_filter,
        });
    }
    None
}

fn parse_pnpm_workspace_filter(tokens: &[&str]) -> Option<String> {
    for (index, token) in tokens.iter().enumerate().skip(1) {
        let lowered = token.to_lowercase();
        if lowered == "--filter" || lowered == "-f" {
            if let Some(value) = tokens.get(index + 1) {
                if !value.starts_with('-') {
                    return Some(value.trim_matches('"').trim_matches('\'').to_string());
                }
            }
            continue;
        }
        if let Some(value) = lowered.strip_prefix("--filter=") {
            let cleaned = value.trim_matches('"').trim_matches('\'');
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

fn pnpm_option_takes_value(token: &str) -> bool {
    matches!(
        token.to_lowercase().as_str(),
        "--filter" | "-f" | "--dir" | "-c" | "--workspace-concurrency" | "--reporter" | "--config"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViteScriptOrigin {
    LocalScript,
    WorkspaceScript,
    NestedScriptInvocation { manager: ScriptManager },
}

pub(super) fn resolve_vite_script_command(
    workdir: &str,
    invocation: &ScriptInvocation,
) -> Option<(String, ViteScriptOrigin)> {
    resolve_vite_script_command_with_depth(workdir, invocation, 0)
}

pub(super) fn resolve_vite_script_command_with_depth(
    workdir: &str,
    invocation: &ScriptInvocation,
    depth: u8,
) -> Option<(String, ViteScriptOrigin)> {
    if depth > 4 {
        return None;
    }

    if let Some(script) = vite_script_command(workdir, &invocation.script) {
        return Some((script, ViteScriptOrigin::LocalScript));
    }
    if let Some(filter) = invocation.workspace_filter.as_deref() {
        if let Some(script) =
            vite_script_command_for_workspace_filter(workdir, filter, &invocation.script)
        {
            return Some((script, ViteScriptOrigin::WorkspaceScript));
        }
    }

    let script_command = script_command_for_name(workdir, &invocation.script)?;
    let nested_invocation = parse_script_invocation(&script_command)?;
    if nested_invocation.manager == invocation.manager
        && nested_invocation.script == invocation.script
        && nested_invocation.workspace_filter == invocation.workspace_filter
    {
        return None;
    }
    if resolve_vite_script_command_with_depth(workdir, &nested_invocation, depth + 1).is_some() {
        return Some((
            script_command,
            ViteScriptOrigin::NestedScriptInvocation {
                manager: nested_invocation.manager,
            },
        ));
    }
    None
}

fn vite_script_command(workdir: &str, script_name: &str) -> Option<String> {
    let script_command = script_command_for_name(workdir, script_name)?;
    if script_command.to_lowercase().contains("vite") {
        Some(script_command)
    } else {
        None
    }
}

fn script_command_for_name(workdir: &str, script_name: &str) -> Option<String> {
    let package_path = PathBuf::from(workdir).join("package.json");
    let Ok(content) = fs::read_to_string(&package_path) else {
        return None;
    };
    let Ok(package_json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return None;
    };
    let scripts = package_json
        .get("scripts")
        .and_then(|value| value.as_object())?;
    let script_command = scripts
        .get(script_name)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|script| !script.is_empty())?;
    Some(script_command.to_string())
}

fn vite_script_command_for_workspace_filter(
    workdir: &str,
    workspace_filter: &str,
    script_name: &str,
) -> Option<String> {
    let suggestions = discover_project_commands(workdir).ok()?;
    let mut candidate_dirs = Vec::new();
    for suggestion in suggestions {
        if !suggestion.script_name.eq_ignore_ascii_case(script_name) {
            continue;
        }
        let Some(package_name) = suggestion.package_name.as_deref() else {
            continue;
        };
        if package_filter_matches(package_name, workspace_filter) {
            candidate_dirs.push(suggestion.workdir);
        }
    }

    candidate_dirs.sort();
    candidate_dirs.dedup();
    for candidate in candidate_dirs {
        if let Some(script) = vite_script_command(&candidate, script_name) {
            return Some(script);
        }
    }

    None
}

fn package_filter_matches(package_name: &str, workspace_filter: &str) -> bool {
    let package = package_name.trim().to_lowercase();
    let mut filter = workspace_filter
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_lowercase();
    filter = filter
        .trim_start_matches("...")
        .trim_end_matches("...")
        .to_string();
    if package.is_empty() || filter.is_empty() {
        return false;
    }
    if package == filter {
        return true;
    }
    if package.contains(&filter) {
        return true;
    }
    if let Some(last_segment) = filter.rsplit('/').next() {
        if !last_segment.is_empty() && package.ends_with(&format!("/{last_segment}")) {
            return true;
        }
    }
    false
}

pub(super) fn is_direct_vite_command(command_lower: &str) -> bool {
    command_lower == "vite"
        || command_lower.starts_with("vite ")
        || command_lower.contains(" vite ")
        || command_lower.contains("vite dev")
}

pub(super) fn is_direct_astro_command(command_lower: &str) -> bool {
    command_lower == "astro"
        || command_lower.starts_with("astro ")
        || command_lower.contains(" astro ")
        || command_lower.contains("astro dev")
}

fn has_host_flag(command_lower: &str) -> bool {
    command_lower.contains("--host")
        || command_lower.contains("--hostname")
        || command_lower.contains(" -h ")
}

fn has_port_flag(command_lower: &str) -> bool {
    command_lower.contains("--port") || command_lower.contains(" -p ")
}

fn has_strict_port_flag(command_lower: &str) -> bool {
    command_lower.contains("--strictport")
}

fn remove_expo_localhost_mode(command: &str) -> String {
    command
        .split_whitespace()
        .filter(|token| !token.eq_ignore_ascii_case("--localhost"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn append_expo_port_flags(command: &str, port: Option<u16>) -> String {
    let mut updated = normalize_vite_command(command);
    updated = remove_expo_localhost_mode(&updated);
    if let Some(port) = port {
        if !has_port_flag(&updated.to_lowercase()) {
            updated.push_str(&format!(" --port {port}"));
        }
    }
    updated
}

fn append_expo_flags_to_script_invocation(
    command: &str,
    manager: &ScriptManager,
    port: Option<u16>,
) -> String {
    let trimmed = remove_expo_localhost_mode(command.trim());
    let Some(port) = port else {
        return trimmed;
    };
    if has_port_flag(&trimmed.to_lowercase()) {
        return trimmed;
    }

    match manager {
        ScriptManager::Yarn => format!("{trimmed} --port {port}"),
        ScriptManager::Npm | ScriptManager::Pnpm | ScriptManager::Bun => {
            if script_args_delimiter_present(&trimmed) {
                format!("{trimmed} --port {port}")
            } else {
                format!("{trimmed} -- --port {port}")
            }
        }
    }
}

fn vite_bind_host(bind_ip: &str) -> String {
    let cleaned = bind_ip.trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("localhost") {
        "0.0.0.0".to_string()
    } else {
        cleaned.to_string()
    }
}

fn append_vite_bind_flags(command: &str, port: Option<u16>, bind_ip: &str) -> String {
    let mut updated = normalize_vite_command(command);
    if has_host_flag(&updated.to_lowercase()) {
        return updated;
    }
    let host = vite_bind_host(bind_ip);
    updated.push_str(&format!(" --host {host}"));
    if let Some(port) = port {
        if !has_port_flag(&updated.to_lowercase()) {
            updated.push_str(&format!(" --port {port}"));
        }
        if !has_strict_port_flag(&updated.to_lowercase()) {
            updated.push_str(" --strictPort");
        }
    }
    updated
}

fn append_astro_bind_flags(command: &str, port: Option<u16>, bind_ip: &str) -> String {
    let mut updated = normalize_vite_command(command);
    if has_host_flag(&updated.to_lowercase()) {
        return updated;
    }
    let host = vite_bind_host(bind_ip);
    updated.push_str(&format!(" --host {host}"));
    if let Some(port) = port {
        if !has_port_flag(&updated.to_lowercase()) {
            updated.push_str(&format!(" --port {port}"));
        }
    }
    updated
}

fn normalize_vite_command(command: &str) -> String {
    let mut tokens: Vec<&str> = command.split_whitespace().collect();
    while matches!(tokens.last(), Some(token) if *token == "--") {
        tokens.pop();
    }
    if tokens.is_empty() {
        command.trim().to_string()
    } else {
        tokens.join(" ")
    }
}

fn rewrite_workspace_script_invocation_to_vite_exec(
    invocation: &ScriptInvocation,
    script_command: &str,
    port: Option<u16>,
    bind_ip: &str,
) -> Option<String> {
    if invocation.manager != ScriptManager::Pnpm {
        return None;
    }
    let filter = invocation.workspace_filter.as_ref()?;
    let vite_command = append_vite_bind_flags(script_command, port, bind_ip);
    Some(format!("pnpm --filter {filter} exec {vite_command}"))
}

fn append_vite_flags_to_script_invocation(
    command: &str,
    manager: &ScriptManager,
    port: Option<u16>,
    bind_ip: &str,
) -> String {
    let flags = vite_bind_flags(port, bind_ip);
    if flags.is_empty() {
        return command.trim().to_string();
    }

    let trimmed = command.trim();
    match manager {
        ScriptManager::Yarn => format!("{trimmed} {flags}"),
        ScriptManager::Npm | ScriptManager::Pnpm | ScriptManager::Bun => {
            if script_args_delimiter_present(trimmed) {
                format!("{trimmed} {flags}")
            } else {
                format!("{trimmed} -- {flags}")
            }
        }
    }
}

fn script_args_delimiter_present(command: &str) -> bool {
    command.split_whitespace().any(|token| token == "--")
}

fn vite_bind_flags(port: Option<u16>, bind_ip: &str) -> String {
    let host = vite_bind_host(bind_ip);
    let mut flags = format!("--host {host}");
    if let Some(port) = port {
        flags.push_str(&format!(" --port {port}"));
        flags.push_str(" --strictPort");
    }
    flags
}

fn wrap_script_command_for_manager(manager: &ScriptManager, vite_command: &str) -> String {
    match manager {
        ScriptManager::Npm => format!("npm exec -- {vite_command}"),
        ScriptManager::Pnpm => format!("pnpm exec {vite_command}"),
        ScriptManager::Yarn => format!("yarn {vite_command}"),
        ScriptManager::Bun => format!("bun x {vite_command}"),
    }
}
