use super::*;

pub fn open_terminal_for_service(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
    run_command: bool,
) -> Result<String, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let service = project
        .services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Service '{service_name}' not found in project '{project_name}'.")
        })?;
    let merged_env = merge_service_env(config, project_name, service_name)?;

    let mut shell_steps = vec![format!("cd {}", shell_quote(&service.workdir))];
    for (key, value) in merged_env.values {
        shell_steps.push(format!("export {key}={}", shell_quote(&value)));
    }
    for (key, value) in terminal_env_pairs(config, project_name, project.services.as_slice()) {
        shell_steps.push(format!("export {key}={}", shell_quote(&value)));
    }
    if run_command {
        shell_steps.push(service.command.clone());
    } else {
        shell_steps.push("clear".to_string());
    }

    let shell_script = shell_steps.join("; ");

    #[cfg(target_os = "macos")]
    {
        run_terminal_script_macos(&shell_script)?;
        if run_command {
            Ok(format!("Opened Terminal and executed '{service_name}'."))
        } else {
            Ok(format!("Opened Terminal for '{service_name}'."))
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = shell_script;
        Err("Open Terminal is currently supported on macOS only.".to_string())
    }
}

pub fn open_terminal_attach_for_service(
    project_name: &str,
    service_name: &str,
    log_file: &Path,
    input_path: &Path,
) -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve Loopbox executable path: {err}"))?;
    let shell_script = format!(
        "exec {} {} --project {} --service {} --log-file {} --input-fifo {}",
        shell_quote(exe.to_string_lossy().as_ref()),
        shell_quote(RUNTIME_ATTACH_SUBCOMMAND),
        shell_quote(project_name),
        shell_quote(service_name),
        shell_quote(log_file.to_string_lossy().as_ref()),
        shell_quote(input_path.to_string_lossy().as_ref()),
    );

    #[cfg(target_os = "macos")]
    {
        run_terminal_script_macos(&shell_script)?;
        Ok(format!(
            "Opened Terminal attach session for '{service_name}'."
        ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = shell_script;
        Err("Attach Terminal is currently supported on macOS only.".to_string())
    }
}

pub(super) fn terminal_env_pairs(
    config: &LoopboxConfig,
    project_name: &str,
    services: &[ServiceConfig],
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let Some(project) = config.projects.get(project_name) else {
        return env;
    };

    env.push(("LOOPBOX_PROJECT".to_string(), project_name.to_string()));
    env.push(("LOOPBOX_DIR".to_string(), project.dir.clone()));
    env.push(("LOOPBOX_IP".to_string(), project.ip.clone()));
    env.push((
        "LOOPBOX_HOST".to_string(),
        project_primary_host(config, project_name),
    ));

    for service in services {
        let key = service.name.to_uppercase();
        let host = service_host_for(project_name, &service.name, &config.global.domain_suffix);
        let effective_ports = service_ports(service);
        let primary_port = effective_ports
            .iter()
            .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
            .map(|entry| entry.port)
            .or_else(|| effective_ports.first().map(|entry| entry.port));
        env.push((
            format!("LOOPBOX_PORT_{key}"),
            primary_port
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ));
        env.push((
            format!("LOOPBOX_PORTS_{key}"),
            effective_ports
                .iter()
                .map(|entry| entry.port.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ));
        env.push((
            format!("LOOPBOX_URL_{key}"),
            format_service_url(&host, primary_port, Some(&project.ip)),
        ));
    }

    env
}

#[cfg(target_os = "macos")]
fn run_terminal_script_macos(shell_script: &str) -> Result<(), String> {
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

#[cfg(target_os = "macos")]
fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
