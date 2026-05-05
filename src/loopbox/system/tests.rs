use super::{apply_script, managed_hosts_block, proxy_redirect_required};
use crate::loopbox::{GlobalConfig, LoopboxConfig, ProjectConfig, ServiceConfig};
use std::collections::BTreeMap;

#[test]
fn proxy_anchor_generation_contains_expected_rule() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            domain_suffix: "niklasschmidt.dev".to_string(),
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "app".to_string(),
            ProjectConfig {
                dir: "/tmp/niklasschmidt.dev".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![
                    ServiceConfig {
                        name: "frontend".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(3000),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "pnpm dev".to_string(),
                        workdir: "/tmp/niklasschmidt.dev".to_string(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                    ServiceConfig {
                        name: "worker".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: None,
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "pnpm worker".to_string(),
                        workdir: "/tmp/niklasschmidt.dev".to_string(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                ],
                default_open_service: Some("frontend".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };

    assert!(proxy_redirect_required(&config));
    let script = apply_script(&config);
    assert!(script.contains("127.0.0.30"));
    assert!(script.contains("127.0.0.1"));
    assert!(script.contains("port 80"));
    assert!(script.contains("18080"));
}

#[test]
fn managed_hosts_block_points_service_hosts_to_project_ip() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            domain_suffix: "niklasschmidt.dev".to_string(),
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "app".to_string(),
            ProjectConfig {
                dir: "/tmp/niklasschmidt.dev".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![
                    ServiceConfig {
                        name: "frontend".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(3000),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "pnpm dev".to_string(),
                        workdir: "/tmp/niklasschmidt.dev".to_string(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                    ServiceConfig {
                        name: "gateway".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(8081),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "go run cmd/gateway/main.go".to_string(),
                        workdir: "/tmp/niklasschmidt.dev".to_string(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                ],
                default_open_service: Some("frontend".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };

    let hosts = managed_hosts_block(&config);
    assert!(
        hosts.contains("127.0.0.30 frontend.app.niklasschmidt.dev gateway.app.niklasschmidt.dev")
    );
    assert!(!hosts.contains("127.0.0.1 frontend.app.niklasschmidt.dev"));
}
