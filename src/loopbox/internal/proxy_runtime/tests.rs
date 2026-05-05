use super::{
    build_proxy_endpoint_routes, build_proxy_routes, has_header_terminator,
    parse_and_redact_headers, parse_day_from_traffic_filename, parse_day_key, parse_request_host,
    parse_request_line, parse_response_status, proxy_event_to_har_entry, redact_path_query,
    sanitize_proxy_max_storage_mb, sanitize_proxy_retention_days, sanitize_proxy_writer_queue_size,
    select_grpc_route_for_authority, strip_host_port, GlobalConfig, LoopboxConfig, PreviewCapture,
    ProjectConfig, ProxyCaptureMode, ProxyEndpointConfig, ProxyEndpointProtocol, ProxyTrafficEvent,
    ProxyTrafficHeader, ProxyTrafficSettings, ServiceConfig, ServiceRuntimeKind,
};
use std::collections::BTreeMap;

fn project_proxy_traffic_enabled(config: &LoopboxConfig, project_name: &str) -> bool {
    config
        .projects
        .get(project_name)
        .map(|project| {
            project
                .proxy_traffic_capture_enabled
                .unwrap_or(config.global.proxy_traffic.capture_enabled_by_default)
        })
        .unwrap_or(false)
}

fn project_proxy_traffic_capture_mode(
    config: &LoopboxConfig,
    project_name: &str,
) -> ProxyCaptureMode {
    let selected = config
        .projects
        .get(project_name)
        .and_then(|project| project.proxy_traffic_capture_mode.clone())
        .unwrap_or_else(|| config.global.proxy_traffic.capture_mode_default.clone());
    super::enforce_traffic_capture_mode(selected)
}

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
fn header_parser_redacts_sensitive_headers() {
    let request = b"GET / HTTP/1.1\r\nHost: demo.localhost\r\nAuthorization: Bearer abc123\r\nCookie: session=topsecret\r\nX-Api-Key: secret-key\r\nUser-Agent: curl/8.6.0\r\n\r\n";
    let redacted_headers = vec![
        "authorization".to_string(),
        "cookie".to_string(),
        "set-cookie".to_string(),
        "x-api-key".to_string(),
        "proxy-authorization".to_string(),
    ];
    let headers = parse_and_redact_headers(request, &redacted_headers);
    assert!(
        headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("host")
                && header.value == "demo.localhost")
    );
    assert!(headers
        .iter()
        .any(|header| header.name.eq_ignore_ascii_case("authorization")
            && header.value == "[redacted]"));
    assert!(headers
        .iter()
        .any(|header| header.name.eq_ignore_ascii_case("cookie") && header.value == "[redacted]"));
    assert!(headers.iter().any(
        |header| header.name.eq_ignore_ascii_case("x-api-key") && header.value == "[redacted]"
    ));
}

#[test]
fn header_parser_keeps_duplicate_headers() {
    let response = b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Type: text/plain\r\n\r\nhello";
    let redacted_headers = vec![
        "authorization".to_string(),
        "cookie".to_string(),
        "set-cookie".to_string(),
        "x-api-key".to_string(),
        "proxy-authorization".to_string(),
    ];
    let headers = parse_and_redact_headers(response, &redacted_headers);
    let cookie_count = headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("set-cookie"))
        .count();
    assert_eq!(cookie_count, 2);
    assert!(headers.iter().all(|header| {
        if header.name.eq_ignore_ascii_case("set-cookie") {
            header.value == "[redacted]"
        } else {
            true
        }
    }));
}

#[test]
fn body_preview_marks_binary_payload() {
    let mut capture = PreviewCapture::new(16, true);
    capture.ingest(&[0x00, 0xFF, 0x01, 0x02]);
    let result = capture.finish();
    assert!(result.binary);
    assert!(result.preview.is_none());
}

#[test]
fn body_preview_marks_truncated_text() {
    let mut capture = PreviewCapture::new(5, true);
    capture.ingest(b"hello-world");
    let result = capture.finish();
    assert!(result.truncated);
    assert!(!result.binary);
    assert_eq!(result.preview.as_deref(), Some("hello"));
}

#[test]
fn grpc_preview_renders_unframed_payload_text() {
    let payload = br#"{"ok":true}"#;
    let mut frame = Vec::new();
    frame.push(0_u8);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);

    let rendered = super::render_grpc_preview(&frame, &[], None, None, false);
    let rendered = rendered.expect("gRPC preview should render when traffic capture is enabled");
    assert!(rendered.contains(r#"{"ok":true}"#));
}

#[test]
fn split_grpc_frames_marks_incomplete_frame() {
    let payload = b"abcdef";
    let mut frame = Vec::new();
    frame.push(0_u8);
    frame.extend_from_slice(&10_u32.to_be_bytes());
    frame.extend_from_slice(payload);

    let (frames, trailing) = super::split_grpc_frames(&frame);
    assert!(!trailing);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1, payload);
    assert!(!frames[0].0.complete);
    assert_eq!(frames[0].0.declared_len, 10);
}

#[test]
fn beautify_protoc_output_formats_json_string_field() {
    let input = r#"items: "[{\"name\":\"demo\",\"ok\":true}]" "#.trim();
    let output = super::beautify_protoc_text_output(input);
    assert!(output.contains("items:"));
    assert!(output.contains("\"name\": \"demo\""));
    assert!(output.contains("\"ok\": true"));
}

#[test]
fn beautify_protoc_output_keeps_normal_fields() {
    let input = "kubernetes_version: \"v1.34.4+k3s1\"";
    let output = super::beautify_protoc_text_output(input);
    assert_eq!(output, input);
}

#[test]
fn route_builder_skips_portless_services() {
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
                        runtime: ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(3000),
                        protocol: ProxyEndpointProtocol::Http1,
                        command: "pnpm dev".to_string(),
                        workdir: "/tmp/niklasschmidt.dev".to_string(),
                        env_files: vec![],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    },
                    ServiceConfig {
                        name: "worker".to_string(),
                        runtime: ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: None,
                        protocol: ProxyEndpointProtocol::Http1,
                        command: "pnpm run worker".to_string(),
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

    let routes = build_proxy_routes(&config);
    assert_eq!(routes.len(), 1);
    assert!(routes.contains_key("frontend.app.niklasschmidt.dev"));
    assert!(!routes.contains_key("worker.app.niklasschmidt.dev"));
    let route = routes
        .get("frontend.app.niklasschmidt.dev")
        .expect("route should exist");
    assert_eq!(route.capture_mode, ProxyCaptureMode::Metadata);
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
    let key = super::ProxyEndpointKey {
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
        super::ProxyEndpointRoute {
            name: "default".to_string(),
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: "127.0.0.10".to_string(),
            upstream_port: 50051,
            authority: None,
            project_name: "demo".to_string(),
            service_name: "default".to_string(),
            grpc_proto_paths: vec![],
            capture_enabled: false,
            capture_mode: ProxyCaptureMode::Metadata,
            capture_text_only: true,
            redacted_header_names: vec![],
            redacted_query_keys: vec![],
            request_body_preview_max_bytes: 1024,
            response_body_preview_max_bytes: 1024,
        },
        super::ProxyEndpointRoute {
            name: "skybrid".to_string(),
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: "127.0.0.30".to_string(),
            upstream_port: 50051,
            authority: Some("gateway.skybrid.localhost".to_string()),
            project_name: "skybrid".to_string(),
            service_name: "gateway".to_string(),
            grpc_proto_paths: vec![],
            capture_enabled: true,
            capture_mode: ProxyCaptureMode::Metadata,
            capture_text_only: true,
            redacted_header_names: vec![],
            redacted_query_keys: vec![],
            request_body_preview_max_bytes: 1024,
            response_body_preview_max_bytes: 1024,
        },
    ];

    let matched = select_grpc_route_for_authority(&routes, Some("gateway.skybrid.localhost"))
        .expect("route should match");
    assert_eq!(matched.name, "skybrid");

    let fallback = select_grpc_route_for_authority(&routes, Some("unknown.localhost"))
        .expect("default route should match");
    assert_eq!(fallback.name, "default");
}

#[test]
fn project_capture_setting_falls_back_to_global_default() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            proxy_traffic: ProxyTrafficSettings {
                capture_enabled_by_default: true,
                ..ProxyTrafficSettings::default()
            },
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.20".to_string(),
                services: vec![],
                default_open_service: None,
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };
    assert!(project_proxy_traffic_enabled(&config, "demo"));
}

#[test]
fn project_capture_mode_falls_back_to_global_default() {
    let config = LoopboxConfig {
        global: GlobalConfig {
            proxy_traffic: ProxyTrafficSettings {
                capture_mode_default: ProxyCaptureMode::Headers,
                ..ProxyTrafficSettings::default()
            },
            ..GlobalConfig::default()
        },
        projects: BTreeMap::from([(
            "demo".to_string(),
            ProjectConfig {
                dir: "/tmp/demo".to_string(),
                ip: "127.0.0.20".to_string(),
                services: vec![],
                default_open_service: None,
                proxy_traffic_capture_enabled: None,
                proxy_traffic_capture_mode: None,
                grpc_proto_paths: vec![],
                proxy_endpoints: vec![],
            },
        )]),
    };
    assert_eq!(
        project_proxy_traffic_capture_mode(&config, "demo"),
        ProxyCaptureMode::Headers
    );
}

#[test]
fn query_key_redaction_works() {
    let path = redact_path_query(
        "/callback?code=abc123&state=ok&token=secret",
        &["token".to_string(), "code".to_string()],
    );
    assert_eq!(path, "/callback?code=[redacted]&state=ok&token=[redacted]");
}

#[test]
fn writer_queue_limit_is_clamped() {
    assert_eq!(sanitize_proxy_writer_queue_size(0), 10_000);
    assert_eq!(sanitize_proxy_writer_queue_size(1), 100);
    assert_eq!(sanitize_proxy_writer_queue_size(500_000), 100_000);
}

#[test]
fn retention_and_storage_limits_are_clamped() {
    assert_eq!(sanitize_proxy_retention_days(0), 7);
    assert_eq!(sanitize_proxy_retention_days(255), 90);
    assert_eq!(sanitize_proxy_max_storage_mb(0), 500);
    assert_eq!(sanitize_proxy_max_storage_mb(9), 50);
    assert_eq!(sanitize_proxy_max_storage_mb(50_000), 10_000);
}

#[test]
fn day_parsers_validate_values() {
    assert!(parse_day_key("2026-02-21").is_some());
    assert!(parse_day_key("2026-02-30").is_none());
    assert!(parse_day_from_traffic_filename("events-2026-02-21.jsonl").is_some());
    assert!(parse_day_from_traffic_filename("events-bad.jsonl").is_none());
}

#[test]
fn har_entry_uses_proxy_event_fields() {
    let event = ProxyTrafficEvent {
        id: 1,
        started_at_utc: "2026-02-21 10:00:00 UTC".to_string(),
        project_name: "demo".to_string(),
        service_name: "api".to_string(),
        protocol: "http1".to_string(),
        host: "api.demo.localhost".to_string(),
        method: "GET".to_string(),
        path: "/v1/items?token=[redacted]&page=1".to_string(),
        status_code: Some(200),
        stream_id: None,
        grpc_service: None,
        grpc_method: None,
        grpc_status: None,
        grpc_message: None,
        duration_ms: 42,
        request_bytes: 120,
        response_bytes: 256,
        request_header_bytes: 120,
        request_body_bytes: 0,
        response_header_bytes: 64,
        response_body_bytes: 192,
        request_headers: vec![ProxyTrafficHeader {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        response_headers: vec![ProxyTrafficHeader {
            name: "content-type".to_string(),
            value: "application/json".to_string(),
        }],
        request_body_preview: None,
        response_body_preview: Some("{\"ok\":true}".to_string()),
        request_body_truncated: false,
        response_body_truncated: false,
        request_body_binary: false,
        response_body_binary: false,
        error: None,
    };

    let entry = proxy_event_to_har_entry(&event);
    assert_eq!(
        entry["startedDateTime"].as_str(),
        Some("2026-02-21T10:00:00Z")
    );
    assert_eq!(
        entry["request"]["url"].as_str(),
        Some("http://api.demo.localhost/v1/items?token=[redacted]&page=1")
    );
    assert_eq!(entry["response"]["status"].as_u64(), Some(200));
    assert_eq!(
        entry["response"]["content"]["mimeType"].as_str(),
        Some("application/json")
    );
}
