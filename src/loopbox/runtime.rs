use super::{
    config_path, default_health_check_interval_secs, discover_project_commands, merge_service_env,
    project_primary_host, reverse_proxy_url_for_host, service_ports, sync_reverse_proxy_sidecar,
    LoopboxConfig, ProjectConfig, ProxyEndpointProtocol, ServiceConfig, ServicePortConfig,
    ServiceRuntimeKind,
};
use axum::http::{HeaderMap, Request, Version};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod container;
mod health;
mod process;
mod terminal;
mod terminal_session;
mod vite;

use container::*;
use health::*;
use process::*;
pub use terminal::open_terminal_for_service;
use terminal::terminal_env_pairs;
#[allow(unused_imports)]
pub use terminal_session::{
    decode_terminal_protocol_message, encode_terminal_protocol_message,
    send_terminal_client_message, terminal_session_snapshot, TerminalClientMessage, TerminalFrame,
    TerminalKeyAction, TerminalMods, TerminalMouseKind, TerminalServerMessage,
    TERMINAL_BACKEND_VERSION,
};
use vite::*;

const MAX_LOG_LINES: usize = 2_000;
const STARTING_GRACE_PERIOD_SECS: u64 = 2;
const HEALTHCHECK_TIMEOUT_MS: u64 = 250;
const HEALTHCHECK_RETRIES: usize = 2;
const DEPENDENCY_READY_TIMEOUT_SECS: u64 = 20;
const DEPENDENCY_READY_POLL_MS: u64 = 150;
const ATTACH_RECENT_LINES: usize = 120;
const RUNTIME_PID_REGISTRY_FILE: &str = "runtime-pids.json";
const RUNTIME_LOG_META_REGISTRY_FILE: &str = "runtime-log-meta.json";
const RUNTIME_LOGS_DIR: &str = "runtime-logs";
const RUNTIME_INPUTS_DIR: &str = "runtime-inputs";
const RUNTIME_TERMINALS_DIR: &str = "runtime-terminals";
const RUNTIME_PTY_SUBCOMMAND: &str = "__runtime_pty_runner";
const RUNTIME_ATTACH_SUBCOMMAND: &str = "__runtime_attach_bridge";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRuntimeState {
    Stopped,
    Starting,
    Running,
    Unhealthy,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRuntimeSnapshot {
    pub project: String,
    pub service: String,
    pub state: ServiceRuntimeState,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
struct RunningService {
    child: Child,
    stdin: Option<ChildStdin>,
    input_path: Option<PathBuf>,
    terminal_control_path: Option<PathBuf>,
    started_at: SystemTime,
    ports: Vec<ServicePortConfig>,
    host: String,
    bind_ip: String,
    process_group_leader: bool,
}

#[derive(Debug, Default)]
struct RuntimeStore {
    running: HashMap<String, RunningService>,
    history: HashMap<String, ServiceRuntimeSnapshot>,
    log_buffers: HashMap<String, Arc<Mutex<VecDeque<String>>>>,
    health_checks: HashMap<String, CachedHealthCheck>,
}

#[derive(Debug, Clone)]
struct CachedHealthCheck {
    checked_at: SystemTime,
    healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimePidEntry {
    key: String,
    project: String,
    service: String,
    pid: u32,
    #[serde(default)]
    process_group_leader: bool,
    command: String,
    workdir: String,
    #[serde(default)]
    input_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_control_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_backend_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_rows: Option<u16>,
    recorded_at: u64,
}

struct RuntimePidRememberInput<'a> {
    project_name: &'a str,
    service_name: &'a str,
    snapshot: &'a ServiceRuntimeSnapshot,
    process_group_leader: bool,
    command: &'a str,
    workdir: &'a str,
    input_path: Option<&'a Path>,
    terminal_control_path: Option<&'a Path>,
    terminal_cols: Option<u16>,
    terminal_rows: Option<u16>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RuntimePidRegistry {
    entries: Vec<RuntimePidEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeLogMetaEntry {
    key: String,
    project: String,
    service: String,
    file_offset: u64,
    last_seen_ts: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RuntimeLogMetaRegistry {
    entries: Vec<RuntimeLogMetaEntry>,
}

static RUNTIME_STORE: OnceLock<Mutex<RuntimeStore>> = OnceLock::new();
static RUNTIME_PID_REGISTRY_PATH: OnceLock<PathBuf> = OnceLock::new();
static RUNTIME_LOG_META_REGISTRY_PATH: OnceLock<PathBuf> = OnceLock::new();
static RUNTIME_SERVICE_OPS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static RUNTIME_LOG_META_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePortOwner {
    pub pid: u32,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePortConflict {
    pub bind_ip: String,
    pub port: u16,
    pub owner: Option<ServicePortOwner>,
}

#[derive(Debug)]
struct RuntimeServiceOpGuard {
    key: String,
}

impl Drop for RuntimeServiceOpGuard {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = runtime_service_ops().lock() {
            in_flight.remove(&self.key);
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimePtyRunnerArgs {
    workdir: String,
    command: String,
    log_file: PathBuf,
    input_path: PathBuf,
    terminal_control_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct RuntimeAttachBridgeArgs {
    project: String,
    service: String,
    log_file: PathBuf,
    input_path: PathBuf,
}

pub fn run_runtime_subcommand_from_args(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some(RUNTIME_PTY_SUBCOMMAND) => {
            let result = parse_runtime_pty_runner_args(args)
                .and_then(run_runtime_pty_runner)
                .unwrap_or_else(|err| {
                    eprintln!("Loopbox runtime PTY runner error: {err}");
                    1
                });
            Some(result)
        }
        Some(RUNTIME_ATTACH_SUBCOMMAND) => {
            let result = parse_runtime_attach_bridge_args(args)
                .and_then(run_runtime_attach_bridge)
                .unwrap_or_else(|err| {
                    eprintln!("Loopbox runtime attach bridge error: {err}");
                    1
                });
            Some(result)
        }
        _ => None,
    }
}

fn parse_runtime_pty_runner_args(args: &[String]) -> Result<RuntimePtyRunnerArgs, String> {
    let mut workdir = None::<String>;
    let mut command = None::<String>;
    let mut log_file = None::<PathBuf>;
    let mut input_path = None::<PathBuf>;
    let mut terminal_control_path = None::<PathBuf>;

    let mut i = 1_usize;
    while i < args.len() {
        let flag = args[i].as_str();
        let next = args
            .get(i + 1)
            .ok_or_else(|| format!("Missing value for argument '{flag}'."))?;
        match flag {
            "--workdir" => workdir = Some(next.clone()),
            "--command" => command = Some(next.clone()),
            "--log-file" => log_file = Some(PathBuf::from(next)),
            "--input-fifo" => input_path = Some(PathBuf::from(next)),
            "--terminal-socket" => terminal_control_path = Some(PathBuf::from(next)),
            _ => return Err(format!("Unknown runtime PTY runner argument '{flag}'.")),
        }
        i += 2;
    }

    Ok(RuntimePtyRunnerArgs {
        workdir: workdir.ok_or_else(|| "Missing --workdir.".to_string())?,
        command: command.ok_or_else(|| "Missing --command.".to_string())?,
        log_file: log_file.ok_or_else(|| "Missing --log-file.".to_string())?,
        input_path: input_path.ok_or_else(|| "Missing --input-fifo.".to_string())?,
        terminal_control_path,
    })
}

fn parse_runtime_attach_bridge_args(args: &[String]) -> Result<RuntimeAttachBridgeArgs, String> {
    let mut project = None::<String>;
    let mut service = None::<String>;
    let mut log_file = None::<PathBuf>;
    let mut input_path = None::<PathBuf>;

    let mut i = 1_usize;
    while i < args.len() {
        let flag = args[i].as_str();
        let next = args
            .get(i + 1)
            .ok_or_else(|| format!("Missing value for argument '{flag}'."))?;
        match flag {
            "--project" => project = Some(next.clone()),
            "--service" => service = Some(next.clone()),
            "--log-file" => log_file = Some(PathBuf::from(next)),
            "--input-fifo" => input_path = Some(PathBuf::from(next)),
            _ => return Err(format!("Unknown runtime attach argument '{flag}'.")),
        }
        i += 2;
    }

    Ok(RuntimeAttachBridgeArgs {
        project: project.ok_or_else(|| "Missing --project.".to_string())?,
        service: service.ok_or_else(|| "Missing --service.".to_string())?,
        log_file: log_file.ok_or_else(|| "Missing --log-file.".to_string())?,
        input_path: input_path.ok_or_else(|| "Missing --input-fifo.".to_string())?,
    })
}

fn run_runtime_pty_runner(args: RuntimePtyRunnerArgs) -> Result<i32, String> {
    if !args.input_path.exists() {
        return Err(format!(
            "Runtime input fifo '{}' does not exist.",
            args.input_path.display()
        ));
    }

    if let Some(terminal_control_path) = args.terminal_control_path.as_ref() {
        match terminal_session::run_terminal_session(terminal_session::TerminalSessionArgs {
            workdir: args.workdir.clone(),
            command: args.command.clone(),
            log_file: args.log_file.clone(),
            input_path: args.input_path.clone(),
            control_path: terminal_control_path.clone(),
            cols: 80,
            rows: 24,
            cell_width_px: 9,
            cell_height_px: 18,
        }) {
            Ok(exit_code) => return Ok(exit_code),
            Err(err) if !err.service_started => {
                remove_service_terminal_endpoint(terminal_control_path);
                append_runtime_runner_warning(
                    &args.log_file,
                    &format!(
                        "Integrated terminal unavailable; falling back to legacy PTY runner: {}",
                        err.message
                    ),
                );
                eprintln!("Loopbox runtime PTY runner warning: {}", err.message);
            }
            Err(err) => {
                append_runtime_runner_warning(
                    &args.log_file,
                    &format!(
                        "Integrated terminal session stopped unexpectedly: {}",
                        err.message
                    ),
                );
                return Err(err.message);
            }
        }
    }

    crate::platform::runtime::run_pty_child(
        &args.command,
        &args.workdir,
        &args.log_file,
        &args.input_path,
    )
}

fn append_runtime_runner_warning(log_file: &Path, message: &str) {
    if let Some(parent) = log_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
    {
        let _ = writeln!(file, "[loopbox] {message}");
    }
}

fn run_runtime_attach_bridge(args: RuntimeAttachBridgeArgs) -> Result<i32, String> {
    if !args.input_path.exists() {
        return Err(format!(
            "Runtime input fifo '{}' does not exist.",
            args.input_path.display()
        ));
    }

    if let Some(parent) = args.log_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    if !args.log_file.exists() {
        prepare_service_log_file(&args.log_file)?;
    }

    println!(
        "Attached to '{}::{}'. Press Ctrl+] to detach.",
        args.project, args.service
    );
    let mut follow_offset = file_length(&args.log_file).unwrap_or(0);
    match attach_replay_start_offset(&args.log_file, ATTACH_RECENT_LINES) {
        Ok(replay_offset) if replay_offset < follow_offset => {
            match read_service_log_raw_delta(&args.log_file, replay_offset) {
                Ok((bytes, end_offset)) => {
                    follow_offset = end_offset;
                    if !bytes.is_empty() {
                        let mut stdout = std::io::stdout().lock();
                        stdout
                            .write_all(&bytes)
                            .map_err(|err| format!("Failed to write attach output: {err}"))?;
                        stdout
                            .flush()
                            .map_err(|err| format!("Failed to flush attach output: {err}"))?;
                    }
                }
                Err(err) => eprintln!("Loopbox runtime attach log warning: {err}"),
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("Loopbox runtime attach log warning: {err}"),
    }
    let _ = std::io::stdout().flush();

    let stop = Arc::new(AtomicBool::new(false));
    let output_stop = Arc::clone(&stop);
    let log_file = args.log_file.clone();
    let output_thread = thread::spawn(move || {
        crate::platform::runtime::follow_log_output(
            &log_file,
            follow_offset,
            output_stop,
            file_length,
            read_service_log_raw_delta,
        )
    });

    let input_result = crate::platform::runtime::forward_terminal_input_to_fifo(
        &args.input_path,
        Arc::clone(&stop),
    );
    stop.store(true, Ordering::SeqCst);

    match output_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("Loopbox runtime attach output warning: {err}"),
        Err(err) => eprintln!("Loopbox runtime attach output thread panicked: {err:?}"),
    }

    input_result?;
    Ok(0)
}

pub fn cleanup_stale_runtime_processes() -> Result<usize, String> {
    let (alive_entries, removed_entries) = prune_runtime_pid_registry()?;
    let mut store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    let alive_keys: HashSet<String> = alive_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect();

    // If a previous in-memory snapshot still says a service is active but no
    // alive PID entry exists anymore, mark it as stopped.
    for (key, snapshot) in store.history.iter_mut() {
        if alive_keys.contains(key) {
            continue;
        }
        if matches!(
            snapshot.state,
            ServiceRuntimeState::Starting
                | ServiceRuntimeState::Running
                | ServiceRuntimeState::Unhealthy
        ) {
            snapshot.state = ServiceRuntimeState::Stopped;
            snapshot.pid = None;
        }
    }

    // Remove stale running handles when a process no longer exists.
    store.running.retain(|key, running| {
        if alive_keys.contains(key) {
            return true;
        }
        let keep = matches!(running.child.try_wait(), Ok(None));
        if !keep {
            if let Some(path) = &running.input_path {
                remove_service_input_endpoint(path);
            }
            if let Some(path) = &running.terminal_control_path {
                remove_service_terminal_endpoint(path);
            }
        }
        keep
    });

    for entry in alive_entries {
        let elapsed = unix_timestamp_to_system_time(entry.recorded_at)
            .elapsed()
            .unwrap_or_default()
            .as_secs();
        let snapshot = ServiceRuntimeSnapshot {
            project: entry.project,
            service: entry.service,
            state: if elapsed < STARTING_GRACE_PERIOD_SECS {
                ServiceRuntimeState::Starting
            } else {
                ServiceRuntimeState::Running
            },
            pid: Some(entry.pid),
            started_at: Some(entry.recorded_at),
            exit_code: None,
            last_error: None,
        };
        upsert_runtime_history(&mut store, entry.key, snapshot);
    }

    Ok(removed_entries)
}

pub fn start_service(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<ServiceRuntimeSnapshot, String> {
    let _ = sync_reverse_proxy_sidecar(config);

    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let service = project
        .services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| format!("Service '{service_name}' not found in project '{project_name}'."))?
        .clone();
    let key = runtime_key(project_name, service_name);
    let _op = begin_service_operation(&key)?;

    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;

        if let Some(mut existing) = store.running.remove(&key) {
            match existing.child.try_wait() {
                Ok(None) => {
                    store.running.insert(key.clone(), existing);
                    return Err(format!(
                        "Service '{service_name}' in project '{project_name}' is already running."
                    ));
                }
                Ok(Some(status)) => {
                    let state = if status.success() {
                        ServiceRuntimeState::Stopped
                    } else {
                        ServiceRuntimeState::Crashed
                    };
                    upsert_runtime_history(
                        &mut store,
                        key.clone(),
                        ServiceRuntimeSnapshot {
                            project: project_name.to_string(),
                            service: service_name.to_string(),
                            state,
                            pid: None,
                            started_at: Some(unix_timestamp(existing.started_at)),
                            exit_code: status.code(),
                            last_error: None,
                        },
                    );
                    let _ = forget_runtime_pid(&key);
                }
                Err(err) => {
                    upsert_runtime_history(
                        &mut store,
                        key.clone(),
                        ServiceRuntimeSnapshot {
                            project: project_name.to_string(),
                            service: service_name.to_string(),
                            state: ServiceRuntimeState::Crashed,
                            pid: None,
                            started_at: Some(unix_timestamp(existing.started_at)),
                            exit_code: None,
                            last_error: Some(format!("Failed to query process status: {err}")),
                        },
                    );
                    let _ = forget_runtime_pid(&key);
                }
            }
        }
    }

    match alive_runtime_pid_entry(&key) {
        Ok(Some(entry)) => {
            let configured_ports = service_ports(&service);
            if !configured_ports.is_empty() {
                let runtime_targets = reachability_targets(&project.ip);
                let any_port_reachable = configured_ports.iter().any(|entry| {
                    port_reachable_with_targets(
                        entry.port,
                        &runtime_targets,
                        HEALTHCHECK_RETRIES,
                        HEALTHCHECK_TIMEOUT_MS,
                    )
                });
                if any_port_reachable {
                    return Err(format!(
                        "Service '{service_name}' in project '{project_name}' is already running (pid {}).",
                        entry.pid
                    ));
                }
                let _ = forget_runtime_pid(&key);
            } else {
                return Err(format!(
                    "Service '{service_name}' in project '{project_name}' is already running (pid {}).",
                    entry.pid
                ));
            }
        }
        Ok(None) => {}
        Err(err) => eprintln!("Loopbox runtime pid registry warning: {err}"),
    }

    let log_file = service_log_file_path(project_name, service_name);
    prepare_service_log_file(&log_file)?;
    let _ = reset_runtime_log_meta(project_name, service_name);
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        store
            .log_buffers
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
            .lock()
            .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?
            .clear();
    }

    let host = service_host_for(project_name, service_name, &config.global.domain_suffix);
    let configured_ports = service_ports(&service);
    for port_entry in &configured_ports {
        let port = port_entry.port;
        if port_reachable_with_targets(
            port,
            std::slice::from_ref(&project.ip),
            HEALTHCHECK_RETRIES,
            HEALTHCHECK_TIMEOUT_MS,
        ) {
            let owner_detail = describe_port_owner(&project.ip, port)
                .map(|owner| format!(" (pid {}: {})", owner.pid, owner.command))
                .unwrap_or_default();
            return Err(format!(
                "Port {} is already in use on {} before starting '{}'{}. Stop the existing process or change the service port.",
                port, project.ip, service_name, owner_detail
            ));
        }
    }

    if is_container_service(&service) {
        let snapshot = start_container_service(
            project_name,
            &service,
            &project.ip,
            configured_ports.as_slice(),
        )?;
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        upsert_runtime_history(&mut store, key, snapshot.clone());
        return Ok(snapshot);
    }

    let input_path = service_input_fifo_path(project_name, service_name);
    let terminal_control_path = if integrated_terminal_enabled_for_service(&service) {
        Some(service_terminal_socket_path(project_name, service_name))
    } else {
        None
    };
    prepare_service_input_fifo(&input_path)?;
    if let Some(path) = terminal_control_path.as_ref() {
        prepare_service_terminal_socket_path(path)?;
    }

    let mut child = match spawn_service_process(
        config,
        project_name,
        &service,
        Some(&input_path),
        terminal_control_path.as_deref(),
    ) {
        Ok(child) => child,
        Err(err) => {
            remove_service_input_endpoint(&input_path);
            if let Some(path) = terminal_control_path.as_ref() {
                remove_service_terminal_endpoint(path);
            }
            return Err(err);
        }
    };
    let stdin = child.stdin.take();
    let pid = child.id();

    let started_at = SystemTime::now();
    let snapshot = ServiceRuntimeSnapshot {
        project: project_name.to_string(),
        service: service_name.to_string(),
        state: ServiceRuntimeState::Starting,
        pid: Some(pid),
        started_at: Some(unix_timestamp(started_at)),
        exit_code: None,
        last_error: None,
    };

    let mut store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    store.running.insert(
        key.clone(),
        RunningService {
            child,
            stdin,
            input_path: Some(input_path.clone()),
            terminal_control_path: terminal_control_path.clone(),
            started_at,
            ports: configured_ports,
            host,
            bind_ip: project.ip.clone(),
            process_group_leader: crate::platform::runtime::supports_process_groups(),
        },
    );
    upsert_runtime_history(&mut store, key, snapshot.clone());
    if let Err(err) = remember_runtime_pid(RuntimePidRememberInput {
        project_name,
        service_name,
        snapshot: &snapshot,
        process_group_leader: crate::platform::runtime::supports_process_groups(),
        command: &service.command,
        workdir: &service.workdir,
        input_path: Some(&input_path),
        terminal_control_path: terminal_control_path.as_deref(),
        terminal_cols: Some(80),
        terminal_rows: Some(24),
    }) {
        eprintln!("Loopbox runtime pid registry warning: {err}");
    }

    Ok(snapshot)
}

pub fn stop_service(
    project_name: &str,
    service_name: &str,
) -> Result<ServiceRuntimeSnapshot, String> {
    let key = runtime_key(project_name, service_name);
    let _op = begin_service_operation(&key)?;
    let (running_service, previous) = {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;

        let running = store.running.remove(&key);
        let previous = if running.is_none() {
            store.history.get(&key).cloned()
        } else {
            None
        };
        (running, previous)
    };

    if let Some(mut running) = running_service {
        let started_at = unix_timestamp(running.started_at);
        let stop_error =
            terminate_pid_if_alive(running.child.id(), running.process_group_leader).err();
        let exit_code = match running.child.try_wait() {
            Ok(Some(status)) => status.code(),
            Ok(None) => {
                let _ = running.child.kill();
                running.child.wait().ok().and_then(|status| status.code())
            }
            Err(_) => None,
        };

        let snapshot = ServiceRuntimeSnapshot {
            project: project_name.to_string(),
            service: service_name.to_string(),
            state: ServiceRuntimeState::Stopped,
            pid: None,
            started_at: Some(started_at),
            exit_code,
            last_error: stop_error.map(|err| {
                format!(
                    "Runtime PID {} was not fully terminated: {err}",
                    running.child.id()
                )
            }),
        };
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
        if let Some(path) = &running.input_path {
            remove_service_input_endpoint(path);
        }
        if let Some(path) = &running.terminal_control_path {
            remove_service_terminal_endpoint(path);
        }
        let _ = forget_runtime_pid(&key);
        return Ok(snapshot);
    }

    if let Some(snapshot) =
        stop_container_service_if_present(project_name, service_name, previous.as_ref())?
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
        let _ = forget_runtime_pid(&key);
        return Ok(snapshot);
    }

    match alive_runtime_pid_entry(&key) {
        Ok(Some(entry)) => {
            let stop_error = terminate_pid_if_alive(entry.pid, entry.process_group_leader).err();

            let snapshot = ServiceRuntimeSnapshot {
                project: project_name.to_string(),
                service: service_name.to_string(),
                state: ServiceRuntimeState::Stopped,
                pid: None,
                started_at: Some(entry.recorded_at),
                exit_code: None,
                last_error: stop_error.map(|err| {
                    format!(
                        "Detached runtime PID {} was not fully terminated: {err}",
                        entry.pid
                    )
                }),
            };
            let mut store = runtime_store()
                .lock()
                .map_err(|_| "Runtime store lock poisoned.".to_string())?;
            upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
            if let Some(path) = &entry.input_path {
                remove_service_input_endpoint(Path::new(path));
            }
            if let Some(path) = &entry.terminal_control_path {
                remove_service_terminal_endpoint(Path::new(path));
            }
            let _ = forget_runtime_pid(&key);
            return Ok(snapshot);
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("Loopbox runtime pid registry warning: {err}");
        }
    }

    if let Some(previous) = previous {
        return Ok(ServiceRuntimeSnapshot {
            state: ServiceRuntimeState::Stopped,
            pid: None,
            exit_code: previous.exit_code,
            ..previous
        });
    }

    Ok(ServiceRuntimeSnapshot {
        project: project_name.to_string(),
        service: service_name.to_string(),
        state: ServiceRuntimeState::Stopped,
        pid: None,
        started_at: None,
        exit_code: None,
        last_error: None,
    })
}

pub fn restart_service(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<ServiceRuntimeSnapshot, String> {
    let stopped = stop_service(project_name, service_name)?;
    if let Some(err) = stopped.last_error {
        return Err(format!(
            "Failed to restart service '{service_name}' because stop did not fully terminate the prior runtime: {err}"
        ));
    }
    start_service(config, project_name, service_name)
}

pub fn service_port_conflicts(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<Vec<ServicePortConflict>, String> {
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

    let mut conflicts = Vec::new();
    for port_entry in service_ports(service) {
        if port_reachable_with_targets(port_entry.port, std::slice::from_ref(&project.ip), 1, 120) {
            conflicts.push(ServicePortConflict {
                bind_ip: project.ip.clone(),
                port: port_entry.port,
                owner: describe_port_owner(&project.ip, port_entry.port),
            });
        }
    }

    Ok(conflicts)
}

pub fn kill_service_port_owner(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
    port: u16,
    expected_pid: u32,
) -> Result<(), String> {
    if expected_pid == 0 {
        return Err("Refusing to kill invalid pid 0.".to_string());
    }

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
    if !service_ports(service)
        .iter()
        .any(|port_entry| port_entry.port == port)
    {
        return Err(format!(
            "Port {port} is not configured for service '{service_name}' in project '{project_name}'."
        ));
    }

    let targets = std::slice::from_ref(&project.ip);
    if !port_reachable_with_targets(port, targets, 1, 120) {
        return Err(format!(
            "Port {port} on {} is no longer in use.",
            project.ip
        ));
    }

    let Some(owner) = describe_port_owner(&project.ip, port) else {
        return Err(format!(
            "Port {port} on {} is in use, but Loopbox could not identify the owning process.",
            project.ip
        ));
    };
    if owner.pid != expected_pid {
        return Err(format!(
            "Port {port} on {} is now owned by pid {}: {}, expected pid {expected_pid}.",
            project.ip, owner.pid, owner.command
        ));
    }

    terminate_pid_if_alive(owner.pid, false)?;
    thread::sleep(Duration::from_millis(140));

    if port_reachable_with_targets(port, targets, 1, 120) {
        let owner_detail = describe_port_owner(&project.ip, port)
            .map(|owner| format!(" (pid {}: {})", owner.pid, owner.command))
            .unwrap_or_default();
        return Err(format!(
            "Port {port} on {} is still in use after killing pid {expected_pid}{owner_detail}.",
            project.ip
        ));
    }

    Ok(())
}

pub fn send_service_input(
    project_name: &str,
    service_name: &str,
    input: &str,
) -> Result<(), String> {
    if input.is_empty() {
        return Err("Input cannot be empty.".to_string());
    }

    let key = runtime_key(project_name, service_name);
    let running_endpoint = {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get_mut(&key) {
            match running.child.try_wait() {
                Ok(Some(_)) => {
                    return Err(format!(
                        "Service '{service_name}' in project '{project_name}' has already exited."
                    ))
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(format!(
                        "Failed to inspect runtime status for '{service_name}' in project '{project_name}': {err}"
                    ))
                }
            }
            if let Some(path) = &running.terminal_control_path {
                if path.exists() {
                    return terminal_session::send_terminal_client_message_to_path(
                        path,
                        &TerminalClientMessage::Paste {
                            text: input.to_string(),
                        },
                    )
                    .map(|_| ());
                }
            }
            if let Some(path) = &running.input_path {
                Some(path.clone())
            } else {
                let Some(stdin) = running.stdin.as_mut() else {
                    return Err(format!(
                        "Service '{service_name}' in project '{project_name}' does not accept runtime input."
                    ));
                };
                stdin
                    .write_all(input.as_bytes())
                    .map_err(|err| format!("Failed to send input to '{service_name}': {err}"))?;
                stdin
                    .flush()
                    .map_err(|err| format!("Failed to flush input for '{service_name}': {err}"))?;
                return Ok(());
            }
        } else {
            None
        }
    };

    if let Some(path) = running_endpoint {
        return write_service_input_endpoint(&path, input, service_name);
    }

    if let Some(entry) = alive_runtime_pid_entry(&key)? {
        if let Some(path) = &entry.terminal_control_path {
            let path = Path::new(path);
            if path.exists() {
                return terminal_session::send_terminal_client_message_to_path(
                    path,
                    &TerminalClientMessage::Paste {
                        text: input.to_string(),
                    },
                )
                .map(|_| ());
            }
        }
        if let Some(path) = &entry.input_path {
            return write_service_input_endpoint(Path::new(path), input, service_name);
        }
    }

    Err(format!(
        "Service '{service_name}' in project '{project_name}' is not attached in this session. Restart it from this window to enable key input."
    ))
}

pub fn service_input_attached(project_name: &str, service_name: &str) -> Result<bool, String> {
    let key = runtime_key(project_name, service_name);
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get_mut(&key) {
            return match running.child.try_wait() {
                Ok(None) => Ok(running
                    .terminal_control_path
                    .as_ref()
                    .is_some_and(|path| path.exists())
                    || running.input_path.as_ref().is_some_and(|path| path.exists())
                    || running.stdin.is_some()),
                Ok(Some(_)) => Ok(false),
                Err(err) => Err(format!(
                    "Failed to inspect runtime status for '{service_name}' in project '{project_name}': {err}"
                )),
            };
        }
    }

    match alive_runtime_pid_entry(&key)? {
        Some(entry) => Ok(entry
            .terminal_control_path
            .as_ref()
            .is_some_and(|path| Path::new(path).exists())
            || entry
                .input_path
                .as_ref()
                .is_some_and(|path| Path::new(path).exists())),
        None => Ok(false),
    }
}

pub fn service_terminal_attached(project_name: &str, service_name: &str) -> Result<bool, String> {
    let key = runtime_key(project_name, service_name);
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get_mut(&key) {
            return match running.child.try_wait() {
                Ok(None) => Ok(running
                    .terminal_control_path
                    .as_ref()
                    .is_some_and(|path| path.exists())),
                Ok(Some(_)) => Ok(false),
                Err(err) => Err(format!(
                    "Failed to inspect runtime status for '{service_name}' in project '{project_name}': {err}"
                )),
            };
        }
    }

    match alive_runtime_pid_entry(&key)? {
        Some(entry) => Ok(entry
            .terminal_control_path
            .as_ref()
            .is_some_and(|path| Path::new(path).exists())),
        None => Ok(false),
    }
}

pub fn open_terminal_attach_for_service(
    project_name: &str,
    service_name: &str,
) -> Result<String, String> {
    let key = runtime_key(project_name, service_name);
    let input_path = resolve_runtime_input_path_for_key(&key)?.ok_or_else(|| {
        format!(
            "Service '{service_name}' in project '{project_name}' is not attached in this session."
        )
    })?;
    if !input_path.exists() {
        return Err(format!(
            "Service '{service_name}' in project '{project_name}' does not have a live input endpoint."
        ));
    }

    let log_file = service_log_file_path(project_name, service_name);
    if !log_file.exists() {
        prepare_service_log_file(&log_file)?;
    }

    terminal::open_terminal_attach_for_service(project_name, service_name, &log_file, &input_path)
}

pub fn start_project_all(
    config: &LoopboxConfig,
    project_name: &str,
) -> Result<Vec<ServiceRuntimeSnapshot>, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let ordered_services = service_start_order(project.services.as_slice())?;
    let service_map: HashMap<String, ServiceConfig> = project
        .services
        .iter()
        .map(|service| (service.name.clone(), service.clone()))
        .collect();

    let mut started = Vec::new();
    let mut errors = Vec::new();
    for service_name in ordered_services {
        let Some(service_cfg) = service_map.get(&service_name) else {
            errors.push(format!(
                "Service '{service_name}' disappeared while resolving start order."
            ));
            continue;
        };

        let mut dependency_failed = false;
        for dependency in &service_cfg.depends_on {
            if let Err(err) = wait_for_service_readiness(config, project_name, dependency) {
                errors.push(format!(
                    "Service '{}' waiting for dependency '{}' failed: {err}",
                    service_name, dependency
                ));
                dependency_failed = true;
            }
        }
        if dependency_failed {
            continue;
        }

        match start_service(config, project_name, &service_name) {
            Ok(snapshot) => started.push(snapshot),
            Err(err) => errors.push(err),
        }

        if let Err(err) = wait_for_service_readiness(config, project_name, &service_name) {
            errors.push(format!(
                "Service '{}' did not become ready in time: {err}",
                service_name
            ));
        }
    }

    if errors.is_empty() {
        Ok(started)
    } else {
        Err(errors.join(" | "))
    }
}

pub fn stop_project_all(
    config: &LoopboxConfig,
    project_name: &str,
) -> Result<Vec<ServiceRuntimeSnapshot>, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let services: Vec<(String, Vec<u16>)> = project
        .services
        .iter()
        .map(|service| {
            let ports = service_ports(service)
                .iter()
                .map(|entry| entry.port)
                .collect::<Vec<_>>();
            (service.name.clone(), ports)
        })
        .collect();

    let mut stopped = Vec::new();
    let mut errors = Vec::new();
    for (service_name, service_ports) in services {
        match stop_service(project_name, &service_name) {
            Ok(mut snapshot) => {
                for port in &service_ports {
                    if let Some(err) =
                        force_release_service_port(&project.ip, *port, project_name, &service_name)
                    {
                        if snapshot.last_error.is_none() {
                            snapshot.last_error = Some(err.clone());
                        }
                        errors.push(err);
                    }
                }
                stopped.push(snapshot);
            }
            Err(err) => {
                for port in &service_ports {
                    if let Some(port_err) =
                        force_release_service_port(&project.ip, *port, project_name, &service_name)
                    {
                        errors.push(port_err);
                    }
                }
                errors.push(format!(
                    "Failed to stop '{service_name}' in project '{project_name}': {err}"
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(stopped)
    } else {
        Err(errors.join(" | "))
    }
}

fn service_start_order(services: &[ServiceConfig]) -> Result<Vec<String>, String> {
    let mut service_names = HashSet::new();
    for service in services {
        service_names.insert(service.name.clone());
    }

    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for service in services {
        indegree.insert(service.name.clone(), 0);
        edges.insert(service.name.clone(), Vec::new());
    }

    for service in services {
        for dependency in &service.depends_on {
            if !service_names.contains(dependency) {
                return Err(format!(
                    "Service '{}' depends on unknown service '{}'.",
                    service.name, dependency
                ));
            }
            if let Some(dependents) = edges.get_mut(dependency) {
                dependents.push(service.name.clone());
            }
            if let Some(in_count) = indegree.get_mut(&service.name) {
                *in_count += 1;
            }
        }
    }

    let mut queue = VecDeque::new();
    for service in services {
        if indegree.get(&service.name).copied().unwrap_or(0) == 0 {
            queue.push_back(service.name.clone());
        }
    }

    let mut order = Vec::new();
    while let Some(service_name) = queue.pop_front() {
        order.push(service_name.clone());
        let dependents = edges.get(&service_name).cloned().unwrap_or_default();
        for dependent in dependents {
            if let Some(entry) = indegree.get_mut(&dependent) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    queue.push_back(dependent);
                }
            }
        }
    }

    if order.len() != services.len() {
        return Err(
            "Dependency cycle detected in services. Remove circular 'depends_on' entries."
                .to_string(),
        );
    }

    Ok(order)
}

fn wait_for_service_readiness(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<(), String> {
    let deadline = SystemTime::now()
        .checked_add(Duration::from_secs(DEPENDENCY_READY_TIMEOUT_SECS))
        .unwrap_or(SystemTime::now());

    loop {
        let snapshot = service_runtime_status(config, project_name, service_name)?;
        match snapshot.state {
            ServiceRuntimeState::Running => return Ok(()),
            ServiceRuntimeState::Starting => {}
            ServiceRuntimeState::Unhealthy => {
                return Err(format!("state is unhealthy (pid {:?})", snapshot.pid));
            }
            ServiceRuntimeState::Crashed => {
                return Err(format!(
                    "state is crashed (exit {:?}, error {:?})",
                    snapshot.exit_code, snapshot.last_error
                ));
            }
            ServiceRuntimeState::Stopped => {
                return Err("state is stopped".to_string());
            }
        }

        if SystemTime::now() >= deadline {
            return Err(format!(
                "timed out after {}s",
                DEPENDENCY_READY_TIMEOUT_SECS
            ));
        }
        thread::sleep(Duration::from_millis(DEPENDENCY_READY_POLL_MS));
    }
}

pub fn service_runtime_status(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<ServiceRuntimeSnapshot, String> {
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
    let key = runtime_key(project_name, service_name);
    if is_container_service(service) {
        let host = service_host_for(project_name, service_name, &config.global.domain_suffix);
        return container_runtime_status(
            config,
            project,
            project_name,
            service,
            &project.ip,
            &host,
            &key,
        );
    }

    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;

        if let Some(mut running) = store.running.remove(&key) {
            match running.child.try_wait() {
                Ok(None) => {
                    let elapsed = running.started_at.elapsed().unwrap_or_default().as_secs();
                    let mut state = if elapsed < STARTING_GRACE_PERIOD_SECS {
                        ServiceRuntimeState::Starting
                    } else {
                        ServiceRuntimeState::Running
                    };

                    if elapsed >= STARTING_GRACE_PERIOD_SECS {
                        let runtime_targets = reachability_targets(&running.bind_ip);
                        let effective_ports = service_ports(service);
                        let health_ports = if effective_ports.is_empty() {
                            &running.ports
                        } else {
                            &effective_ports
                        };
                        if !service_ports_healthy(
                            ServiceHealthCheckContext {
                                config,
                                project,
                                project_name,
                                service_name,
                                targets: &runtime_targets,
                                host: &running.host,
                            },
                            health_ports,
                            &mut store.health_checks,
                        ) {
                            state = ServiceRuntimeState::Unhealthy;
                        }
                    }

                    let snapshot = ServiceRuntimeSnapshot {
                        project: project_name.to_string(),
                        service: service_name.to_string(),
                        state,
                        pid: Some(running.child.id()),
                        started_at: Some(unix_timestamp(running.started_at)),
                        exit_code: None,
                        last_error: None,
                    };
                    upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
                    store.running.insert(key.clone(), running);
                    return Ok(snapshot);
                }
                Ok(Some(status)) => {
                    let snapshot = ServiceRuntimeSnapshot {
                        project: project_name.to_string(),
                        service: service_name.to_string(),
                        state: if status.success() {
                            ServiceRuntimeState::Stopped
                        } else {
                            ServiceRuntimeState::Crashed
                        },
                        pid: None,
                        started_at: Some(unix_timestamp(running.started_at)),
                        exit_code: status.code(),
                        last_error: None,
                    };
                    upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
                    let _ = forget_runtime_pid(&runtime_key(project_name, service_name));
                    return Ok(snapshot);
                }
                Err(err) => {
                    let snapshot = ServiceRuntimeSnapshot {
                        project: project_name.to_string(),
                        service: service_name.to_string(),
                        state: ServiceRuntimeState::Crashed,
                        pid: None,
                        started_at: Some(unix_timestamp(running.started_at)),
                        exit_code: None,
                        last_error: Some(format!("Failed to query process status: {err}")),
                    };
                    upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
                    let _ = forget_runtime_pid(&runtime_key(project_name, service_name));
                    return Ok(snapshot);
                }
            }
        }
    }

    match alive_runtime_pid_entry(&key) {
        Ok(Some(entry)) => {
            let started_at = unix_timestamp_to_system_time(entry.recorded_at);
            let elapsed = started_at.elapsed().unwrap_or_default().as_secs();
            let host = service_host_for(project_name, service_name, &config.global.domain_suffix);
            let runtime_targets = reachability_targets(&project.ip);
            let mut state = if elapsed < STARTING_GRACE_PERIOD_SECS {
                ServiceRuntimeState::Starting
            } else {
                ServiceRuntimeState::Running
            };

            if elapsed >= STARTING_GRACE_PERIOD_SECS {
                let effective_ports = service_ports(service);
                let mut store = runtime_store()
                    .lock()
                    .map_err(|_| "Runtime store lock poisoned.".to_string())?;
                if !service_ports_healthy(
                    ServiceHealthCheckContext {
                        config,
                        project,
                        project_name,
                        service_name,
                        targets: &runtime_targets,
                        host: &host,
                    },
                    &effective_ports,
                    &mut store.health_checks,
                ) {
                    state = ServiceRuntimeState::Unhealthy;
                }
            }

            let snapshot = ServiceRuntimeSnapshot {
                project: project_name.to_string(),
                service: service_name.to_string(),
                state,
                pid: Some(entry.pid),
                started_at: Some(entry.recorded_at),
                exit_code: None,
                last_error: None,
            };
            let mut store = runtime_store()
                .lock()
                .map_err(|_| "Runtime store lock poisoned.".to_string())?;
            upsert_runtime_history(&mut store, key.clone(), snapshot.clone());
            return Ok(snapshot);
        }
        Ok(None) => {}
        Err(err) => eprintln!("Loopbox runtime pid registry warning: {err}"),
    }

    let mut store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    if let Some(previous) = store.history.get(&key).cloned() {
        if matches!(
            previous.state,
            ServiceRuntimeState::Starting
                | ServiceRuntimeState::Running
                | ServiceRuntimeState::Unhealthy
        ) {
            let snapshot = ServiceRuntimeSnapshot {
                state: ServiceRuntimeState::Stopped,
                pid: None,
                ..previous
            };
            upsert_runtime_history(&mut store, key, snapshot.clone());
            return Ok(snapshot);
        }
        return Ok(previous);
    }

    Ok(ServiceRuntimeSnapshot {
        project: project_name.to_string(),
        service: service_name.to_string(),
        state: ServiceRuntimeState::Stopped,
        pid: None,
        started_at: None,
        exit_code: None,
        last_error: None,
    })
}

pub fn service_logs_tail(
    project_name: &str,
    service_name: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let effective_limit = limit.clamp(1, MAX_LOG_LINES);
    let log_path = service_log_file_path(project_name, service_name);
    if log_path.exists() {
        return read_service_log_tail(&log_path, effective_limit);
    }
    match docker_logs_tail_for_service(project_name, service_name, effective_limit) {
        Ok(Some(lines)) => return Ok(lines),
        Ok(None) => {}
        Err(err) => eprintln!("Loopbox runtime docker logs warning: {err}"),
    }

    let key = runtime_key(project_name, service_name);
    let store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    let Some(buffer) = store.log_buffers.get(&key) else {
        return Ok(Vec::new());
    };
    let guard = buffer
        .lock()
        .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?;
    let skip = guard.len().saturating_sub(effective_limit);
    Ok(guard.iter().skip(skip).cloned().collect())
}

pub fn service_logs(project_name: &str, service_name: &str) -> Result<Vec<String>, String> {
    let log_path = service_log_file_path(project_name, service_name);
    if !log_path.exists() {
        match docker_logs_tail_for_service(project_name, service_name, MAX_LOG_LINES) {
            Ok(Some(lines)) => return Ok(lines),
            Ok(None) => {}
            Err(err) => eprintln!("Loopbox runtime docker logs warning: {err}"),
        }
    }
    let key = runtime_key(project_name, service_name);
    let buffer = {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        store
            .log_buffers
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
            .clone()
    };

    if !log_path.exists() {
        let logs = buffer
            .lock()
            .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?
            .iter()
            .cloned()
            .collect();
        return Ok(logs);
    }

    let current_len = file_length(&log_path)?;
    let mut meta = match runtime_log_meta_entry_for_key(&key) {
        Ok(Some(meta)) => meta,
        Ok(None) => RuntimeLogMetaEntry {
            key: key.clone(),
            project: project_name.to_string(),
            service: service_name.to_string(),
            file_offset: 0,
            last_seen_ts: unix_timestamp(SystemTime::now()),
        },
        Err(err) => {
            eprintln!("Loopbox runtime log meta warning: {err}");
            RuntimeLogMetaEntry {
                key: key.clone(),
                project: project_name.to_string(),
                service: service_name.to_string(),
                file_offset: 0,
                last_seen_ts: unix_timestamp(SystemTime::now()),
            }
        }
    };

    let mut reset_buffer = false;
    let buffer_is_empty = buffer
        .lock()
        .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?
        .is_empty();

    let incoming_lines = if current_len == 0 {
        meta.file_offset = 0;
        Vec::new()
    } else if buffer_is_empty && meta.file_offset == current_len {
        // On first view after restart, seed the UI with tail context even if
        // offset is already at EOF from the previous session.
        reset_buffer = true;
        read_service_log_tail(&log_path, MAX_LOG_LINES)?
    } else if meta.file_offset == 0 {
        reset_buffer = true;
        meta.file_offset = current_len;
        read_service_log_tail(&log_path, MAX_LOG_LINES)?
    } else if meta.file_offset > current_len {
        // File was truncated/rotated.
        reset_buffer = true;
        meta.file_offset = current_len;
        read_service_log_tail(&log_path, MAX_LOG_LINES)?
    } else if meta.file_offset < current_len {
        let (delta, end_offset) =
            read_service_log_delta(&log_path, meta.file_offset, MAX_LOG_LINES)?;
        meta.file_offset = end_offset;
        delta
    } else {
        Vec::new()
    };

    meta.last_seen_ts = unix_timestamp(SystemTime::now());
    if let Err(err) = upsert_runtime_log_meta(meta) {
        eprintln!("Loopbox runtime log meta warning: {err}");
    }

    let mut guard = buffer
        .lock()
        .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?;
    if reset_buffer {
        guard.clear();
    }
    for line in incoming_lines {
        guard.push_back(line);
        while guard.len() > MAX_LOG_LINES {
            guard.pop_front();
        }
    }
    let logs = guard.iter().cloned().collect();
    Ok(logs)
}

pub fn clear_service_logs(project_name: &str, service_name: &str) -> Result<(), String> {
    let key = runtime_key(project_name, service_name);
    let store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    if let Some(buffer) = store.log_buffers.get(&key) {
        buffer
            .lock()
            .map_err(|_| "Runtime log buffer lock poisoned.".to_string())?
            .clear();
    }
    drop(store);
    let log_path = service_log_file_path(project_name, service_name);
    prepare_service_log_file(&log_path)?;
    let _ = reset_runtime_log_meta(project_name, service_name);
    Ok(())
}

pub fn service_log_attached(project_name: &str, service_name: &str) -> Result<bool, String> {
    let key = runtime_key(project_name, service_name);
    {
        let store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get(&key) {
            if pid_exists(running.child.id()) {
                return Ok(true);
            }
        }
    }
    if docker_container_state(&runtime_container_name(project_name, service_name))
        .ok()
        .flatten()
        .is_some()
    {
        return Ok(true);
    }
    Ok(alive_runtime_pid_entry(&key)?.is_some())
}

fn runtime_store() -> &'static Mutex<RuntimeStore> {
    RUNTIME_STORE.get_or_init(|| Mutex::new(RuntimeStore::default()))
}

fn runtime_service_ops() -> &'static Mutex<HashSet<String>> {
    RUNTIME_SERVICE_OPS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn runtime_log_meta_io_lock() -> &'static Mutex<()> {
    RUNTIME_LOG_META_IO_LOCK.get_or_init(|| Mutex::new(()))
}

fn upsert_runtime_history(store: &mut RuntimeStore, key: String, snapshot: ServiceRuntimeSnapshot) {
    let previous = store.history.get(&key).cloned();
    if let Err(err) =
        crate::loopbox::record_runtime_incident_transition(previous.as_ref(), &snapshot)
    {
        eprintln!("Loopbox incident timeline warning: {err}");
    }
    store.history.insert(key, snapshot);
}

fn begin_service_operation(key: &str) -> Result<RuntimeServiceOpGuard, String> {
    let mut in_flight = runtime_service_ops()
        .lock()
        .map_err(|_| "Runtime operation lock poisoned.".to_string())?;
    if !in_flight.insert(key.to_string()) {
        return Err(
            "Another runtime operation is already in progress for this service.".to_string(),
        );
    }
    Ok(RuntimeServiceOpGuard {
        key: key.to_string(),
    })
}

fn remember_runtime_pid(input: RuntimePidRememberInput<'_>) -> Result<(), String> {
    let Some(pid) = input.snapshot.pid else {
        return Ok(());
    };
    let key = runtime_key(input.project_name, input.service_name);
    let mut registry = load_runtime_pid_registry()?;
    registry.entries.retain(|entry| entry.key != key);
    registry.entries.push(RuntimePidEntry {
        key,
        project: input.project_name.to_string(),
        service: input.service_name.to_string(),
        pid,
        process_group_leader: input.process_group_leader,
        command: input.command.to_string(),
        workdir: input.workdir.to_string(),
        input_path: input
            .input_path
            .map(|path| path.to_string_lossy().to_string()),
        terminal_control_path: input
            .terminal_control_path
            .map(|path| path.to_string_lossy().to_string()),
        terminal_backend_version: input
            .terminal_control_path
            .map(|_| TERMINAL_BACKEND_VERSION.to_string()),
        terminal_cols: input.terminal_cols,
        terminal_rows: input.terminal_rows,
        recorded_at: unix_timestamp(SystemTime::now()),
    });
    save_runtime_pid_registry(&registry)
}

fn forget_runtime_pid(key: &str) -> Result<(), String> {
    let path = runtime_pid_registry_path();
    if !path.exists() {
        return Ok(());
    }

    let mut registry = load_runtime_pid_registry()?;
    let mut removed_input_paths = Vec::new();
    let mut removed_terminal_paths = Vec::new();
    let before = registry.entries.len();
    registry.entries.retain(|entry| {
        let remove = entry.key == key;
        if remove {
            if let Some(path) = &entry.input_path {
                removed_input_paths.push(path.clone());
            }
            if let Some(path) = &entry.terminal_control_path {
                removed_terminal_paths.push(path.clone());
            }
        }
        !remove
    });
    if registry.entries.is_empty() {
        let _ = fs::remove_file(path);
        for input_path in removed_input_paths {
            remove_service_input_endpoint(Path::new(&input_path));
        }
        for terminal_path in removed_terminal_paths {
            remove_service_terminal_endpoint(Path::new(&terminal_path));
        }
        return Ok(());
    }
    if registry.entries.len() != before {
        save_runtime_pid_registry(&registry)?;
    }
    for input_path in removed_input_paths {
        remove_service_input_endpoint(Path::new(&input_path));
    }
    for terminal_path in removed_terminal_paths {
        remove_service_terminal_endpoint(Path::new(&terminal_path));
    }
    Ok(())
}

fn runtime_pid_registry_path() -> PathBuf {
    RUNTIME_PID_REGISTRY_PATH
        .get_or_init(resolve_runtime_pid_registry_path)
        .clone()
}

fn runtime_log_meta_registry_path() -> PathBuf {
    RUNTIME_LOG_META_REGISTRY_PATH
        .get_or_init(resolve_runtime_log_meta_registry_path)
        .clone()
}

fn runtime_logs_dir_path() -> PathBuf {
    let base = runtime_pid_registry_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(RUNTIME_LOGS_DIR)
}

fn runtime_inputs_dir_path() -> PathBuf {
    let base = runtime_pid_registry_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(RUNTIME_INPUTS_DIR)
}

fn runtime_terminals_dir_path() -> PathBuf {
    #[cfg(unix)]
    {
        let uid = rustix::process::getuid().as_raw();
        PathBuf::from("/tmp").join(format!("loopbox-{uid}-{RUNTIME_TERMINALS_DIR}"))
    }

    #[cfg(not(unix))]
    {
        let base = runtime_pid_registry_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(RUNTIME_TERMINALS_DIR)
    }
}

fn service_log_file_path(project_name: &str, service_name: &str) -> PathBuf {
    runtime_logs_dir_path().join(format!(
        "{}__{}.log",
        sanitize_runtime_name(project_name),
        sanitize_runtime_name(service_name)
    ))
}

fn service_input_fifo_path(project_name: &str, service_name: &str) -> PathBuf {
    runtime_inputs_dir_path().join(format!(
        "{}__{}.fifo",
        sanitize_runtime_name(project_name),
        sanitize_runtime_name(service_name)
    ))
}

fn service_terminal_socket_path(project_name: &str, service_name: &str) -> PathBuf {
    let key = format!(
        "{}::{project_name}::{service_name}",
        runtime_pid_registry_path().display()
    );
    runtime_terminals_dir_path().join(format!("{:016x}.sock", stable_runtime_hash(&key)))
}

fn resolve_runtime_pid_registry_path() -> PathBuf {
    let config = config_path();
    let preferred_base_dir = config
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    if directory_writable(&preferred_base_dir) {
        return preferred_base_dir.join(RUNTIME_PID_REGISTRY_FILE);
    }

    let fallback_base_dir = std::env::temp_dir().join("loopbox");
    if directory_writable(&fallback_base_dir) {
        return fallback_base_dir.join(RUNTIME_PID_REGISTRY_FILE);
    }

    PathBuf::from(".").join(RUNTIME_PID_REGISTRY_FILE)
}

fn resolve_runtime_log_meta_registry_path() -> PathBuf {
    let base = runtime_pid_registry_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(RUNTIME_LOG_META_REGISTRY_FILE)
}

fn directory_writable(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }

    let probe = path.join(".loopbox-write-probe");
    match fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn sanitize_runtime_name(input: &str) -> String {
    let mut value = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            value.push(c.to_ascii_lowercase());
        } else {
            value.push('_');
        }
    }
    if value.trim_matches('_').is_empty() {
        "service".to_string()
    } else {
        value
    }
}

fn stable_runtime_hash(input: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn prepare_service_log_file(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    fs::write(path, "").map_err(|err| format!("Failed to initialize {}: {err}", path.display()))
}

fn prepare_service_input_fifo(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    if path.exists() {
        remove_service_input_endpoint(path);
    }
    let output = Command::new("mkfifo").arg(path).output().map_err(|err| {
        format!(
            "Failed to create service input fifo '{}': {err}",
            path.display()
        )
    })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            Err(format!(
                "Failed to create service input fifo '{}': {stderr}",
                path.display()
            ))
        } else if !stdout.is_empty() {
            Err(format!(
                "Failed to create service input fifo '{}': {stdout}",
                path.display()
            ))
        } else {
            Err(format!(
                "Failed to create service input fifo '{}'. Exit: {}",
                path.display(),
                output.status
            ))
        }
    }
}

fn remove_service_input_endpoint(path: &Path) {
    let _ = fs::remove_file(path);
}

fn prepare_service_terminal_socket_path(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    if path.exists() {
        remove_service_terminal_endpoint(path);
    }
    Ok(())
}

fn remove_service_terminal_endpoint(path: &Path) {
    let _ = fs::remove_file(path);
}

fn build_pty_wrapped_launch_command(
    launch_command: &str,
    log_file: &Path,
    input_path: &Path,
) -> String {
    let fifo = shell_quote(input_path.to_string_lossy().as_ref());
    let log = shell_quote(log_file.to_string_lossy().as_ref());
    let inner = shell_quote(launch_command);
    format!("exec 3<> {fifo}; exec /bin/bash -lc {inner} <&3 >> {log} 2>&1")
}

#[allow(clippy::too_many_arguments)]
fn spawn_service_process_via_native_pty_runner(
    config: &LoopboxConfig,
    project_name: &str,
    services: &[ServiceConfig],
    service: &ServiceConfig,
    launch_command: &str,
    log_file: &Path,
    input_path: &Path,
    terminal_control_path: Option<&Path>,
    merged_env_values: &BTreeMap<String, String>,
    vite_allowed_hosts: Option<&str>,
) -> Result<Child, String> {
    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve Loopbox executable path: {err}"))?;
    let mut command = Command::new(current_exe);
    command
        .arg(RUNTIME_PTY_SUBCOMMAND)
        .arg("--workdir")
        .arg(&service.workdir)
        .arg("--command")
        .arg(launch_command)
        .arg("--log-file")
        .arg(log_file)
        .arg("--input-fifo")
        .arg(input_path)
        .current_dir(&service.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = terminal_control_path {
        command.arg("--terminal-socket").arg(path);
    }

    crate::platform::runtime::configure_process_group(&mut command);

    inject_loopbox_env(&mut command, config, project_name, services);
    for (key, value) in merged_env_values {
        if key == "__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS" {
            continue;
        }
        command.env(key, value);
    }
    if let Some(hosts) = vite_allowed_hosts {
        command.env("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS", hosts);
    }

    command.spawn().map_err(|err| {
        format!(
            "Failed to start PTY runner for service '{}' in '{}': {err}",
            service.name, service.workdir
        )
    })
}

fn write_service_input_endpoint(
    path: &Path,
    input: &str,
    service_name: &str,
) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| {
            format!(
                "Failed to open runtime input for '{service_name}' at '{}': {err}",
                path.display()
            )
        })?;
    file.write_all(input.as_bytes())
        .map_err(|err| format!("Failed to send input to '{service_name}': {err}"))?;
    file.flush()
        .map_err(|err| format!("Failed to flush input for '{service_name}': {err}"))?;
    Ok(())
}

fn read_service_log_tail(path: &Path, max_lines: usize) -> Result<Vec<String>, String> {
    let file =
        fs::File::open(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = VecDeque::with_capacity(max_lines);
    for line in reader.lines() {
        let line = line.map_err(|err| format!("Failed to parse {}: {err}", path.display()))?;
        lines.push_back(line);
        while lines.len() > max_lines {
            lines.pop_front();
        }
    }
    Ok(lines.into_iter().collect())
}

fn attach_replay_start_offset(path: &Path, max_lines: usize) -> Result<u64, String> {
    if max_lines == 0 {
        return file_length(path);
    }

    let file =
        fs::File::open(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line_starts = VecDeque::with_capacity(max_lines);
    let mut buffer = Vec::new();
    let mut offset = 0_u64;

    loop {
        buffer.clear();
        let line_start = offset;
        let bytes_read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|err| format!("Failed to parse {}: {err}", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        line_starts.push_back(line_start);
        while line_starts.len() > max_lines {
            line_starts.pop_front();
        }
        offset += bytes_read as u64;
    }

    Ok(line_starts.front().copied().unwrap_or(0))
}

fn read_service_log_raw_delta(path: &Path, start_offset: u64) -> Result<(Vec<u8>, u64), String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|err| format!("Failed to read {} metadata: {err}", path.display()))?
        .len();
    let bounded_offset = start_offset.min(len);
    file.seek(SeekFrom::Start(bounded_offset))
        .map_err(|err| format!("Failed to seek {}: {err}", path.display()))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|err| format!("Failed to parse {}: {err}", path.display()))?;
    let end_offset = bounded_offset + bytes.len() as u64;
    Ok((bytes, end_offset))
}

fn read_service_log_delta(
    path: &Path,
    start_offset: u64,
    max_lines: usize,
) -> Result<(Vec<String>, u64), String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|err| format!("Failed to read {} metadata: {err}", path.display()))?
        .len();
    let bounded_offset = start_offset.min(len);
    file.seek(SeekFrom::Start(bounded_offset))
        .map_err(|err| format!("Failed to seek {}: {err}", path.display()))?;

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut lines = VecDeque::with_capacity(max_lines);
    loop {
        line.clear();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|err| format!("Failed to parse {}: {err}", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        lines.push_back(line.clone());
        while lines.len() > max_lines {
            lines.pop_front();
        }
    }

    let end_offset = reader
        .stream_position()
        .map_err(|err| format!("Failed to read {} position: {err}", path.display()))?;
    Ok((lines.into_iter().collect(), end_offset))
}

fn file_length(path: &Path) -> Result<u64, String> {
    let len = fs::metadata(path)
        .map_err(|err| format!("Failed to read {} metadata: {err}", path.display()))?
        .len();
    Ok(len)
}

fn load_runtime_pid_registry() -> Result<RuntimePidRegistry, String> {
    let path = runtime_pid_registry_path();
    if !path.exists() {
        return Ok(RuntimePidRegistry::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|err| format!("Failed to parse {}: {err}", path.display()))
}

fn save_runtime_pid_registry(registry: &RuntimePidRegistry) -> Result<(), String> {
    let path = runtime_pid_registry_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let payload = serde_json::to_string_pretty(registry)
        .map_err(|err| format!("Failed to serialize runtime pid registry: {err}"))?;
    fs::write(&path, payload).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn load_runtime_log_meta_registry() -> Result<RuntimeLogMetaRegistry, String> {
    let path = runtime_log_meta_registry_path();
    if !path.exists() {
        return Ok(RuntimeLogMetaRegistry::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    match serde_json::from_str(&content) {
        Ok(registry) => Ok(registry),
        Err(err) => {
            let _ = fs::remove_file(&path);
            eprintln!(
                "Loopbox runtime log meta warning: failed to parse {}: {err}. Resetting file.",
                path.display()
            );
            Ok(RuntimeLogMetaRegistry::default())
        }
    }
}

fn save_runtime_log_meta_registry(registry: &RuntimeLogMetaRegistry) -> Result<(), String> {
    let path = runtime_log_meta_registry_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let payload = serde_json::to_string_pretty(registry)
        .map_err(|err| format!("Failed to serialize runtime log meta registry: {err}"))?;
    let nonce = unix_timestamp(SystemTime::now());
    let tmp_path = path.with_extension(format!("tmp-{nonce}"));
    fs::write(&tmp_path, payload)
        .map_err(|err| format!("Failed to write {}: {err}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path).map_err(|err| {
        format!(
            "Failed to move {} to {}: {err}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn runtime_log_meta_entry_for_key(key: &str) -> Result<Option<RuntimeLogMetaEntry>, String> {
    let _guard = runtime_log_meta_io_lock()
        .lock()
        .map_err(|_| "Runtime log meta lock poisoned.".to_string())?;
    let registry = load_runtime_log_meta_registry()?;
    Ok(registry.entries.into_iter().find(|entry| entry.key == key))
}

fn upsert_runtime_log_meta(entry: RuntimeLogMetaEntry) -> Result<(), String> {
    let _guard = runtime_log_meta_io_lock()
        .lock()
        .map_err(|_| "Runtime log meta lock poisoned.".to_string())?;
    let mut registry = load_runtime_log_meta_registry()?;
    registry
        .entries
        .retain(|candidate| candidate.key != entry.key);
    registry.entries.push(entry);
    save_runtime_log_meta_registry(&registry)
}

fn reset_runtime_log_meta(project_name: &str, service_name: &str) -> Result<(), String> {
    upsert_runtime_log_meta(RuntimeLogMetaEntry {
        key: runtime_key(project_name, service_name),
        project: project_name.to_string(),
        service: service_name.to_string(),
        file_offset: 0,
        last_seen_ts: unix_timestamp(SystemTime::now()),
    })
}

fn prune_runtime_pid_registry() -> Result<(Vec<RuntimePidEntry>, usize), String> {
    let path = runtime_pid_registry_path();
    if !path.exists() {
        return Ok((Vec::new(), 0));
    }

    let registry = load_runtime_pid_registry()?;
    let total_entries = registry.entries.len();
    let mut alive_by_key: HashMap<String, RuntimePidEntry> = HashMap::new();
    let mut removed_entries = 0_usize;

    for entry in registry.entries {
        if entry.pid == 0 {
            if let Some(path) = &entry.input_path {
                remove_service_input_endpoint(Path::new(path));
            }
            if let Some(path) = &entry.terminal_control_path {
                remove_service_terminal_endpoint(Path::new(path));
            }
            removed_entries += 1;
            continue;
        }

        if pid_exists(entry.pid) {
            match alive_by_key.get(&entry.key) {
                Some(current) if current.recorded_at > entry.recorded_at => {}
                _ => {
                    alive_by_key.insert(entry.key.clone(), entry);
                }
            }
        } else {
            if let Some(path) = &entry.input_path {
                remove_service_input_endpoint(Path::new(path));
            }
            if let Some(path) = &entry.terminal_control_path {
                remove_service_terminal_endpoint(Path::new(path));
            }
            removed_entries += 1;
        }
    }
    let mut alive_entries: Vec<RuntimePidEntry> = alive_by_key.into_values().collect();
    alive_entries.sort_by(|left, right| left.key.cmp(&right.key));

    if alive_entries.is_empty() {
        let _ = fs::remove_file(path);
    } else if alive_entries.len() != total_entries {
        save_runtime_pid_registry(&RuntimePidRegistry {
            entries: alive_entries.clone(),
        })?;
    }

    Ok((alive_entries, removed_entries))
}

fn alive_runtime_pid_entry(key: &str) -> Result<Option<RuntimePidEntry>, String> {
    let (alive_entries, _removed_entries) = prune_runtime_pid_registry()?;
    Ok(alive_entries.into_iter().find(|entry| entry.key == key))
}

fn resolve_runtime_input_path_for_key(key: &str) -> Result<Option<PathBuf>, String> {
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get_mut(key) {
            match running.child.try_wait() {
                Ok(None) => {
                    if let Some(path) = &running.input_path {
                        return Ok(Some(path.clone()));
                    }
                }
                Ok(Some(_)) => {}
                Err(err) => {
                    return Err(format!(
                        "Failed to inspect runtime status for '{key}': {err}"
                    ));
                }
            }
        }
    }

    Ok(alive_runtime_pid_entry(key)?.and_then(|entry| entry.input_path.map(PathBuf::from)))
}

fn resolve_runtime_terminal_path_for_key(key: &str) -> Result<Option<PathBuf>, String> {
    {
        let mut store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        if let Some(running) = store.running.get_mut(key) {
            match running.child.try_wait() {
                Ok(None) => {
                    if let Some(path) = &running.terminal_control_path {
                        return Ok(Some(path.clone()));
                    }
                }
                Ok(Some(_)) => {}
                Err(err) => {
                    return Err(format!(
                        "Failed to inspect runtime status for '{key}': {err}"
                    ));
                }
            }
        }
    }

    Ok(alive_runtime_pid_entry(key)?
        .and_then(|entry| entry.terminal_control_path.map(PathBuf::from)))
}

fn spawn_service_process(
    config: &LoopboxConfig,
    project_name: &str,
    service: &ServiceConfig,
    input_path: Option<&Path>,
    terminal_control_path: Option<&Path>,
) -> Result<Child, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let merged_env = merge_service_env(config, project_name, &service.name)?;

    let service_host = service_host_for(project_name, &service.name, &config.global.domain_suffix);
    let vite_allowed_hosts = if command_is_vite_like(service) {
        let existing_allowed = merged_env
            .values
            .iter()
            .find_map(|(key, value)| {
                if key == "__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS" {
                    Some(value.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");
        let mut hosts: Vec<String> = existing_allowed
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        if !hosts.iter().any(|value| value == &service_host) {
            hosts.push(service_host);
        }
        Some(hosts.join(","))
    } else {
        None
    };

    let launch_command = apply_bind_hints_to_command(service, &project.ip);
    let log_file = service_log_file_path(project_name, &service.name);
    if let (Some(input_path), Some(terminal_control_path)) = (input_path, terminal_control_path) {
        return spawn_service_process_via_native_pty_runner(
            config,
            project_name,
            project.services.as_slice(),
            service,
            &launch_command,
            &log_file,
            input_path,
            Some(terminal_control_path),
            &merged_env.values,
            vite_allowed_hosts.as_deref(),
        );
    }

    if let Some(input_path) = input_path {
        if command_requires_terminal_tty(service) {
            return spawn_service_process_via_native_pty_runner(
                config,
                project_name,
                project.services.as_slice(),
                service,
                &launch_command,
                &log_file,
                input_path,
                None,
                &merged_env.values,
                vite_allowed_hosts.as_deref(),
            );
        }
    }

    let launch_command = if let Some(input_path) = input_path {
        build_pty_wrapped_launch_command(&launch_command, &log_file, input_path)
    } else {
        format!(
            "( {} ) >> {} 2>&1",
            launch_command,
            shell_quote(log_file.to_string_lossy().as_ref())
        )
    };
    let mut command = Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg(launch_command)
        .current_dir(&service.workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::platform::runtime::configure_process_group(&mut command);

    inject_loopbox_env(
        &mut command,
        config,
        project_name,
        project.services.as_slice(),
    );
    for (key, value) in merged_env.values {
        if key == "__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS" {
            continue;
        }
        command.env(key, value);
    }
    if let Some(hosts) = vite_allowed_hosts {
        command.env("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS", hosts);
    }

    command.spawn().map_err(|err| {
        format!(
            "Failed to start service '{}' in '{}': {err}",
            service.name, service.workdir
        )
    })
}

fn command_is_vite_like(service: &ServiceConfig) -> bool {
    let raw = service.command.trim();
    if raw.is_empty() {
        return false;
    }

    let lower = raw.to_lowercase();
    if is_direct_vite_command(&lower) {
        return true;
    }

    let Some(invocation) = parse_script_invocation(raw) else {
        return false;
    };
    resolve_vite_script_command(&service.workdir, &invocation).is_some()
}

fn command_requires_terminal_tty(service: &ServiceConfig) -> bool {
    let lower = service.command.to_lowercase();
    lower.contains("expo") || lower.contains("react-native") || lower.contains("metro")
}

fn integrated_terminal_enabled_for_service(service: &ServiceConfig) -> bool {
    if service.runtime != ServiceRuntimeKind::Process {
        return false;
    }
    #[cfg(test)]
    {
        // Unit tests run inside Rust's test harness binary, so spawning
        // current_exe as the persistent helper does not enter Loopbox main().
        command_requires_terminal_tty(service)
    }
    #[cfg(not(test))]
    {
        cfg!(target_os = "macos")
    }
}

fn primary_service_port(service: &ServiceConfig) -> Option<u16> {
    let effective_ports = service_ports(service);
    effective_ports
        .iter()
        .find(|entry| entry.protocol == ProxyEndpointProtocol::Http1)
        .map(|entry| entry.port)
        .or_else(|| effective_ports.first().map(|entry| entry.port))
}

fn inject_loopbox_env(
    command: &mut Command,
    config: &LoopboxConfig,
    project_name: &str,
    services: &[ServiceConfig],
) {
    for (key, value) in terminal_env_pairs(config, project_name, services) {
        command.env(key, value);
    }
}

fn runtime_key(project_name: &str, service_name: &str) -> String {
    format!("{project_name}::{service_name}")
}

fn service_host_for(project_name: &str, service_name: &str, suffix: &str) -> String {
    format!(
        "{}.{}.{}",
        service_name.trim().to_lowercase(),
        project_name.trim().to_lowercase(),
        suffix.trim().trim_start_matches('.').to_lowercase()
    )
}

fn format_http_url(host: &str, port: Option<u16>) -> String {
    match port {
        Some(80) | None => format!("http://{host}"),
        Some(port) => format!("http://{host}:{port}"),
    }
}

fn format_service_url(host: &str, port: Option<u16>, direct_ip: Option<&str>) -> String {
    if port.is_some() {
        if let Some(proxy_url) = reverse_proxy_url_for_host(host) {
            return proxy_url;
        }
        if let (Some(ip), Some(service_port)) = (direct_ip, port) {
            return format_http_url(ip, Some(service_port));
        }
    }
    format_http_url(host, port)
}

fn unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_timestamp_to_system_time(timestamp: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(timestamp))
        .unwrap_or(UNIX_EPOCH)
}

fn force_release_service_port(
    bind_ip: &str,
    port: u16,
    project_name: &str,
    service_name: &str,
) -> Option<String> {
    let targets = reachability_targets(bind_ip);
    if !port_reachable_with_targets(port, &targets, 1, 120) {
        return None;
    }

    let owner_before = describe_port_owner(bind_ip, port);
    let mut terminate_error = None;
    if let Some(owner) = owner_before.as_ref() {
        if let Err(err) = terminate_pid_if_alive(owner.pid, false) {
            terminate_error = Some(err);
        }
    }

    thread::sleep(Duration::from_millis(140));
    if !port_reachable_with_targets(port, &targets, 1, 120) {
        return None;
    }

    let owner_after = describe_port_owner(bind_ip, port);
    let owner_detail = owner_after
        .as_ref()
        .or(owner_before.as_ref())
        .map(|owner| format!(" (pid {}: {})", owner.pid, owner.command))
        .unwrap_or_default();
    let termination_detail = terminate_error
        .map(|err| format!(" Termination attempt failed: {err}"))
        .unwrap_or_default();

    Some(format!(
        "Port {port} on {bind_ip} is still in use after stopping '{service_name}' in project '{project_name}'{owner_detail}.{termination_detail}"
    ))
}

fn describe_port_owner(bind_ip: &str, port: u16) -> Option<ServicePortOwner> {
    let pid = listening_pid_for_port(bind_ip, port)?;
    let command = process_command_for_pid(pid).unwrap_or_else(|| "unknown".to_string());
    Some(ServicePortOwner { pid, command })
}

fn listening_pid_for_port(bind_ip: &str, port: u16) -> Option<u32> {
    crate::platform::runtime::listening_pid_for_port(bind_ip, port)
}

fn process_command_for_pid(pid: u32) -> Option<String> {
    crate::platform::runtime::process_command_for_pid(pid)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests;
