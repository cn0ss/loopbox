use super::*;
use crate::loopbox::{GlobalConfig, ProjectConfig, ProxyEndpointProtocol, ServicePortConfig};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    nanos.to_string()
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !pid_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    !pid_exists(pid)
}

#[cfg(unix)]
fn wait_for_process_group_member(pgid: u32, pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if crate::platform::process::process_group_pids(pgid).contains(&pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    crate::platform::process::process_group_pids(pgid).contains(&pid)
}

fn runtime_config_with_port(
    command: &str,
    service_port: Option<u16>,
) -> (LoopboxConfig, String, String) {
    runtime_config_with_ip_and_port(command, "127.0.0.20", service_port)
}

fn runtime_config_with_ip_and_port(
    command: &str,
    project_ip: &str,
    service_port: Option<u16>,
) -> (LoopboxConfig, String, String) {
    let project = format!("runtime-{}", nonce());
    let service = "backend".to_string();
    let service_cfg = ServiceConfig {
        name: service.clone(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: service_port,
        protocol: ProxyEndpointProtocol::Http1,
        command: command.to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    (
        LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([(
                project.clone(),
                ProjectConfig {
                    dir: "/tmp".to_string(),
                    ip: project_ip.to_string(),
                    services: vec![service_cfg],
                    default_open_service: Some(service.clone()),
                    proxy_traffic_capture_enabled: None,
                    proxy_traffic_capture_mode: None,
                    grpc_proto_paths: vec![],
                    proxy_endpoints: vec![],
                },
            )]),
        },
        project,
        service,
    )
}

fn reserve_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
    listener.local_addr().expect("listener address").port()
}

fn random_test_port() -> u16 {
    let seed = nonce();
    30_000 + (seed.bytes().map(u16::from).sum::<u16>() % 20_000)
}

#[cfg(target_os = "macos")]
fn wait_for_port_owner(bind_ip: &str, port: u16, timeout: Duration) -> Option<u32> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Some(pid) = listening_pid_for_port(bind_ip, port) {
            return Some(pid);
        }
        thread::sleep(Duration::from_millis(50));
    }
    listening_pid_for_port(bind_ip, port)
}

#[cfg(target_os = "macos")]
fn wait_for_ready_file(path: &Path, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    path.exists()
}

fn runtime_config(command: &str) -> (LoopboxConfig, String, String) {
    let seed = nonce();
    let port = 20_000 + (seed.bytes().map(u16::from).sum::<u16>() % 20_000);
    runtime_config_with_port(command, Some(port))
}

fn runtime_config_with_two_services(
    first_command: &str,
    second_command: &str,
) -> (LoopboxConfig, String, String, String) {
    let project = format!("runtime-{}", nonce());
    let first_service = "api".to_string();
    let second_service = "worker".to_string();
    (
        LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([(
                project.clone(),
                ProjectConfig {
                    dir: "/tmp".to_string(),
                    ip: "127.0.0.20".to_string(),
                    services: vec![
                        ServiceConfig {
                            name: first_service.clone(),
                            runtime: crate::loopbox::ServiceRuntimeKind::Process,
                            container: None,
                            ports: vec![],
                            port: None,
                            protocol: ProxyEndpointProtocol::Http1,
                            command: first_command.to_string(),
                            workdir: "/tmp".to_string(),
                            env_files: vec![],
                            depends_on: vec![],
                            autostart: false,
                            health_path: None,
                        },
                        ServiceConfig {
                            name: second_service.clone(),
                            runtime: crate::loopbox::ServiceRuntimeKind::Process,
                            container: None,
                            ports: vec![],
                            port: None,
                            protocol: ProxyEndpointProtocol::Http1,
                            command: second_command.to_string(),
                            workdir: "/tmp".to_string(),
                            env_files: vec![],
                            depends_on: vec![],
                            autostart: false,
                            health_path: None,
                        },
                    ],
                    default_open_service: Some(first_service.clone()),
                    proxy_traffic_capture_enabled: None,
                    proxy_traffic_capture_mode: None,
                    grpc_proto_paths: vec![],
                    proxy_endpoints: vec![],
                },
            )]),
        },
        project,
        first_service,
        second_service,
    )
}

fn drop_runtime_tracking(project: &str, service: &str) {
    let key = runtime_key(project, service);
    let mut store = runtime_store().lock().expect("runtime store lock");
    store.running.remove(&key);
    store.history.remove(&key);
    store.log_buffers.remove(&key);
}

#[cfg(target_os = "macos")]
#[test]
fn service_port_conflicts_reports_current_port_owner() {
    let port = reserve_loopback_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind blocked port");
    let (config, project, service) =
        runtime_config_with_ip_and_port("sleep 1", "127.0.0.1", Some(port));

    let conflicts = service_port_conflicts(&config, &project, &service).expect("port conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].bind_ip, "127.0.0.1");
    assert_eq!(conflicts[0].port, port);
    let owner = conflicts[0].owner.as_ref().expect("port owner");
    assert_eq!(owner.pid, std::process::id());
    assert!(!owner.command.is_empty());

    drop(listener);
}

#[test]
fn service_port_conflicts_ignores_unblocked_ports() {
    let port = random_test_port();
    let (config, project, service) =
        runtime_config_with_ip_and_port("sleep 1", "127.0.0.1", Some(port));

    let conflicts = service_port_conflicts(&config, &project, &service).expect("port conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn kill_service_port_owner_rejects_unconfigured_ports() {
    let port = random_test_port();
    let (config, project, service) =
        runtime_config_with_ip_and_port("sleep 1", "127.0.0.1", Some(port));

    let err = kill_service_port_owner(&config, &project, &service, port + 1, 1)
        .expect_err("unconfigured port must fail");

    assert!(err.contains("not configured"));
}

#[cfg(target_os = "macos")]
#[test]
fn kill_service_port_owner_rejects_changed_owner_pid() {
    let port = reserve_loopback_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind blocked port");
    let (config, project, service) =
        runtime_config_with_ip_and_port("sleep 1", "127.0.0.1", Some(port));

    let err = kill_service_port_owner(&config, &project, &service, port, u32::MAX)
        .expect_err("pid mismatch must fail");

    assert!(err.contains("expected pid"));
    assert!(pid_exists(std::process::id()));

    drop(listener);
}

#[cfg(target_os = "macos")]
#[test]
fn kill_service_port_owner_releases_child_port_blocker() {
    let port = reserve_loopback_port();
    let ready_file = std::env::temp_dir().join(format!("loopbox-port-blocker-{}.ready", nonce()));
    let mut child = Command::new(std::env::current_exe().expect("current test exe"))
        .arg("runtime_port_blocker_child")
        .arg("--ignored")
        .env("LOOPBOX_PORT_BLOCKER_BIND", "127.0.0.1")
        .env("LOOPBOX_PORT_BLOCKER_PORT", port.to_string())
        .env("LOOPBOX_PORT_BLOCKER_READY", &ready_file)
        .spawn()
        .expect("spawn port blocker child");
    assert!(wait_for_ready_file(&ready_file, Duration::from_secs(3)));

    let owner_pid =
        wait_for_port_owner("127.0.0.1", port, Duration::from_secs(3)).expect("port owner");
    assert_eq!(owner_pid, child.id());
    let (config, project, service) =
        runtime_config_with_ip_and_port("sleep 1", "127.0.0.1", Some(port));

    kill_service_port_owner(&config, &project, &service, port, owner_pid).expect("kill port owner");

    assert!(wait_for_pid_exit(owner_pid, Duration::from_secs(3)));
    assert!(wait_for_port_owner("127.0.0.1", port, Duration::from_millis(300)).is_none());

    let _ = child.wait();
    let _ = std::fs::remove_file(&ready_file);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn runtime_port_blocker_child() {
    let Ok(bind_ip) = std::env::var("LOOPBOX_PORT_BLOCKER_BIND") else {
        return;
    };
    let port = std::env::var("LOOPBOX_PORT_BLOCKER_PORT")
        .expect("port env")
        .parse::<u16>()
        .expect("valid port");
    let ready_file = PathBuf::from(std::env::var("LOOPBOX_PORT_BLOCKER_READY").expect("ready env"));
    let _listener = TcpListener::bind((bind_ip.as_str(), port)).expect("bind child listener");
    std::fs::write(&ready_file, b"ready").expect("write ready file");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[test]
fn reachability_targets_adds_localhost_fallback_for_loopback_alias() {
    let targets = reachability_targets("127.0.0.30");
    assert_eq!(targets.first().map(String::as_str), Some("127.0.0.30"));
    assert_eq!(targets.len(), 1);
}

#[test]
fn parse_runtime_pty_runner_args_accepts_expected_flags() {
    let args = vec![
        "__runtime_pty_runner".to_string(),
        "--workdir".to_string(),
        "/tmp".to_string(),
        "--command".to_string(),
        "echo hi".to_string(),
        "--log-file".to_string(),
        "/tmp/loopbox-test.log".to_string(),
        "--input-fifo".to_string(),
        "/tmp/loopbox-test.fifo".to_string(),
    ];

    let parsed = parse_runtime_pty_runner_args(&args).expect("parse args");
    assert_eq!(parsed.workdir, "/tmp");
    assert_eq!(parsed.command, "echo hi");
    assert_eq!(
        parsed.log_file,
        std::path::PathBuf::from("/tmp/loopbox-test.log")
    );
    assert_eq!(
        parsed.input_path,
        std::path::PathBuf::from("/tmp/loopbox-test.fifo")
    );
}

#[test]
fn parse_runtime_attach_bridge_args_accepts_expected_flags() {
    let args = vec![
        "__runtime_attach_bridge".to_string(),
        "--project".to_string(),
        "frame-it".to_string(),
        "--service".to_string(),
        "mobile".to_string(),
        "--log-file".to_string(),
        "/tmp/loopbox-test.log".to_string(),
        "--input-fifo".to_string(),
        "/tmp/loopbox-test.fifo".to_string(),
    ];

    let parsed = parse_runtime_attach_bridge_args(&args).expect("parse args");
    assert_eq!(parsed.project, "frame-it");
    assert_eq!(parsed.service, "mobile");
    assert_eq!(
        parsed.log_file,
        std::path::PathBuf::from("/tmp/loopbox-test.log")
    );
    assert_eq!(
        parsed.input_path,
        std::path::PathBuf::from("/tmp/loopbox-test.fifo")
    );
}

#[test]
fn terminal_protocol_round_trips_snapshot_and_key_messages() {
    let frame = TerminalFrame {
        cols: 80,
        rows: 24,
        title: "demo".to_string(),
        cursor_x: 3,
        cursor_y: 2,
        lines: vec!["hello".to_string(), "world".to_string()],
    };
    let snapshot = TerminalServerMessage::Snapshot(frame.clone());
    let encoded = encode_terminal_protocol_message(&snapshot).expect("encode snapshot");
    let decoded: TerminalServerMessage =
        decode_terminal_protocol_message(&encoded).expect("decode snapshot");
    assert_eq!(decoded, snapshot);

    let key = TerminalClientMessage::Key {
        code: "KeyC".to_string(),
        text: Some("c".to_string()),
        mods: TerminalMods {
            ctrl: true,
            alt: false,
            shift: false,
            meta: false,
        },
        action: TerminalKeyAction::Press,
    };
    let encoded = encode_terminal_protocol_message(&key).expect("encode key");
    let decoded: TerminalClientMessage =
        decode_terminal_protocol_message(&encoded).expect("decode key");
    assert_eq!(decoded, key);
}

#[cfg(unix)]
#[test]
fn pty_runner_falls_back_when_integrated_terminal_cannot_start() {
    let dir = std::env::temp_dir().join(format!("loopbox-terminal-fallback-{}", nonce()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let fifo = dir.join("input.fifo");
    let log_file = dir.join("runtime.log");
    let socket_path_that_cannot_bind = dir.join("terminal.sock");
    std::fs::create_dir_all(&socket_path_that_cannot_bind).expect("create blocking socket dir");

    let mkfifo = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .output()
        .expect("run mkfifo");
    assert!(
        mkfifo.status.success(),
        "mkfifo failed: {}",
        String::from_utf8_lossy(&mkfifo.stderr)
    );

    let exit_code = run_runtime_pty_runner(RuntimePtyRunnerArgs {
        workdir: dir.to_string_lossy().to_string(),
        command: "printf 'fallback terminal ok\\n'".to_string(),
        log_file: log_file.clone(),
        input_path: fifo,
        terminal_control_path: Some(socket_path_that_cannot_bind),
    })
    .expect("runner should fall back to legacy pty");

    assert_eq!(exit_code, 0);
    let log = std::fs::read_to_string(&log_file).expect("read runtime log");
    assert!(log.contains("Integrated terminal unavailable"));
    assert!(log.contains("fallback terminal ok"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_pid_registry_persists_terminal_socket_metadata() {
    let key = runtime_key("term-project", "web");
    let entry = RuntimePidEntry {
        key: key.clone(),
        project: "term-project".to_string(),
        service: "web".to_string(),
        pid: 12345,
        process_group_leader: true,
        command: "npm run dev".to_string(),
        workdir: "/tmp".to_string(),
        input_path: Some("/tmp/loopbox-old.fifo".to_string()),
        terminal_control_path: Some("/tmp/loopbox-term.sock".to_string()),
        terminal_backend_version: Some(TERMINAL_BACKEND_VERSION.to_string()),
        terminal_cols: Some(100),
        terminal_rows: Some(30),
        recorded_at: 1,
    };
    let payload = serde_json::to_string(&entry).expect("serialize entry");
    assert!(payload.contains("terminal_control_path"));
    assert!(payload.contains("terminal_backend_version"));

    let decoded: RuntimePidEntry = serde_json::from_str(&payload).expect("deserialize entry");
    assert_eq!(
        decoded.terminal_control_path.as_deref(),
        Some("/tmp/loopbox-term.sock")
    );
    assert_eq!(decoded.terminal_cols, Some(100));
    assert_eq!(decoded.terminal_rows, Some(30));
}

#[test]
fn legacy_runtime_pid_registry_entries_without_terminal_socket_still_load() {
    let payload = r#"{
        "key": "legacy::web",
        "project": "legacy",
        "service": "web",
        "pid": 12345,
        "process_group_leader": true,
        "command": "npm run dev",
        "workdir": "/tmp",
        "input_path": "/tmp/legacy.fifo",
        "recorded_at": 1
    }"#;

    let decoded: RuntimePidEntry = serde_json::from_str(payload).expect("deserialize legacy entry");
    assert_eq!(decoded.input_path.as_deref(), Some("/tmp/legacy.fifo"));
    assert_eq!(decoded.terminal_control_path, None);
    assert_eq!(decoded.terminal_backend_version, None);
    assert_eq!(decoded.terminal_cols, None);
    assert_eq!(decoded.terminal_rows, None);
}

#[cfg(unix)]
#[test]
fn terminal_socket_paths_fit_macos_unix_socket_limit() {
    let project_name = "very-long-project-name-for-terminal-socket-paths-".repeat(8);
    let service_name = "very-long-service-name-for-terminal-socket-paths-".repeat(8);
    let path = service_terminal_socket_path(&project_name, &service_name);
    let path = path.to_string_lossy();

    assert!(path.starts_with("/tmp/loopbox-"));
    assert!(
        path.len() < 104,
        "macOS sockaddr_un.sun_path is 104 bytes; got {} bytes at {path}",
        path.len()
    );
}

#[test]
fn apply_bind_hints_adds_vite_flags_for_pnpm_dev_script() {
    let nonce = nonce();
    let workdir = std::env::temp_dir().join(format!("loopbox-vite-bind-{nonce}"));
    std::fs::create_dir_all(&workdir).expect("create temp workdir");
    std::fs::write(
        workdir.join("package.json"),
        r#"{
                "name": "frontend",
                "scripts": {
                    "dev": "vite dev"
                }
            }"#,
    )
    .expect("write package json");

    let service = ServiceConfig {
        name: "frontend".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(3000),
        protocol: ProxyEndpointProtocol::Http1,
        command: "pnpm dev".to_string(),
        workdir: workdir.to_string_lossy().to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.starts_with("pnpm exec vite dev"));
    assert!(adjusted.contains("--host 127.0.0.30"));
    assert!(adjusted.contains("--port 3000"));
    assert!(adjusted.contains("--strictPort"));
    assert!(!adjusted.contains(" -- --host"));

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn apply_bind_hints_adds_astro_flags_for_pnpm_dev_script() {
    let nonce = nonce();
    let workdir = std::env::temp_dir().join(format!("loopbox-astro-bind-{nonce}"));
    std::fs::create_dir_all(&workdir).expect("create temp workdir");
    std::fs::write(
        workdir.join("package.json"),
        r#"{
                "name": "frontend",
                "scripts": {
                    "dev": "astro dev"
                }
            }"#,
    )
    .expect("write package json");

    let service = ServiceConfig {
        name: "frontend".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(3000),
        protocol: ProxyEndpointProtocol::Http1,
        command: "pnpm dev".to_string(),
        workdir: workdir.to_string_lossy().to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.starts_with("pnpm exec astro dev"));
    assert!(adjusted.contains("--host 127.0.0.30"));
    assert!(adjusted.contains("--port 3000"));
    assert!(!adjusted.contains("--strictPort"));

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn apply_bind_hints_normalizes_trailing_vite_delimiter() {
    let service = ServiceConfig {
        name: "frontend".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(3000),
        protocol: ProxyEndpointProtocol::Http1,
        command: "vite --".to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.starts_with("vite --host 127.0.0.30"));
    assert!(adjusted.contains("--port 3000"));
    assert!(adjusted.contains("--strictPort"));
    assert!(!adjusted.contains("vite -- --host"));
}

#[test]
fn apply_bind_hints_skips_port_flags_for_portless_vite_service() {
    let service = ServiceConfig {
        name: "frontend".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: None,
        protocol: ProxyEndpointProtocol::Http1,
        command: "vite".to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.contains("--host 127.0.0.30"));
    assert!(!adjusted.contains("--port"));
    assert!(!adjusted.contains("--strictPort"));
}

#[test]
fn apply_bind_hints_adds_port_for_expo_command_without_localhost_mode() {
    let service = ServiceConfig {
        name: "mobile".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(8123),
        protocol: ProxyEndpointProtocol::Http1,
        command: "npx expo start".to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert_eq!(adjusted, "npx expo start --port 8123");
}

#[test]
fn apply_bind_hints_removes_expo_localhost_mode() {
    let nonce = nonce();
    let workdir = std::env::temp_dir().join(format!("loopbox-expo-bind-{nonce}"));
    std::fs::create_dir_all(&workdir).expect("create temp workdir");
    std::fs::write(
        workdir.join("package.json"),
        r#"{
                "name": "mobile",
                "scripts": {
                    "start": "expo start"
                }
            }"#,
    )
    .expect("write package json");

    let service = ServiceConfig {
        name: "mobile".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(8123),
        protocol: ProxyEndpointProtocol::Http1,
        command: "npm run start -- --localhost --port 8081".to_string(),
        workdir: workdir.to_string_lossy().to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert_eq!(adjusted, "npm run start -- --port 8081");
    assert!(!adjusted.contains("--localhost"));

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn primary_service_port_prefers_http1_from_multi_port_config() {
    let service = ServiceConfig {
        name: "gateway".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![
            ServicePortConfig {
                port: 50051,
                protocol: ProxyEndpointProtocol::GrpcH2c,
                health_path: None,
            },
            ServicePortConfig {
                port: 8080,
                protocol: ProxyEndpointProtocol::Http1,
                health_path: Some("/health".to_string()),
            },
        ],
        port: None,
        protocol: ProxyEndpointProtocol::Http1,
        command: "pnpm dev".to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };
    assert_eq!(primary_service_port(&service), Some(8080));
}

#[test]
fn terminal_env_pairs_export_all_service_ports() {
    let project = "multiport".to_string();
    let service = ServiceConfig {
        name: "gateway".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![
            ServicePortConfig {
                port: 50051,
                protocol: ProxyEndpointProtocol::GrpcH2c,
                health_path: None,
            },
            ServicePortConfig {
                port: 8080,
                protocol: ProxyEndpointProtocol::Http1,
                health_path: Some("/health".to_string()),
            },
        ],
        port: None,
        protocol: ProxyEndpointProtocol::Http1,
        command: "pnpm dev".to_string(),
        workdir: "/tmp".to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };
    let config = LoopboxConfig {
        global: GlobalConfig::default(),
        projects: BTreeMap::from([(
            project.clone(),
            ProjectConfig {
                dir: "/tmp".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![service.clone()],
                default_open_service: Some(service.name.clone()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };

    let env_pairs = terminal_env_pairs(&config, &project, &[service]);
    let env_map = env_pairs.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(
        env_map.get("LOOPBOX_PORT_GATEWAY").map(String::as_str),
        Some("8080")
    );
    assert_eq!(
        env_map.get("LOOPBOX_PORTS_GATEWAY").map(String::as_str),
        Some("50051,8080")
    );
}

#[test]
fn parse_script_invocation_supports_pnpm_filter_syntax() {
    let parsed =
        parse_script_invocation("pnpm --filter @hit-app/web dev").expect("parse invocation");
    assert_eq!(parsed.manager, ScriptManager::Pnpm);
    assert_eq!(parsed.script, "dev");
    assert_eq!(parsed.workspace_filter.as_deref(), Some("@hit-app/web"));
}

#[test]
fn apply_bind_hints_adds_vite_flags_for_pnpm_filter_dev_script() {
    let nonce = nonce();
    let root = std::env::temp_dir().join(format!("loopbox-vite-filter-{nonce}"));
    let web_dir = root.join("apps").join("web");
    std::fs::create_dir_all(&web_dir).expect("create monorepo web dir");
    std::fs::write(root.join("package.json"), r#"{"name":"root"}"#).expect("write root package");
    std::fs::write(
        web_dir.join("package.json"),
        r#"{
                "name": "@hit-app/web",
                "scripts": {
                    "dev": "vite"
                }
            }"#,
    )
    .expect("write web package json");

    let service = ServiceConfig {
        name: "web".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(3000),
        protocol: ProxyEndpointProtocol::Http1,
        command: "pnpm --filter @hit-app/web dev".to_string(),
        workdir: root.to_string_lossy().to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.starts_with("pnpm --filter @hit-app/web exec vite --host 127.0.0.30"));
    assert!(adjusted.contains("--port 3000"));
    assert!(adjusted.contains("--strictPort"));
    assert!(!adjusted.contains(" dev -- --host"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn apply_bind_hints_adds_vite_flags_for_nested_root_script_chain() {
    let nonce = nonce();
    let root = std::env::temp_dir().join(format!("loopbox-vite-nested-{nonce}"));
    let web_dir = root.join("apps").join("web");
    std::fs::create_dir_all(&web_dir).expect("create monorepo web dir");
    std::fs::write(
        root.join("package.json"),
        r#"{
                "name":"hit-app",
                "scripts": {
                    "dev:web": "pnpm --filter @hit-app/web dev"
                }
            }"#,
    )
    .expect("write root package");
    std::fs::write(
        web_dir.join("package.json"),
        r#"{
                "name": "@hit-app/web",
                "scripts": {
                    "dev": "vite"
                }
            }"#,
    )
    .expect("write web package json");

    let service = ServiceConfig {
        name: "web".to_string(),
        runtime: crate::loopbox::ServiceRuntimeKind::Process,
        container: None,
        ports: vec![],
        port: Some(3000),
        protocol: ProxyEndpointProtocol::Http1,
        command: "npm run dev:web".to_string(),
        workdir: root.to_string_lossy().to_string(),
        env_files: vec![],
        depends_on: vec![],
        autostart: false,
        health_path: None,
    };

    let adjusted = apply_bind_hints_to_command(&service, "127.0.0.30");
    assert!(adjusted.starts_with("pnpm --filter @hit-app/web exec vite --host 127.0.0.30"));
    assert!(adjusted.contains("--port 3000"));
    assert!(adjusted.contains("--strictPort"));
    assert!(!adjusted.starts_with("npm run dev:web"));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn start_and_stop_portless_service_changes_status() {
    let (config, project, service) = runtime_config_with_port("sleep 3", None);

    let started = start_service(&config, &project, &service).expect("start service");
    assert!(matches!(
        started.state,
        ServiceRuntimeState::Starting | ServiceRuntimeState::Running
    ));

    let status = service_runtime_status(&config, &project, &service).expect("runtime status");
    assert!(matches!(
        status.state,
        ServiceRuntimeState::Starting | ServiceRuntimeState::Running
    ));

    let stopped = stop_service(&project, &service).expect("stop service");
    assert_eq!(stopped.state, ServiceRuntimeState::Stopped);
}

#[cfg(unix)]
#[test]
fn send_service_input_writes_to_running_process_stdin() {
    let (config, project, service) = runtime_config_with_port(
        "while read -r line; do echo INPUT:$line; sleep 5; done",
        None,
    );

    start_service(&config, &project, &service).expect("start service");
    thread::sleep(Duration::from_millis(120));

    send_service_input(&project, &service, "r\n").expect("send input");
    thread::sleep(Duration::from_millis(180));

    let logs = service_logs(&project, &service).expect("service logs");
    assert!(logs.iter().any(|line| line.contains("INPUT:r")));

    let _ = stop_service(&project, &service);
}

#[cfg(unix)]
#[test]
fn service_input_attached_stays_true_after_runtime_tracking_is_lost() {
    let (config, project, service) = runtime_config_with_port("sleep 5", None);

    start_service(&config, &project, &service).expect("start service");
    assert!(service_input_attached(&project, &service).expect("input attached"));

    // Simulate UI/runtime process restart where in-memory child handles are gone.
    drop_runtime_tracking(&project, &service);
    assert!(service_input_attached(&project, &service).expect("input detached"));

    let _ = stop_service(&project, &service);
}

#[cfg(unix)]
#[test]
fn send_service_input_works_after_runtime_tracking_is_lost() {
    let (config, project, service) = runtime_config_with_port(
        "while read -r line; do echo INPUT:$line; sleep 5; done",
        None,
    );

    start_service(&config, &project, &service).expect("start service");
    thread::sleep(Duration::from_millis(120));

    // Simulate a Loopbox UI/runtime restart while the service keeps running.
    drop_runtime_tracking(&project, &service);
    assert!(service_input_attached(&project, &service).expect("input attached"));

    send_service_input(&project, &service, "x\n").expect("send detached input");
    thread::sleep(Duration::from_millis(180));

    let logs = service_logs(&project, &service).expect("service logs");
    assert!(logs.iter().any(|line| line.contains("INPUT:x")));

    let _ = stop_service(&project, &service);
}

#[cfg(unix)]
#[test]
fn start_and_stop_service_changes_status() {
    let (config, project, service) = runtime_config("sleep 3");

    let started = start_service(&config, &project, &service).expect("start service");
    assert!(matches!(
        started.state,
        ServiceRuntimeState::Starting | ServiceRuntimeState::Running
    ));

    let status = service_runtime_status(&config, &project, &service).expect("runtime status");
    assert!(matches!(
        status.state,
        ServiceRuntimeState::Starting
            | ServiceRuntimeState::Running
            | ServiceRuntimeState::Unhealthy
    ));

    let stopped = stop_service(&project, &service).expect("stop service");
    assert_eq!(stopped.state, ServiceRuntimeState::Stopped);
}

#[cfg(unix)]
#[test]
fn service_without_health_target_skips_active_health_checks() {
    let (config, project, service) = runtime_config("sleep 4");

    start_service(&config, &project, &service).expect("start service");
    thread::sleep(Duration::from_millis(2_300));

    let status = service_runtime_status(&config, &project, &service).expect("runtime status");
    assert_eq!(status.state, ServiceRuntimeState::Running);

    let _ = stop_service(&project, &service);
}

#[cfg(unix)]
#[test]
fn stop_project_all_continues_after_service_stop_error() {
    let (config, project, first_service, second_service) =
        runtime_config_with_two_services("sleep 6", "sleep 6");
    start_service(&config, &project, &first_service).expect("start first service");
    start_service(&config, &project, &second_service).expect("start second service");

    let locked_key = runtime_key(&project, &first_service);
    runtime_service_ops()
        .lock()
        .expect("op lock")
        .insert(locked_key.clone());

    let stop_result = stop_project_all(&config, &project);
    runtime_service_ops()
        .lock()
        .expect("op lock")
        .remove(&locked_key);
    assert!(stop_result.is_err());

    let second_status =
        service_runtime_status(&config, &project, &second_service).expect("second status");
    assert_eq!(second_status.state, ServiceRuntimeState::Stopped);

    let _ = stop_service(&project, &first_service);
}

#[test]
fn service_start_order_respects_dependencies() {
    let services = vec![
        ServiceConfig {
            name: "server".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(8080),
            protocol: ProxyEndpointProtocol::Http1,
            command: "sleep 2".to_string(),
            workdir: "/tmp".to_string(),
            env_files: vec![],
            depends_on: vec!["gateway".to_string()],
            autostart: false,
            health_path: None,
        },
        ServiceConfig {
            name: "gateway".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(8081),
            protocol: ProxyEndpointProtocol::Http1,
            command: "sleep 2".to_string(),
            workdir: "/tmp".to_string(),
            env_files: vec![],
            depends_on: vec![],
            autostart: false,
            health_path: None,
        },
    ];

    let order = service_start_order(&services).expect("valid order");
    assert_eq!(order, vec!["gateway".to_string(), "server".to_string()]);
}

#[test]
fn service_start_order_rejects_cycles() {
    let services = vec![
        ServiceConfig {
            name: "server".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(8080),
            protocol: ProxyEndpointProtocol::Http1,
            command: "sleep 2".to_string(),
            workdir: "/tmp".to_string(),
            env_files: vec![],
            depends_on: vec!["gateway".to_string()],
            autostart: false,
            health_path: None,
        },
        ServiceConfig {
            name: "gateway".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(8081),
            protocol: ProxyEndpointProtocol::Http1,
            command: "sleep 2".to_string(),
            workdir: "/tmp".to_string(),
            env_files: vec![],
            depends_on: vec!["server".to_string()],
            autostart: false,
            health_path: None,
        },
    ];

    let err = service_start_order(&services).expect_err("cycle must fail");
    assert!(err.contains("cycle"));
}

#[test]
fn grpc_health_service_name_trims_slashes() {
    assert_eq!(
        grpc_health_service_name(Some("/gateway.health.v1.Gateway")),
        Some("gateway.health.v1.Gateway".to_string())
    );
    assert_eq!(grpc_health_service_name(Some("   ")), None);
    assert_eq!(grpc_health_service_name(Some("/")), None);
}

#[test]
fn grpc_health_request_encoding_supports_empty_and_named_service() {
    let empty = encode_grpc_health_check_request(None);
    assert_eq!(empty, vec![0, 0, 0, 0, 0]);

    let named = encode_grpc_health_check_request(Some("gateway"));
    assert_eq!(named[0], 0);
    assert_eq!(
        u32::from_be_bytes([named[1], named[2], named[3], named[4]]),
        9
    );
    assert_eq!(
        &named[5..],
        &[0x0A, 0x07, b'g', b'a', b't', b'e', b'w', b'a', b'y']
    );
}

#[test]
fn grpc_health_response_decoding_reads_serving_status() {
    let serving_frame = vec![0, 0, 0, 0, 2, 0x08, 0x01];
    assert_eq!(decode_grpc_health_response_status(&serving_frame), Some(1));

    let not_serving_frame = vec![0, 0, 0, 0, 2, 0x08, 0x02];
    assert_eq!(
        decode_grpc_health_response_status(&not_serving_frame),
        Some(2)
    );
}

#[cfg(unix)]
#[test]
fn stop_service_terminates_descendant_process_tree() {
    let nonce = nonce();
    let script_path = std::env::temp_dir().join(format!("loopbox-descendants-{nonce}.sh"));
    let child_pid_path =
        std::env::temp_dir().join(format!("loopbox-descendants-child-{nonce}.pid"));
    std::fs::write(
        &script_path,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nsleep 30 &\necho $! > '{}'\nwait\n",
            child_pid_path.to_string_lossy()
        ),
    )
    .expect("write descendant script");

    let command = format!(
        "/bin/bash {}",
        shell_quote(script_path.to_string_lossy().as_ref())
    );
    let (config, project, service) = runtime_config_with_port(&command, None);

    start_service(&config, &project, &service).expect("start service");

    let mut child_pid = None;
    for _ in 0..40 {
        if child_pid_path.exists() {
            let pid = std::fs::read_to_string(&child_pid_path)
                .ok()
                .and_then(|content| content.trim().parse::<u32>().ok());
            if pid.is_some() {
                child_pid = pid;
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    let child_pid = child_pid.expect("child pid file should be populated");
    assert!(pid_exists(child_pid));

    let stopped = stop_service(&project, &service).expect("stop service");
    assert_eq!(stopped.state, ServiceRuntimeState::Stopped);

    let descendant_exited = wait_for_pid_exit(child_pid, Duration::from_secs(2));
    if !descendant_exited {
        let _ = terminate_pid_if_alive(child_pid, false);
    }

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&child_pid_path);

    assert!(
        descendant_exited,
        "descendant process {child_pid} should be terminated by stop_service"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "GitHub macOS runners intermittently keep the orphaned background job alive after group TERM/KILL"]
fn stop_service_terminates_group_members_when_leader_exits_early() {
    let nonce = nonce();
    let script_path = std::env::temp_dir().join(format!("loopbox-early-exit-{nonce}.sh"));
    let child_pid_path = std::env::temp_dir().join(format!("loopbox-early-exit-child-{nonce}.pid"));
    std::fs::write(
        &script_path,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ntrap '' HUP\nsleep 30 >/dev/null 2>&1 &\necho $! > '{}'\n",
            child_pid_path.to_string_lossy()
        ),
    )
    .expect("write early exit script");

    let command = format!(
        "/bin/bash {}",
        shell_quote(script_path.to_string_lossy().as_ref())
    );
    let (config, project, service) = runtime_config_with_port(&command, None);
    let started = start_service(&config, &project, &service).expect("start service");
    let leader_pid = started.pid.expect("started service pid");

    let mut child_pid = None;
    for _ in 0..40 {
        if child_pid_path.exists() {
            let pid = std::fs::read_to_string(&child_pid_path)
                .ok()
                .and_then(|content| content.trim().parse::<u32>().ok());
            if pid.is_some() {
                child_pid = pid;
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    let child_pid = child_pid.expect("child pid file should be populated");
    assert!(pid_exists(child_pid));
    assert!(
        wait_for_process_group_member(leader_pid, child_pid, Duration::from_secs(2)),
        "detached group member {child_pid} should join service process group {leader_pid}"
    );

    // Give the launching shell time to exit so stop logic must target the group,
    // not only the original leader pid.
    thread::sleep(Duration::from_millis(150));

    let stopped = stop_service(&project, &service).expect("stop service");
    assert_eq!(stopped.state, ServiceRuntimeState::Stopped);

    let descendant_exited = wait_for_pid_exit(child_pid, Duration::from_secs(2));
    if !descendant_exited {
        let _ = terminate_pid_if_alive(child_pid, false);
    }

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&child_pid_path);

    assert!(
        descendant_exited,
        "detached group member {child_pid} should be terminated by stop_service"
    );
}

#[cfg(unix)]
#[test]
fn service_logs_capture_stdout_and_stderr() {
    let (config, project, service) = runtime_config("echo hello-log; echo error-log 1>&2; sleep 1");
    start_service(&config, &project, &service).expect("start service");
    thread::sleep(Duration::from_millis(250));

    let logs = service_logs(&project, &service).expect("logs");
    assert!(logs.iter().any(|line| line.contains("hello-log")));
    assert!(logs.iter().any(|line| line.contains("error-log")));

    let _ = stop_service(&project, &service);
}

#[test]
fn service_logs_tail_returns_recent_lines() {
    let (_config, project, service) = runtime_config_with_port("sleep 1", None);
    let log_path = service_log_file_path(&project, &service);
    prepare_service_log_file(&log_path).expect("prepare log file");
    std::fs::write(&log_path, "line-1\nline-2\nline-3\n").expect("write test logs");

    let logs = service_logs_tail(&project, &service, 2).expect("logs tail");
    assert_eq!(logs, vec!["line-2".to_string(), "line-3".to_string()]);

    let _ = clear_service_logs(&project, &service);
}

#[test]
fn attach_replay_start_offset_replays_recent_lines() {
    let (_config, project, service) = runtime_config_with_port("sleep 1", None);
    let log_path = service_log_file_path(&project, &service);
    prepare_service_log_file(&log_path).expect("prepare log file");
    std::fs::write(&log_path, "line-1\nline-2\nline-3\nline-4\n").expect("write test logs");

    let offset = attach_replay_start_offset(&log_path, 2).expect("start offset");
    let (bytes, end_offset) = read_service_log_raw_delta(&log_path, offset).expect("raw delta");
    assert_eq!(String::from_utf8_lossy(&bytes), "line-3\nline-4\n");
    assert_eq!(end_offset, file_length(&log_path).expect("file length"));

    let _ = clear_service_logs(&project, &service);
}

#[test]
fn read_service_log_raw_delta_preserves_control_bytes() {
    let (_config, project, service) = runtime_config_with_port("sleep 1", None);
    let log_path = service_log_file_path(&project, &service);
    prepare_service_log_file(&log_path).expect("prepare log file");
    let payload = b"\x1b[2K\x1b[1Gexpo\r\n\x1b[?25l\x1b[39m";
    std::fs::write(&log_path, payload).expect("write raw payload");

    let (bytes, end_offset) = read_service_log_raw_delta(&log_path, 0).expect("raw delta");
    assert_eq!(bytes, payload);
    assert_eq!(end_offset, payload.len() as u64);

    let (empty, same_offset) =
        read_service_log_raw_delta(&log_path, payload.len() as u64 + 99).expect("bounded delta");
    assert!(empty.is_empty());
    assert_eq!(same_offset, payload.len() as u64);

    let _ = clear_service_logs(&project, &service);
}

#[test]
fn service_logs_seed_tail_when_buffer_empty_at_eof_offset() {
    let (_config, project, service) = runtime_config_with_port("sleep 1", None);
    let key = runtime_key(&project, &service);
    let log_path = service_log_file_path(&project, &service);
    prepare_service_log_file(&log_path).expect("prepare log file");
    std::fs::write(&log_path, "first-line\nsecond-line\n").expect("write test logs");
    let file_len = file_length(&log_path).expect("read file length");
    upsert_runtime_log_meta(RuntimeLogMetaEntry {
        key: key.clone(),
        project: project.clone(),
        service: service.clone(),
        file_offset: file_len,
        last_seen_ts: unix_timestamp(SystemTime::now()),
    })
    .expect("save log meta");
    runtime_store()
        .lock()
        .expect("runtime store lock")
        .log_buffers
        .remove(&key);

    drop_runtime_tracking(&project, &service);
    let logs = service_logs(&project, &service).expect("seeded logs");
    assert!(logs.iter().any(|line| line.contains("first-line")));
    assert!(logs.iter().any(|line| line.contains("second-line")));

    let _ = clear_service_logs(&project, &service);
}

#[test]
fn service_logs_persist_offset_after_new_lines() {
    let (_config, project, service) = runtime_config_with_port("sleep 1", None);
    let key = runtime_key(&project, &service);
    let log_path = service_log_file_path(&project, &service);
    prepare_service_log_file(&log_path).expect("prepare log file");
    let _ = reset_runtime_log_meta(&project, &service);
    std::fs::write(&log_path, "alpha\n").expect("write initial logs");

    let first = service_logs(&project, &service).expect("first logs");
    assert!(first.iter().any(|line| line.contains("alpha")));
    let first_meta = runtime_log_meta_entry_for_key(&key)
        .expect("load first meta")
        .expect("meta entry");
    let first_offset = first_meta.file_offset;
    assert!(first_offset > 0);

    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .expect("open log file");
        writeln!(file, "beta").expect("append log line");
    }

    let second = service_logs(&project, &service).expect("second logs");
    assert!(second.iter().any(|line| line.contains("beta")));
    let second_meta = runtime_log_meta_entry_for_key(&key)
        .expect("load second meta")
        .expect("meta entry");
    assert!(second_meta.file_offset > first_offset);

    let _ = clear_service_logs(&project, &service);
}

#[cfg(unix)]
#[test]
fn service_log_attached_tracks_running_state() {
    let (config, project, service) = runtime_config("sleep 3");
    start_service(&config, &project, &service).expect("start service");
    assert!(service_log_attached(&project, &service).expect("attached true"));

    let _ = stop_service(&project, &service);
    assert!(!service_log_attached(&project, &service).expect("attached false"));
}

#[cfg(unix)]
#[test]
fn detached_service_logs_can_be_reattached_from_file() {
    let (config, project, service) = runtime_config("echo reattach-log; sleep 3");
    start_service(&config, &project, &service).expect("start service");
    thread::sleep(Duration::from_millis(250));

    drop_runtime_tracking(&project, &service);
    let logs = service_logs(&project, &service).expect("reattached logs");
    assert!(logs.iter().any(|line| line.contains("reattach-log")));

    let _ = stop_service(&project, &service);
}

#[cfg(unix)]
#[test]
fn status_and_stop_work_after_runtime_store_is_dropped() {
    let (config, project, service) = runtime_config("sleep 4");
    let started = start_service(&config, &project, &service).expect("start service");
    let pid = started.pid.expect("pid");

    drop_runtime_tracking(&project, &service);

    let status = service_runtime_status(&config, &project, &service).expect("runtime status");
    assert_eq!(status.pid, Some(pid));
    assert!(matches!(
        status.state,
        ServiceRuntimeState::Starting
            | ServiceRuntimeState::Running
            | ServiceRuntimeState::Unhealthy
    ));

    let stopped = stop_service(&project, &service).expect("stop detached service");
    assert_eq!(stopped.state, ServiceRuntimeState::Stopped);
    if stopped.last_error.is_none() {
        assert!(!pid_exists(pid));
    }
}

#[test]
fn stale_registry_entries_are_pruned_and_running_history_resets() {
    let (config, project, service) = runtime_config("sleep 1");
    let key = runtime_key(&project, &service);

    let mut registry = load_runtime_pid_registry().expect("load registry");
    registry.entries.retain(|entry| entry.key != key);
    registry.entries.push(RuntimePidEntry {
        key: key.clone(),
        project: project.clone(),
        service: service.clone(),
        pid: 999_999,
        process_group_leader: false,
        command: "sleep 1".to_string(),
        workdir: "/tmp".to_string(),
        input_path: None,
        terminal_control_path: None,
        terminal_backend_version: None,
        terminal_cols: None,
        terminal_rows: None,
        recorded_at: unix_timestamp(SystemTime::now()),
    });
    save_runtime_pid_registry(&registry).expect("save registry");

    {
        let mut store = runtime_store().lock().expect("runtime store lock");
        store.history.insert(
            key.clone(),
            ServiceRuntimeSnapshot {
                project: project.clone(),
                service: service.clone(),
                state: ServiceRuntimeState::Running,
                pid: Some(999_999),
                started_at: Some(unix_timestamp(SystemTime::now())),
                exit_code: None,
                last_error: None,
            },
        );
    }

    let removed = cleanup_stale_runtime_processes().expect("cleanup");
    assert!(removed >= 1);

    let status = service_runtime_status(&config, &project, &service)
        .expect("runtime status after stale pid cleanup");
    assert_eq!(status.state, ServiceRuntimeState::Stopped);
}

#[test]
fn zero_pid_registry_entries_are_pruned() {
    let key = format!("zero-pid-{}", nonce());
    let mut registry = load_runtime_pid_registry().expect("load registry");
    registry.entries.retain(|entry| entry.key != key);
    registry.entries.push(RuntimePidEntry {
        key: key.clone(),
        project: "zero-pid-project".to_string(),
        service: "zero-pid-service".to_string(),
        pid: 0,
        process_group_leader: true,
        command: "sleep 1".to_string(),
        workdir: "/tmp".to_string(),
        input_path: None,
        terminal_control_path: None,
        terminal_backend_version: None,
        terminal_cols: None,
        terminal_rows: None,
        recorded_at: unix_timestamp(SystemTime::now()),
    });
    save_runtime_pid_registry(&registry).expect("save registry");

    let removed = cleanup_stale_runtime_processes().expect("cleanup");
    assert!(removed >= 1);

    let reloaded = load_runtime_pid_registry().expect("reload registry");
    assert!(!reloaded.entries.iter().any(|entry| entry.key == key));
}

#[test]
fn runtime_container_name_is_sanitized_and_stable() {
    let name = runtime_container_name("My Project/Dev", "postgres@primary");
    assert_eq!(name, "loopbox-my-project-dev-postgres-primary");

    let long = runtime_container_name(
        "a-very-long-project-name-that-keeps-going-and-going-and-going",
        "a-very-long-service-name-that-keeps-going-and-going-and-going",
    );
    assert!(long.len() <= 63);
    assert!(!long.ends_with('-'));
}
