use super::*;
use crate::loopbox::{
    GlobalConfig, LoopboxConfig, ProjectConfig, ProxyCaptureMode, ProxyEndpointConfig,
    ProxyEndpointProtocol, ResourceMetricsSettings, ServiceConfig,
};
use std::collections::BTreeMap;

#[test]
fn normalize_config_assigns_first_service_as_default_open_when_invalid() {
    let mut config = LoopboxConfig {
        global: GlobalConfig::default(),
        projects: BTreeMap::from([(
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
                        workdir: String::new(),
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
                        workdir: String::new(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                ],
                default_open_service: Some("does-not-exist".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };

    normalize_config(&mut config);
    let project = config
        .projects
        .get("niklasschmidt.dev")
        .expect("project should be present");

    assert_eq!(project.default_open_service.as_deref(), Some("backend"));
    assert_eq!(project.services[0].workdir, "/tmp/niklasschmidt.dev");
}

#[test]
fn normalize_config_upgrades_legacy_test_suffix_to_localhost() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            domain_suffix: "test".to_string(),
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    normalize_config(&mut config);

    assert_eq!(config.global.domain_suffix, "localhost");
}

#[test]
fn normalize_config_migrates_legacy_body_preview_flag_to_capture_mode() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            proxy_traffic: crate::loopbox::ProxyTrafficSettings {
                capture_mode_default: ProxyCaptureMode::Metadata,
                capture_body_preview: true,
                ..crate::loopbox::ProxyTrafficSettings::default()
            },
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    normalize_config(&mut config);

    let expected_mode = ProxyCaptureMode::BodyPreview;
    assert_eq!(
        config.global.proxy_traffic.capture_mode_default,
        expected_mode
    );
}

#[test]
fn normalize_config_defaults_and_clamps_resource_metrics_settings() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            resource_metrics: ResourceMetricsSettings {
                enabled: true,
                sample_interval_secs: 0,
                retention_days: 255,
                max_storage_mb: 10_000,
            },
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    normalize_config(&mut config);

    assert!(config.global.resource_metrics.enabled);
    assert_eq!(config.global.resource_metrics.sample_interval_secs, 5);
    assert_eq!(config.global.resource_metrics.retention_days, 90);
    assert_eq!(config.global.resource_metrics.max_storage_mb, 5_000);

    config.global.resource_metrics.sample_interval_secs = 1;
    config.global.resource_metrics.retention_days = 0;
    config.global.resource_metrics.max_storage_mb = 1;

    normalize_config(&mut config);

    assert_eq!(config.global.resource_metrics.sample_interval_secs, 2);
    assert_eq!(config.global.resource_metrics.retention_days, 7);
    assert_eq!(config.global.resource_metrics.max_storage_mb, 25);
}

#[test]
fn normalize_config_sanitizes_proxy_endpoints() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            proxy_endpoints: vec![
                ProxyEndpointConfig {
                    name: "   ".to_string(),
                    listen_host: " ".to_string(),
                    listen_port: 50051,
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    upstream_host: "127.0.0.30".to_string(),
                    upstream_port: 50052,
                    authority: None,
                    project_name: None,
                    service_name: None,
                },
                ProxyEndpointConfig {
                    name: "duplicate-listener".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 50051,
                    protocol: ProxyEndpointProtocol::TcpPassthrough,
                    upstream_host: "127.0.0.31".to_string(),
                    upstream_port: 50053,
                    authority: None,
                    project_name: None,
                    service_name: None,
                },
                ProxyEndpointConfig {
                    name: "missing-upstream".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 50061,
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    upstream_host: "   ".to_string(),
                    upstream_port: 50062,
                    authority: None,
                    project_name: None,
                    service_name: None,
                },
            ],
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    normalize_config(&mut config);

    assert_eq!(config.global.proxy_endpoints.len(), 1);
    let endpoint = &config.global.proxy_endpoints[0];
    assert_eq!(endpoint.name, "endpoint-1");
    assert_eq!(endpoint.listen_host, "127.0.0.1");
    assert_eq!(endpoint.listen_port, 50051);
    assert_eq!(endpoint.upstream_host, "127.0.0.30");
    assert_eq!(endpoint.upstream_port, 50052);
    assert_eq!(endpoint.protocol, ProxyEndpointProtocol::GrpcH2c);
}

#[test]
fn normalize_config_keeps_distinct_grpc_authority_routes_on_same_listener() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            proxy_endpoints: vec![
                ProxyEndpointConfig {
                    name: "grpc-a".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 50051,
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    upstream_host: "127.0.0.30".to_string(),
                    upstream_port: 50051,
                    authority: Some("gateway.skybrid.localhost".to_string()),
                    project_name: None,
                    service_name: None,
                },
                ProxyEndpointConfig {
                    name: "grpc-b".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 50051,
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    upstream_host: "127.0.0.31".to_string(),
                    upstream_port: 50051,
                    authority: Some("gateway.other.localhost".to_string()),
                    project_name: None,
                    service_name: None,
                },
            ],
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    normalize_config(&mut config);

    assert_eq!(config.global.proxy_endpoints.len(), 2);
    assert_eq!(
        config.global.proxy_endpoints[0].authority.as_deref(),
        Some("gateway.skybrid.localhost")
    );
    assert_eq!(
        config.global.proxy_endpoints[1].authority.as_deref(),
        Some("gateway.other.localhost")
    );
}

#[test]
fn normalize_config_migrates_matching_global_endpoint_to_project() {
    let mut config = LoopboxConfig {
        global: GlobalConfig {
            proxy_endpoints: vec![ProxyEndpointConfig {
                name: "gateway".to_string(),
                listen_host: "127.0.0.1".to_string(),
                listen_port: 50051,
                protocol: ProxyEndpointProtocol::GrpcH2c,
                upstream_host: "127.0.0.30".to_string(),
                upstream_port: 50052,
                authority: Some("gateway.skybrid.localhost".to_string()),
                project_name: None,
                service_name: None,
            }],
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "skybrid".to_string(),
            ProjectConfig {
                dir: "/tmp/skybrid".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![ServiceConfig {
                    name: "gateway".to_string(),
                    runtime: crate::loopbox::ServiceRuntimeKind::Process,
                    container: None,
                    ports: vec![],
                    port: Some(50052),
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    command: "go run ./cmd/gateway".to_string(),
                    workdir: "/tmp/skybrid".to_string(),
                    env_files: vec![],
                    depends_on: vec![],
                    autostart: false,
                    health_path: None,
                }],
                default_open_service: Some("gateway".to_string()),
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };

    normalize_config(&mut config);

    assert!(config.global.proxy_endpoints.is_empty());
    let project = config
        .projects
        .get("skybrid")
        .expect("project should exist");
    assert_eq!(project.proxy_endpoints.len(), 1);
    assert_eq!(
        project.proxy_endpoints[0].project_name.as_deref(),
        Some("skybrid")
    );
    assert_eq!(
        project.proxy_endpoints[0].authority.as_deref(),
        Some("gateway.skybrid.localhost")
    );
}
