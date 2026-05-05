use super::*;
use crate::loopbox::LoopboxConfig;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn generated_service_hosts_only_returns_service_entries() {
    let services = vec![
        ServiceConfig {
            name: "backend".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(8080),
            protocol: ProxyEndpointProtocol::Http1,
            command: "npm run dev".to_string(),
            workdir: "/tmp/niklasschmidt.dev".to_string(),
            env_files: vec![],
            depends_on: vec![],
            autostart: false,
            health_path: None,
        },
        ServiceConfig {
            name: "frontend".to_string(),
            runtime: crate::loopbox::ServiceRuntimeKind::Process,
            container: None,
            ports: vec![],
            port: Some(3000),
            protocol: ProxyEndpointProtocol::Http1,
            command: "npm run dev".to_string(),
            workdir: "/tmp/niklasschmidt.dev".to_string(),
            env_files: vec![],
            depends_on: vec![],
            autostart: false,
            health_path: None,
        },
    ];

    let hosts = generated_service_hosts("showcase", &services, "niklasschmidt.dev");

    assert_eq!(
        hosts,
        vec![
            "backend.showcase.niklasschmidt.dev".to_string(),
            "frontend.showcase.niklasschmidt.dev".to_string()
        ]
    );
    assert!(!hosts
        .iter()
        .any(|host| host == "showcase.niklasschmidt.dev"));
}

#[test]
fn update_project_preserves_default_service_when_it_still_exists() {
    let mut config = LoopboxConfig::default();
    config.projects.insert(
        "niklasschmidt.dev".to_string(),
        ProjectConfig {
            dir: "/tmp/niklasschmidt.dev".to_string(),
            ip: "127.0.0.30".to_string(),
            services: vec![
                ServiceConfig {
                    name: "backend".to_string(),
                    runtime: crate::loopbox::ServiceRuntimeKind::Process,
                    container: None,
                    ports: vec![],
                    port: Some(8080),
                    protocol: ProxyEndpointProtocol::Http1,
                    command: "pnpm backend".to_string(),
                    workdir: "/tmp/niklasschmidt.dev".to_string(),
                    env_files: vec![],
                    depends_on: vec![],
                    autostart: false,
                    health_path: None,
                },
                ServiceConfig {
                    name: "frontend".to_string(),
                    runtime: crate::loopbox::ServiceRuntimeKind::Process,
                    container: None,
                    ports: vec![],
                    port: Some(3000),
                    protocol: ProxyEndpointProtocol::Http1,
                    command: "pnpm frontend".to_string(),
                    workdir: "/tmp/niklasschmidt.dev".to_string(),
                    env_files: vec![],
                    depends_on: vec![],
                    autostart: false,
                    health_path: None,
                },
            ],
            default_open_service: Some("backend".to_string()),
            proxy_traffic_capture_enabled: None,
            proxy_traffic_capture_mode: None,
            grpc_proto_paths: vec![],
            proxy_endpoints: vec![],
        },
    );

    let input = UpdateProjectInput {
        dir: "/tmp/niklasschmidt.dev-new".to_string(),
        ip: String::new(),
        services: vec![
            ServiceEntry {
                name: "backend".to_string(),
                ports: vec![],
                port: "8088".to_string(),
                protocol: "http1".to_string(),
                command: "pnpm backend".to_string(),
                workdir: "/tmp/niklasschmidt.dev-new".to_string(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
            ServiceEntry {
                name: "gateway".to_string(),
                ports: vec![],
                port: "8081".to_string(),
                protocol: "http1".to_string(),
                command: "pnpm gateway".to_string(),
                workdir: "/tmp/niklasschmidt.dev-new".to_string(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
        ],
    };

    update_project(&mut config, "niklasschmidt.dev", &input).expect("update should succeed");

    let project = config
        .projects
        .get("niklasschmidt.dev")
        .expect("project should still exist");

    assert_eq!(project.dir, "/tmp/niklasschmidt.dev-new");
    assert_eq!(project.ip, "127.0.0.30");
    assert_eq!(project.services.len(), 2);
    assert_eq!(project.services[0].name, "backend");
    assert_eq!(project.services[0].port, Some(8088));
    assert_eq!(project.services[1].name, "gateway");
    assert_eq!(project.default_open_service, Some("backend".to_string()));
}

#[test]
fn add_project_allows_service_without_port() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "tools".to_string(),
        dir: "/tmp/tools".to_string(),
        ip: "127.0.0.40".to_string(),
        services: vec![ServiceEntry {
            name: "convex".to_string(),
            ports: vec![],
            port: String::new(),
            protocol: "http1".to_string(),
            command: "pnpm run dev:convex".to_string(),
            workdir: "/tmp/tools".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    assert_eq!(project.services[0].port, None);
}

#[test]
fn default_open_service_prefers_service_with_port() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "mixed".to_string(),
        dir: "/tmp/mixed".to_string(),
        ip: "127.0.0.41".to_string(),
        services: vec![
            ServiceEntry {
                name: "worker".to_string(),
                ports: vec![],
                port: String::new(),
                protocol: "http1".to_string(),
                command: "pnpm worker".to_string(),
                workdir: "/tmp/mixed".to_string(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
            ServiceEntry {
                name: "frontend".to_string(),
                ports: vec![],
                port: "3000".to_string(),
                protocol: "http1".to_string(),
                command: "pnpm dev".to_string(),
                workdir: "/tmp/mixed".to_string(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
        ],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    assert_eq!(project.default_open_service.as_deref(), Some("frontend"));
}

#[test]
fn add_project_writes_agents_guidance_file() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("unix time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loopbox-agents-guidance-{nonce}"));
    fs::create_dir_all(&root).expect("create temp project dir");

    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "guided".to_string(),
        dir: root.display().to_string(),
        ip: "127.0.0.52".to_string(),
        services: vec![ServiceEntry {
            name: "web".to_string(),
            ports: vec![],
            port: "3000".to_string(),
            protocol: "http1".to_string(),
            command: "npm run dev".to_string(),
            workdir: root.display().to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    assert_eq!(project_name, "guided");

    let agents_path = root.join("AGENTS.md");
    let agents = fs::read_to_string(&agents_path).expect("read AGENTS.md");
    assert!(agents.contains("## Loopbox Agent API"));
    assert!(agents.contains("Read discovery_file first on every new session or reconnect."));
    assert!(agents.contains("http://127.0.0.1:39393/v1/openapi.json"));
    assert!(agents.contains("/Users/niklas/.config/loopbox/agent-api.json"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn preview_add_project_does_not_write_agent_guidance_or_mutate_config() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("unix time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loopbox-preview-no-side-effects-{nonce}"));
    fs::create_dir_all(&root).expect("create temp project dir");

    let config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "previewed".to_string(),
        dir: root.display().to_string(),
        ip: String::new(),
        services: vec![ServiceEntry {
            name: "web".to_string(),
            ports: vec![],
            port: "3000".to_string(),
            protocol: "http1".to_string(),
            command: "npm run dev".to_string(),
            workdir: root.display().to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let (project_name, project) =
        preview_add_project(&config, &input).expect("project preview should validate");

    assert_eq!(project_name, "previewed");
    assert!(validate_project_ip(&config.global, &project.ip).is_ok());
    assert!(config.projects.is_empty());
    assert!(!root.join("AGENTS.md").exists());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn add_project_rejects_unknown_dependencies() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "deps".to_string(),
        dir: "/tmp/deps".to_string(),
        ip: "127.0.0.42".to_string(),
        services: vec![ServiceEntry {
            name: "server".to_string(),
            ports: vec![],
            port: "8080".to_string(),
            protocol: "http1".to_string(),
            command: "pnpm dev".to_string(),
            workdir: "/tmp/deps".to_string(),
            env_files: String::new(),
            depends_on: "gateway".to_string(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let err = add_project(&mut config, &input).expect_err("unknown dependency must fail");
    assert!(err.contains("depends on unknown service"));
}

#[test]
fn add_project_normalizes_dependency_list() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "depsnorm".to_string(),
        dir: "/tmp/depsnorm".to_string(),
        ip: "127.0.0.43".to_string(),
        services: vec![
            ServiceEntry {
                name: "gateway".to_string(),
                ports: vec![],
                port: "8081".to_string(),
                protocol: "http1".to_string(),
                command: "pnpm gateway".to_string(),
                workdir: "/tmp/depsnorm".to_string(),
                env_files: String::new(),
                depends_on: String::new(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
            ServiceEntry {
                name: "server".to_string(),
                ports: vec![],
                port: "8080".to_string(),
                protocol: "http1".to_string(),
                command: "pnpm server".to_string(),
                workdir: "/tmp/depsnorm".to_string(),
                env_files: String::new(),
                depends_on: "gateway, SERVER, gateway".to_string(),
                autostart: false,
                health_path: String::new(),
                runtime: "process".to_string(),
                container_image: String::new(),
                container_args: String::new(),
                container_env: String::new(),
                container_volumes: String::new(),
                container_auto_remove: false,
            },
        ],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    let server = project
        .services
        .iter()
        .find(|service| service.name == "server")
        .expect("server service");
    assert_eq!(server.depends_on, vec!["gateway".to_string()]);
}

#[test]
fn add_project_parses_service_protocol() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "grpc".to_string(),
        dir: "/tmp/grpc".to_string(),
        ip: "127.0.0.44".to_string(),
        services: vec![ServiceEntry {
            name: "gateway".to_string(),
            ports: vec![],
            port: "50051".to_string(),
            protocol: "grpc_h2c".to_string(),
            command: "go run ./cmd/gateway".to_string(),
            workdir: "/tmp/grpc".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    assert_eq!(project.services[0].protocol, ProxyEndpointProtocol::GrpcH2c);
}

#[test]
fn add_project_parses_container_runtime_service() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "containers".to_string(),
        dir: "/tmp/containers".to_string(),
        ip: "127.0.0.45".to_string(),
        services: vec![ServiceEntry {
            name: "db".to_string(),
            ports: vec![],
            port: "5432".to_string(),
            protocol: "tcp_passthrough".to_string(),
            command: String::new(),
            workdir: "/tmp/containers".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "container".to_string(),
            container_image: "postgres:16".to_string(),
            container_args: "-c max_connections=200".to_string(),
            container_env: "POSTGRES_DB=app\nPOSTGRES_PASSWORD=secret".to_string(),
            container_volumes: "/tmp/pg:/var/lib/postgresql/data".to_string(),
            container_auto_remove: true,
        }],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    let service = &project.services[0];
    assert_eq!(service.runtime, ServiceRuntimeKind::Container);
    assert_eq!(
        service
            .container
            .as_ref()
            .map(|container| container.image.as_str()),
        Some("postgres:16")
    );
    assert_eq!(service.command, "");
}

#[test]
fn add_project_rejects_container_runtime_without_image() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "missing-image".to_string(),
        dir: "/tmp/containers".to_string(),
        ip: "127.0.0.46".to_string(),
        services: vec![ServiceEntry {
            name: "db".to_string(),
            ports: vec![],
            port: "5432".to_string(),
            protocol: "tcp_passthrough".to_string(),
            command: String::new(),
            workdir: "/tmp/containers".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "container".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let err = add_project(&mut config, &input).expect_err("missing image must fail");
    assert!(err.contains("requires an image"));
}

#[test]
fn add_project_rejects_unknown_service_protocol() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "badproto".to_string(),
        dir: "/tmp/badproto".to_string(),
        ip: "127.0.0.45".to_string(),
        services: vec![ServiceEntry {
            name: "api".to_string(),
            ports: vec![],
            port: "8080".to_string(),
            protocol: "weird".to_string(),
            command: "pnpm dev".to_string(),
            workdir: "/tmp/badproto".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let err = add_project(&mut config, &input).expect_err("unknown protocol must fail");
    assert!(err.contains("invalid protocol"));
}

#[test]
fn add_project_normalizes_smart_dashes_in_command() {
    let mut config = LoopboxConfig::default();
    let input = AddProjectInput {
        name: "smart-dash".to_string(),
        dir: "/tmp/smart-dash".to_string(),
        ip: "127.0.0.47".to_string(),
        services: vec![ServiceEntry {
            name: "mobile".to_string(),
            ports: vec![],
            port: "8081".to_string(),
            protocol: "http1".to_string(),
            command: "npx expo run:ios —device 'iPhone von Niklas'".to_string(),
            workdir: "/tmp/smart-dash".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            runtime: "process".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        }],
    };

    let project_name = add_project(&mut config, &input).expect("project should be added");
    let project = config
        .projects
        .get(&project_name)
        .expect("project should exist");
    assert_eq!(
        project.services[0].command,
        "npx expo run:ios --device 'iPhone von Niklas'"
    );
}
