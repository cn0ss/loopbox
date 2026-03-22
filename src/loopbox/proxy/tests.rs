use super::{
    build_proxy_endpoint_routes, build_proxy_routes, has_header_terminator, parse_request_host,
    parse_request_line, parse_response_status, select_grpc_route_for_authority, strip_host_port,
    ProxyEndpointKey, ProxyEndpointRoute,
};
use crate::loopbox::{
    GlobalConfig, LoopboxConfig, ProjectConfig, ProxyEndpointConfig, ProxyEndpointProtocol,
    ServiceConfig, ServiceRuntimeKind,
};
use std::collections::BTreeMap;

#[test]
fn host_parser_extracts_host_without_port() {
    let request =
        b"GET / HTTP/1.1\r\nHost: web.vereinsapp.localhost:3000\r\nConnection: close\r\n\r\n";
    assert_eq!(
        parse_request_host(request).as_deref(),
        Some("web.vereinsapp.localhost")
    );
}

#[test]
fn strip_host_port_handles_ipv6_style_hosts() {
    assert_eq!(strip_host_port("[::1]:8080"), "::1");
    assert_eq!(
        strip_host_port("frontend.app.niklasschmidt.dev:5173"),
        "frontend.app.niklasschmidt.dev"
    );
}

#[test]
fn header_terminator_detection_works() {
    assert!(has_header_terminator(
        b"GET / HTTP/1.1\r\nHost: demo.localhost\r\n\r\n"
    ));
    assert!(!has_header_terminator(
        b"GET / HTTP/1.1\r\nHost: demo.localhost\r\n"
    ));
}

#[test]
fn request_line_parser_extracts_method_and_path() {
    let request = b"POST /api/items?status=open HTTP/1.1\r\nHost: demo.localhost\r\n\r\n";
    let (method, path) = parse_request_line(request);
    assert_eq!(method, "POST");
    assert_eq!(path, "/api/items?status=open");
}

#[test]
fn response_status_parser_extracts_status_code() {
    let response = b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n";
    assert_eq!(parse_response_status(response), Some(201));
}

#[test]
fn route_builder_skips_non_http_service_protocols() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            domain_suffix: "localhost".to_string(),
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "skybrid".to_string(),
            ProjectConfig {
                dir: "/tmp/skybrid".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![ServiceConfig {
                    name: "gateway".to_string(),
                    runtime: ServiceRuntimeKind::Process,
                    container: None,
                    ports: vec![],
                    port: Some(50051),
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

    let routes = build_proxy_routes(&config);
    assert!(!routes.contains_key("gateway.skybrid.localhost"));
}

#[test]
fn endpoint_route_builder_includes_project_proxy_endpoints() {
    let config = LoopboxConfig {
        global: GlobalConfig::default(),
        projects: BTreeMap::from([(
            "skybrid".to_string(),
            ProjectConfig {
                dir: "/tmp/skybrid".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![ServiceConfig {
                    name: "gateway".to_string(),
                    runtime: ServiceRuntimeKind::Process,
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
            },
        )]),
    };

    let routes = build_proxy_endpoint_routes(&config);
    assert_eq!(routes.len(), 2);
    let route = routes
        .values()
        .flat_map(|route_set| route_set.iter())
        .find(|route| route.name == "gateway")
        .expect("project endpoint route should exist");
    assert_eq!(route.project_name, "skybrid");
    assert_eq!(route.service_name, "gateway");
    assert_eq!(
        route.authority.as_deref(),
        Some("gateway.skybrid.localhost")
    );
}

#[test]
fn endpoint_route_builder_auto_includes_grpc_service_listener() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            domain_suffix: "localhost".to_string(),
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "skybrid".to_string(),
            ProjectConfig {
                dir: "/tmp/skybrid".to_string(),
                ip: "127.0.0.30".to_string(),
                services: vec![ServiceConfig {
                    name: "gateway".to_string(),
                    runtime: ServiceRuntimeKind::Process,
                    container: None,
                    ports: vec![],
                    port: Some(50051),
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

    let routes = build_proxy_endpoint_routes(&config);
    let key = ProxyEndpointKey {
        listen_host: "127.0.0.1".to_string(),
        listen_port: 50051,
    };
    let route_set = routes
        .get(&key)
        .expect("auto grpc listener should exist for service port");
    let route = route_set
        .iter()
        .find(|route| route.service_name == "gateway")
        .expect("auto grpc route should exist");
    assert_eq!(route.protocol, ProxyEndpointProtocol::GrpcH2c);
    assert_eq!(route.upstream_host, "127.0.0.30");
    assert_eq!(route.upstream_port, 50051);
    assert_eq!(
        route.authority.as_deref(),
        Some("gateway.skybrid.localhost")
    );
}

#[test]
fn endpoint_route_builder_includes_global_proxy_endpoints() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            proxy_endpoints: vec![
                ProxyEndpointConfig {
                    name: "gateway".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 50051,
                    protocol: ProxyEndpointProtocol::GrpcH2c,
                    upstream_host: "127.0.0.30".to_string(),
                    upstream_port: 50052,
                    authority: None,
                    project_name: None,
                    service_name: None,
                },
                ProxyEndpointConfig {
                    name: "internal-tcp".to_string(),
                    listen_host: "127.0.0.1".to_string(),
                    listen_port: 15000,
                    protocol: ProxyEndpointProtocol::TcpPassthrough,
                    upstream_host: "127.0.0.40".to_string(),
                    upstream_port: 15001,
                    authority: None,
                    project_name: None,
                    service_name: None,
                },
            ],
            ..GlobalConfig::default()
        },
        projects: BTreeMap::new(),
    };

    let routes = build_proxy_endpoint_routes(&config);
    assert_eq!(routes.len(), 2);
    let grpc_route = routes
        .values()
        .flat_map(|route_set| route_set.iter())
        .find(|route| route.name == "gateway")
        .expect("grpc endpoint route should exist");
    assert_eq!(grpc_route.protocol, ProxyEndpointProtocol::GrpcH2c);
    assert_eq!(grpc_route.upstream_host, "127.0.0.30");
    assert_eq!(grpc_route.upstream_port, 50052);
}

#[test]
fn grpc_authority_route_selection_prefers_matching_route() {
    let routes = vec![
        ProxyEndpointRoute {
            name: "default".to_string(),
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: "127.0.0.10".to_string(),
            upstream_port: 50051,
            authority: None,
            project_name: "demo".to_string(),
            service_name: "default".to_string(),
        },
        ProxyEndpointRoute {
            name: "skybrid".to_string(),
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: "127.0.0.30".to_string(),
            upstream_port: 50051,
            authority: Some("gateway.skybrid.localhost".to_string()),
            project_name: "skybrid".to_string(),
            service_name: "gateway".to_string(),
        },
    ];

    let matched = select_grpc_route_for_authority(&routes, Some("gateway.skybrid.localhost"))
        .expect("route should match");
    assert_eq!(matched.name, "skybrid");

    let fallback = select_grpc_route_for_authority(&routes, Some("unknown.localhost"))
        .expect("default route should match");
    assert_eq!(fallback.name, "default");
}
