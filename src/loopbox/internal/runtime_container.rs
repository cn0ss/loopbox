use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
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

#[derive(Debug, Clone, PartialEq)]
pub struct DockerContainerStats {
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub process_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerRuntimeStatus {
    Ready,
    CliMissing,
    DaemonUnavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingContainerStartAction {
    Continue,
    RemoveBeforeStart,
    AlreadyRunning,
}

pub fn start_container(input: &StartContainerInput) -> Result<(), String> {
    ensure_docker_runtime_ready()?;

    let image = input.container.image.trim();
    if image.is_empty() {
        return Err(format!(
            "Service '{}' uses runtime 'container' but container.image is empty.",
            input.service_name
        ));
    }

    let container_name = runtime_container_name(&input.project_name, &input.service_name);
    match existing_container_start_action(inspect_container(&container_name)?) {
        ExistingContainerStartAction::AlreadyRunning => {
            return Err(format!(
                "Container service '{}' in project '{}' is already running.",
                input.service_name, input.project_name
            ));
        }
        ExistingContainerStartAction::RemoveBeforeStart => remove_container(&container_name)?,
        ExistingContainerStartAction::Continue => {}
    }

    let args = docker_run_args(input)?;
    let mut command = Command::new("docker");
    command.args(&args);

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

fn docker_run_args(input: &StartContainerInput) -> Result<Vec<String>, String> {
    let image = input.container.image.trim();
    if image.is_empty() {
        return Err(format!(
            "Service '{}' uses runtime 'container' but container.image is empty.",
            input.service_name
        ));
    }

    let container_name = runtime_container_name(&input.project_name, &input.service_name);
    let bind_target = normalize_bind_target_ip(&input.bind_ip);
    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        container_name,
        "--label".to_string(),
        format!("loopbox.project={}", input.project_name),
        "--label".to_string(),
        format!("loopbox.service={}", input.service_name),
    ];

    if input.container.auto_remove {
        args.push("--rm".to_string());
    }

    for env_pair in &input.container.env {
        let trimmed = env_pair.trim();
        if !trimmed.is_empty() {
            args.push("-e".to_string());
            args.push(trimmed.to_string());
        }
    }
    for volume in &input.container.volumes {
        let trimmed = volume.trim();
        if !trimmed.is_empty() {
            args.push("-v".to_string());
            args.push(trimmed.to_string());
        }
    }
    for port in &input.ports {
        args.push("-p".to_string());
        args.push(format!("{bind_target}:{port}:{port}"));
    }

    args.push(image.to_string());
    for arg in &input.container.args {
        let trimmed = arg.trim();
        if !trimmed.is_empty() {
            args.push(trimmed.to_string());
        }
    }

    Ok(args)
}

fn existing_container_start_action(
    state: Option<DockerContainerState>,
) -> ExistingContainerStartAction {
    match state {
        None => ExistingContainerStartAction::Continue,
        Some(state) if state.running => ExistingContainerStartAction::AlreadyRunning,
        Some(_) => ExistingContainerStartAction::RemoveBeforeStart,
    }
}

pub fn docker_runtime_status() -> DockerRuntimeStatus {
    match Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .output()
    {
        Ok(output) => docker_status_from_version_probe(
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).trim(),
        ),
        Err(err) if err.kind() == ErrorKind::NotFound => DockerRuntimeStatus::CliMissing,
        Err(err) => {
            DockerRuntimeStatus::DaemonUnavailable(format!("failed to run docker version: {err}"))
        }
    }
}

pub fn docker_runtime_unavailable_message(status: &DockerRuntimeStatus) -> Option<String> {
    match status {
        DockerRuntimeStatus::Ready => None,
        DockerRuntimeStatus::CliMissing => {
            Some("Docker CLI is not installed or is not available on PATH.".to_string())
        }
        DockerRuntimeStatus::DaemonUnavailable(detail) => Some(format!(
            "Docker CLI is installed, but the Docker daemon is not reachable: {detail}"
        )),
    }
}

fn ensure_docker_runtime_ready() -> Result<(), String> {
    let status = docker_runtime_status();
    if let Some(message) = docker_runtime_unavailable_message(&status) {
        Err(message)
    } else {
        Ok(())
    }
}

fn docker_status_from_version_probe(success: bool, stderr: &str) -> DockerRuntimeStatus {
    if success {
        return DockerRuntimeStatus::Ready;
    }

    let detail = stderr.trim();
    DockerRuntimeStatus::DaemonUnavailable(if detail.is_empty() {
        "docker version exited with a non-zero status".to_string()
    } else {
        detail.to_string()
    })
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

    docker_inspect_state_from_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
        &String::from_utf8_lossy(&output.stdout),
    )
    .map_err(|err| {
        if err.starts_with("docker inspect failed") {
            err.replace(
                "docker inspect failed",
                &format!("docker inspect failed for container '{name}'"),
            )
        } else {
            err
        }
    })
}

fn docker_inspect_state_from_output(
    success: bool,
    stderr: &str,
    stdout: &str,
) -> Result<Option<DockerContainerState>, String> {
    if !success {
        let detail = stderr.to_ascii_lowercase();
        if detail.contains("no such object") || detail.contains("no such container") {
            return Ok(None);
        }
        return Err(format!("docker inspect failed: {}", stderr.trim()));
    }

    let raw = stdout.trim();
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
    docker_remove_result_from_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
    )
    .map_err(|err| {
        err.replace(
            "docker rm failed",
            &format!("docker rm failed for container '{name}'"),
        )
    })
}

fn docker_remove_result_from_output(success: bool, stderr: &str) -> Result<(), String> {
    if success {
        return Ok(());
    }
    let detail = stderr.to_ascii_lowercase();
    if detail.contains("no such container") {
        return Ok(());
    }
    Err(format!("docker rm failed: {}", stderr.trim()))
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

    let lines = docker_logs_lines_from_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
        &String::from_utf8_lossy(&output.stdout),
    )
    .map_err(|err| {
        err.replace(
            "docker logs failed",
            &format!("docker logs failed for container '{container_name}'"),
        )
    })?;
    Ok(Some(lines))
}

pub fn container_resource_stats(name: &str) -> Result<Option<DockerContainerStats>, String> {
    let output = Command::new("docker")
        .arg("stats")
        .arg("--no-stream")
        .arg("--format")
        .arg("{{.CPUPerc}}|{{.MemUsage}}|{{.PIDs}}")
        .arg(name)
        .output()
        .map_err(|err| format!("Failed to run docker stats for container '{name}': {err}"))?;

    docker_container_stats_from_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
        &String::from_utf8_lossy(&output.stdout),
    )
    .map_err(|err| {
        if err.starts_with("docker stats failed") {
            err.replace(
                "docker stats failed",
                &format!("docker stats failed for container '{name}'"),
            )
        } else {
            err
        }
    })
}

fn docker_container_stats_from_output(
    success: bool,
    stderr: &str,
    stdout: &str,
) -> Result<Option<DockerContainerStats>, String> {
    if !success {
        let detail = stderr.to_ascii_lowercase();
        if detail.contains("no such object") || detail.contains("no such container") {
            return Ok(None);
        }
        return Err(format!("docker stats failed: {}", stderr.trim()));
    }

    let raw = stdout.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut parts = raw.split('|');
    let cpu_percent = parts.next().and_then(parse_docker_percent);
    let memory_bytes = parts.next().and_then(parse_docker_memory_usage_bytes);
    let process_count = parts
        .next()
        .and_then(|raw| raw.trim().parse::<usize>().ok());

    Ok(Some(DockerContainerStats {
        cpu_percent,
        memory_bytes,
        process_count,
    }))
}

fn parse_docker_percent(raw: &str) -> Option<f64> {
    raw.trim().trim_end_matches('%').trim().parse::<f64>().ok()
}

fn parse_docker_memory_usage_bytes(raw: &str) -> Option<u64> {
    let used = raw.split('/').next()?.trim();
    parse_docker_size_bytes(used)
}

fn parse_docker_size_bytes(raw: &str) -> Option<u64> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }

    let mut split_at = value.len();
    for (index, ch) in value.char_indices() {
        if !(ch.is_ascii_digit() || ch == '.') {
            split_at = index;
            break;
        }
    }
    let number = value[..split_at].trim().parse::<f64>().ok()?;
    let unit = value[split_at..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1.0,
        "kb" => 1_000.0,
        "mb" => 1_000_000.0,
        "gb" => 1_000_000_000.0,
        "tb" => 1_000_000_000_000.0,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier).round() as u64)
}

fn docker_logs_lines_from_output(
    success: bool,
    stderr: &str,
    stdout: &str,
) -> Result<Vec<String>, String> {
    if !success {
        return Err(format!("docker logs failed: {}", stderr.trim()));
    }

    let mut lines = Vec::new();
    lines.extend(stdout.lines().map(|line| line.to_string()));
    lines.extend(stderr.lines().map(|line| line.to_string()));
    Ok(lines)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_status_messages_distinguish_missing_cli_and_daemon() {
        assert_eq!(
            docker_runtime_unavailable_message(&DockerRuntimeStatus::CliMissing),
            Some("Docker CLI is not installed or is not available on PATH.".to_string())
        );
        assert_eq!(
            docker_runtime_unavailable_message(&DockerRuntimeStatus::DaemonUnavailable(
                "Cannot connect to the Docker daemon".to_string()
            )),
            Some(
                "Docker CLI is installed, but the Docker daemon is not reachable: Cannot connect to the Docker daemon"
                    .to_string()
            )
        );
        assert_eq!(
            docker_runtime_unavailable_message(&DockerRuntimeStatus::Ready),
            None
        );
    }

    #[test]
    fn failed_docker_version_probe_is_daemon_unavailable() {
        assert_eq!(
            docker_status_from_version_probe(false, "Cannot connect to the Docker daemon"),
            DockerRuntimeStatus::DaemonUnavailable(
                "Cannot connect to the Docker daemon".to_string()
            )
        );
        assert_eq!(
            docker_status_from_version_probe(false, ""),
            DockerRuntimeStatus::DaemonUnavailable(
                "docker version exited with a non-zero status".to_string()
            )
        );
    }

    #[test]
    fn docker_run_args_bind_ports_and_include_container_options() {
        let input = StartContainerInput {
            project_name: "Demo App".to_string(),
            service_name: "Postgres DB".to_string(),
            bind_ip: "127.0.0.61".to_string(),
            ports: vec![5432, 9187],
            container: ContainerSpec {
                image: "postgres:16-alpine".to_string(),
                args: vec![
                    "postgres".to_string(),
                    "-c".to_string(),
                    "fsync=off".to_string(),
                ],
                env: vec!["POSTGRES_PASSWORD=loopbox".to_string()],
                volumes: vec!["pgdata:/var/lib/postgresql/data".to_string()],
                auto_remove: true,
            },
        };

        let args = docker_run_args(&input).expect("docker run args");

        assert_eq!(args[0], "run");
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.contains(&"--name".to_string()));
        assert!(args.contains(&"loopbox-demo-app-postgres-db".to_string()));
        assert!(args.contains(&"127.0.0.61:5432:5432".to_string()));
        assert!(args.contains(&"127.0.0.61:9187:9187".to_string()));
        assert!(args.contains(&"POSTGRES_PASSWORD=loopbox".to_string()));
        assert!(args.contains(&"pgdata:/var/lib/postgresql/data".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("fsync=off"));
    }

    #[test]
    fn existing_container_policy_removes_stopped_and_rejects_running() {
        assert_eq!(
            existing_container_start_action(None),
            ExistingContainerStartAction::Continue
        );
        assert_eq!(
            existing_container_start_action(Some(DockerContainerState {
                running: false,
                exit_code: Some(1),
            })),
            ExistingContainerStartAction::RemoveBeforeStart
        );
        assert_eq!(
            existing_container_start_action(Some(DockerContainerState {
                running: true,
                exit_code: None,
            })),
            ExistingContainerStartAction::AlreadyRunning
        );
    }

    #[test]
    fn docker_stats_parser_extracts_cpu_memory_and_pid_count() {
        let stats = docker_container_stats_from_output(true, "", "12.34%|15.5MiB / 1GiB|7\n")
            .expect("parse stats")
            .expect("stats should be present");

        assert_eq!(stats.cpu_percent, Some(12.34));
        assert_eq!(stats.memory_bytes, Some(16_252_928));
        assert_eq!(stats.process_count, Some(7));
    }

    #[test]
    fn docker_stats_parser_treats_missing_container_as_none() {
        let stats = docker_container_stats_from_output(false, "No such container", "")
            .expect("missing container is not an error");

        assert!(stats.is_none());
    }

    #[test]
    fn inspect_output_parser_distinguishes_missing_and_existing_containers() {
        assert_eq!(
            docker_inspect_state_from_output(true, "", "true|0\n").expect("running state"),
            Some(DockerContainerState {
                running: true,
                exit_code: Some(0),
            })
        );
        assert_eq!(
            docker_inspect_state_from_output(true, "", "false|137\n").expect("stopped state"),
            Some(DockerContainerState {
                running: false,
                exit_code: Some(137),
            })
        );
        assert_eq!(
            docker_inspect_state_from_output(false, "Error: No such object: demo", "")
                .expect("missing state"),
            None
        );
        assert!(
            docker_inspect_state_from_output(false, "permission denied", "")
                .unwrap_err()
                .contains("docker inspect failed")
        );
    }

    #[test]
    fn remove_result_parser_ignores_missing_container() {
        assert!(docker_remove_result_from_output(true, "").is_ok());
        assert!(docker_remove_result_from_output(false, "No such container: demo").is_ok());
        assert!(docker_remove_result_from_output(false, "permission denied")
            .unwrap_err()
            .contains("docker rm failed"));
    }

    #[test]
    fn logs_output_parser_combines_stdout_and_stderr_lines() {
        let lines =
            docker_logs_lines_from_output(true, "warn\n", "line-1\nline-2\n").expect("log lines");

        assert_eq!(lines, vec!["line-1", "line-2", "warn"]);
        assert!(docker_logs_lines_from_output(false, "boom", "")
            .unwrap_err()
            .contains("docker logs failed"));
    }
}
