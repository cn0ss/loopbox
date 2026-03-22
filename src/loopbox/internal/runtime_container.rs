use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub image: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub volumes: Vec<String>,
    pub auto_remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartContainerInput {
    pub project_name: String,
    pub service_name: String,
    pub bind_ip: String,
    pub ports: Vec<u16>,
    pub container: ContainerSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContainerState {
    pub running: bool,
    pub exit_code: Option<i32>,
}

pub fn start_container(input: &StartContainerInput) -> Result<(), String> {
    let image = input.container.image.trim();
    if image.is_empty() {
        return Err(format!(
            "Service '{}' uses runtime 'container' but container.image is empty.",
            input.service_name
        ));
    }

    let container_name = runtime_container_name(&input.project_name, &input.service_name);
    if let Some(state) = inspect_container(&container_name)? {
        if state.running {
            return Err(format!(
                "Container service '{}' in project '{}' is already running.",
                input.service_name, input.project_name
            ));
        }
        remove_container(&container_name)?;
    }

    let bind_target = normalize_bind_target_ip(&input.bind_ip);
    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(&container_name)
        .arg("--label")
        .arg(format!("loopbox.project={}", input.project_name))
        .arg("--label")
        .arg(format!("loopbox.service={}", input.service_name));

    if input.container.auto_remove {
        command.arg("--rm");
    }

    for env_pair in &input.container.env {
        let trimmed = env_pair.trim();
        if trimmed.is_empty() {
            continue;
        }
        command.arg("-e").arg(trimmed);
    }
    for volume in &input.container.volumes {
        let trimmed = volume.trim();
        if trimmed.is_empty() {
            continue;
        }
        command.arg("-v").arg(trimmed);
    }
    for port in &input.ports {
        command
            .arg("-p")
            .arg(format!("{bind_target}:{port}:{port}"));
    }

    command.arg(image);
    for arg in &input.container.args {
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            continue;
        }
        command.arg(trimmed);
    }

    let output = command.output().map_err(|err| {
        format!(
            "Failed to launch Docker for service '{}' in project '{}': {err}",
            input.service_name, input.project_name
        )
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "Docker run failed for service '{}' in project '{}': {}",
            input.service_name,
            input.project_name,
            if detail.is_empty() {
                "unknown error".to_string()
            } else {
                detail
            }
        ));
    }

    Ok(())
}

pub fn inspect_container(name: &str) -> Result<Option<DockerContainerState>, String> {
    let output = Command::new("docker")
        .arg("inspect")
        .arg("--type")
        .arg("container")
        .arg("--format")
        .arg("{{.State.Running}}|{{.State.ExitCode}}")
        .arg(name)
        .output()
        .map_err(|err| format!("Failed to run docker inspect for container '{name}': {err}"))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if detail.contains("no such object") || detail.contains("no such container") {
            return Ok(None);
        }
        return Err(format!(
            "docker inspect failed for container '{name}': {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = raw.split('|');
    let running = matches!(parts.next().map(str::trim), Some("true"));
    let exit_code = parts.next().and_then(|raw| raw.trim().parse::<i32>().ok());
    Ok(Some(DockerContainerState { running, exit_code }))
}

pub fn remove_container(name: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(name)
        .output()
        .map_err(|err| format!("Failed to run docker rm for container '{name}': {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if detail.contains("no such container") {
        return Ok(());
    }
    Err(format!(
        "docker rm failed for container '{name}': {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

pub fn logs_tail(container_name: &str, limit: usize) -> Result<Option<Vec<String>>, String> {
    if inspect_container(container_name)?.is_none() {
        return Ok(None);
    }

    let output = Command::new("docker")
        .arg("logs")
        .arg("--tail")
        .arg(limit.to_string())
        .arg(container_name)
        .output()
        .map_err(|err| {
            format!("Failed to run docker logs for container '{container_name}': {err}")
        })?;

    if !output.status.success() {
        return Err(format!(
            "docker logs failed for container '{container_name}': {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut lines = Vec::new();
    lines.extend(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.to_string()),
    );
    lines.extend(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(|line| line.to_string()),
    );
    Ok(Some(lines))
}

pub fn runtime_container_name(project_name: &str, service_name: &str) -> String {
    let clean = |value: &str| {
        let mut out = String::new();
        for ch in value.chars() {
            let normalized = ch.to_ascii_lowercase();
            if normalized.is_ascii_alphanumeric() {
                out.push(normalized);
            } else if !out.ends_with('-') {
                out.push('-');
            }
        }
        out.trim_matches('-').to_string()
    };

    let project = clean(project_name);
    let service = clean(service_name);
    let mut name = format!("loopbox-{project}-{service}");
    if name.len() > 63 {
        name.truncate(63);
        while name.ends_with('-') {
            name.pop();
        }
    }
    if name.is_empty() {
        "loopbox-container".to_string()
    } else {
        name
    }
}

fn normalize_bind_target_ip(bind_ip: &str) -> String {
    let trimmed = bind_ip.trim();
    if trimmed.is_empty() {
        "127.0.0.1".to_string()
    } else {
        trimmed.to_string()
    }
}
