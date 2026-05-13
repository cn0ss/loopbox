use super::*;

pub(super) fn healthcheck_ok(targets: &[String], host: &str, port: u16, health_path: &str) -> bool {
    let request =
        format!("GET {health_path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");

    for target in targets {
        let Some(ip) = parse_ip(target) else {
            continue;
        };
        let addr = SocketAddr::new(ip, port);

        for attempt in 0..=HEALTHCHECK_RETRIES {
            let Ok(mut stream) =
                TcpStream::connect_timeout(&addr, Duration::from_millis(HEALTHCHECK_TIMEOUT_MS))
            else {
                if attempt < HEALTHCHECK_RETRIES {
                    thread::sleep(Duration::from_millis(60));
                }
                continue;
            };

            if stream.write_all(request.as_bytes()).is_err() {
                if attempt < HEALTHCHECK_RETRIES {
                    thread::sleep(Duration::from_millis(60));
                }
                continue;
            }

            let mut first_line = String::new();
            let mut reader = BufReader::new(stream);
            if reader.read_line(&mut first_line).is_err() {
                if attempt < HEALTHCHECK_RETRIES {
                    thread::sleep(Duration::from_millis(60));
                }
                continue;
            }

            let status_code = first_line
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u16>().ok());
            if status_code
                .map(|code| (200..=399).contains(&code))
                .unwrap_or(false)
            {
                return true;
            }

            if attempt < HEALTHCHECK_RETRIES {
                thread::sleep(Duration::from_millis(60));
            }
        }
    }

    false
}

pub(super) fn service_ports_healthy(
    config: &LoopboxConfig,
    project: &ProjectConfig,
    project_name: &str,
    service_name: &str,
    ports: &[ServicePortConfig],
    targets: &[String],
    host: &str,
    health_checks: &mut HashMap<String, CachedHealthCheck>,
) -> bool {
    // Health checks are opt-in per port via health_path.
    // If no configured port has a health target, treat the service as
    // healthy while the process is alive.
    let has_any_health_target = ports.iter().any(port_has_health_target);
    if !has_any_health_target {
        return true;
    }

    for entry in ports {
        if !port_has_health_target(entry) {
            continue;
        }

        if !port_reachable_with_targets(
            entry.port,
            targets,
            HEALTHCHECK_RETRIES,
            HEALTHCHECK_TIMEOUT_MS,
        ) {
            return false;
        }

        let interval_secs = effective_health_check_interval_secs(config, project, entry);
        let cache_key = health_check_cache_key(project_name, service_name, entry, targets, host);
        if let Some(cached) = health_checks.get(&cache_key) {
            let fresh = cached
                .checked_at
                .elapsed()
                .map(|elapsed| elapsed.as_secs() < interval_secs)
                .unwrap_or(false);
            if fresh {
                if !cached.healthy {
                    return false;
                }
                continue;
            }
        }

        if entry.protocol == ProxyEndpointProtocol::Http1 {
            if let Some(health_path) = entry
                .health_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let healthy = healthcheck_ok(targets, host, entry.port, health_path);
                health_checks.insert(
                    cache_key,
                    CachedHealthCheck {
                        checked_at: SystemTime::now(),
                        healthy,
                    },
                );
                if !healthy {
                    return false;
                }
            }
        } else if entry.protocol == ProxyEndpointProtocol::GrpcH2c {
            let grpc_target = entry
                .health_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(target) = grpc_target {
                let healthy = grpc_healthcheck_ok(targets, entry.port, Some(target));
                health_checks.insert(
                    cache_key,
                    CachedHealthCheck {
                        checked_at: SystemTime::now(),
                        healthy,
                    },
                );
                if !healthy {
                    return false;
                }
            }
        }
    }

    true
}

pub(super) fn effective_health_check_interval_secs(
    config: &LoopboxConfig,
    project: &ProjectConfig,
    entry: &ServicePortConfig,
) -> u64 {
    super::super::config::sanitize_health_check_interval_secs(entry.health_check_interval_secs)
        .or_else(|| {
            super::super::config::sanitize_health_check_interval_secs(
                project.health_check_interval_secs,
            )
        })
        .or_else(|| {
            super::super::config::sanitize_health_check_interval_secs(Some(
                config.global.health_check_interval_secs,
            ))
        })
        .unwrap_or_else(default_health_check_interval_secs)
}

fn health_check_cache_key(
    project_name: &str,
    service_name: &str,
    entry: &ServicePortConfig,
    targets: &[String],
    host: &str,
) -> String {
    let health_path = entry
        .health_path
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    format!(
        "{project_name}\n{service_name}\n{host}\n{}\n{:?}\n{health_path}\n{}",
        entry.port,
        entry.protocol,
        targets.join(",")
    )
}

fn port_has_health_target(entry: &ServicePortConfig) -> bool {
    entry
        .health_path
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

pub(super) fn grpc_healthcheck_ok(
    targets: &[String],
    port: u16,
    health_target: Option<&str>,
) -> bool {
    for target in targets {
        let Some(ip) = parse_ip(target) else {
            continue;
        };
        if grpc_healthcheck_ok_target(ip, port, health_target) {
            return true;
        }
    }
    false
}

pub(super) fn grpc_healthcheck_ok_target(
    ip: IpAddr,
    port: u16,
    health_target: Option<&str>,
) -> bool {
    let service_name = grpc_health_service_name(health_target);
    let request_payload = Bytes::from(encode_grpc_health_check_request(service_name.as_deref()));
    let timeout = Duration::from_millis(HEALTHCHECK_TIMEOUT_MS);

    if tokio::runtime::Handle::try_current().is_ok() {
        let worker = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
            else {
                return None;
            };
            Some(runtime.block_on(grpc_healthcheck_ok_target_async(
                ip,
                port,
                request_payload,
                timeout,
            )))
        });
        return matches!(worker.join(), Ok(Some(true)));
    }

    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    else {
        return false;
    };

    runtime.block_on(grpc_healthcheck_ok_target_async(
        ip,
        port,
        request_payload,
        timeout,
    ))
}

pub(super) async fn grpc_healthcheck_ok_target_async(
    ip: IpAddr,
    port: u16,
    request_payload: Bytes,
    timeout: Duration,
) -> bool {
    let addr = SocketAddr::new(ip, port);
    let stream = match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => stream,
        _ => return false,
    };

    let (mut sender, connection) =
        match tokio::time::timeout(timeout, h2::client::handshake(stream)).await {
            Ok(Ok(parts)) => parts,
            _ => return false,
        };
    let connection_task = tokio::spawn(async move {
        let _ = connection.await;
    });

    let request = match Request::builder()
        .method("POST")
        .uri("/grpc.health.v1.Health/Check")
        .version(Version::HTTP_2)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("user-agent", "loopbox-runtime-health")
        .body(())
    {
        Ok(request) => request,
        Err(_) => {
            connection_task.abort();
            return false;
        }
    };

    let (response_future, mut send_stream) = match sender.send_request(request, false) {
        Ok(parts) => parts,
        Err(_) => {
            connection_task.abort();
            return false;
        }
    };

    if send_stream.send_data(request_payload, true).is_err() {
        connection_task.abort();
        return false;
    }

    let response = match tokio::time::timeout(timeout, response_future).await {
        Ok(Ok(response)) => response,
        _ => {
            connection_task.abort();
            return false;
        }
    };

    let header_grpc_status = grpc_status_from_headers(response.headers());
    let mut body = response.into_body();
    let mut response_payload = Vec::new();

    loop {
        match tokio::time::timeout(timeout, body.data()).await {
            Ok(Some(Ok(chunk))) => {
                response_payload.extend_from_slice(&chunk);
                if response_payload.len() > 64 * 1024 {
                    connection_task.abort();
                    return false;
                }
            }
            Ok(Some(Err(_))) => {
                connection_task.abort();
                return false;
            }
            Ok(None) => break,
            Err(_) => {
                connection_task.abort();
                return false;
            }
        }
    }

    let trailer_grpc_status = match tokio::time::timeout(timeout, body.trailers()).await {
        Ok(Ok(Some(trailers))) => grpc_status_from_headers(&trailers),
        Ok(Ok(None)) => None,
        _ => {
            connection_task.abort();
            return false;
        }
    };
    connection_task.abort();

    let grpc_status = trailer_grpc_status.or(header_grpc_status).unwrap_or(0);
    if grpc_status != 0 {
        return false;
    }

    matches!(
        decode_grpc_health_response_status(&response_payload),
        Some(1)
    )
}

pub(super) fn grpc_status_from_headers(headers: &HeaderMap) -> Option<i32> {
    headers
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i32>().ok())
}

pub(super) fn grpc_health_service_name(health_target: Option<&str>) -> Option<String> {
    let raw = health_target?.trim();
    if raw.is_empty() {
        return None;
    }

    let cleaned = raw.trim_start_matches('/').trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

pub(super) fn encode_grpc_health_check_request(service_name: Option<&str>) -> Vec<u8> {
    let mut protobuf = Vec::new();
    if let Some(service) = service_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // field 1 (service), wire type 2
        protobuf.push(0x0A);
        encode_protobuf_varint(service.len() as u64, &mut protobuf);
        protobuf.extend_from_slice(service.as_bytes());
    }

    let mut framed = Vec::with_capacity(5 + protobuf.len());
    framed.push(0);
    framed.extend_from_slice(&(protobuf.len() as u32).to_be_bytes());
    framed.extend_from_slice(&protobuf);
    framed
}

pub(super) fn decode_grpc_health_response_status(payload: &[u8]) -> Option<u64> {
    let mut offset = 0_usize;
    while offset + 5 <= payload.len() {
        let compressed = payload[offset];
        offset += 1;
        let frame_len = u32::from_be_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]) as usize;
        offset += 4;

        if compressed != 0 || offset + frame_len > payload.len() {
            return None;
        }

        let message = &payload[offset..offset + frame_len];
        offset += frame_len;
        if let Some(status) = decode_health_status_from_protobuf(message) {
            return Some(status);
        }
    }
    None
}

pub(super) fn decode_health_status_from_protobuf(message: &[u8]) -> Option<u64> {
    let mut offset = 0_usize;
    while offset < message.len() {
        let key = decode_protobuf_varint(message, &mut offset)?;
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        match (field_number, wire_type) {
            (1, 0) => return decode_protobuf_varint(message, &mut offset),
            (_, 0) => {
                decode_protobuf_varint(message, &mut offset)?;
            }
            (_, 1) => {
                offset = offset.checked_add(8)?;
            }
            (_, 2) => {
                let len = decode_protobuf_varint(message, &mut offset)? as usize;
                offset = offset.checked_add(len)?;
            }
            (_, 5) => {
                offset = offset.checked_add(4)?;
            }
            _ => return None,
        }
        if offset > message.len() {
            return None;
        }
    }
    None
}

pub(super) fn encode_protobuf_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(super) fn decode_protobuf_varint(input: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result = 0_u64;
    let mut shift = 0_u32;
    while *offset < input.len() && shift <= 63 {
        let byte = input[*offset];
        *offset += 1;
        result |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

pub(super) fn port_reachable_with_targets(
    port: u16,
    targets: &[String],
    retries: usize,
    timeout_ms: u64,
) -> bool {
    for target in targets {
        let Some(ip) = parse_ip(target) else {
            continue;
        };
        let addr = SocketAddr::new(ip, port);
        for attempt in 0..=retries {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok() {
                return true;
            }
            if attempt < retries {
                thread::sleep(Duration::from_millis(60));
            }
        }
    }
    false
}

pub(super) fn reachability_targets(bind_ip: &str) -> Vec<String> {
    let cleaned = bind_ip.trim();
    if cleaned.is_empty() {
        vec!["127.0.0.1".to_string()]
    } else {
        vec![cleaned.to_string()]
    }
}

pub(super) fn parse_ip(input: &str) -> Option<IpAddr> {
    input.parse::<IpAddr>().ok().or_else(|| {
        if input == "localhost" {
            "127.0.0.1".parse::<IpAddr>().ok()
        } else {
            None
        }
    })
}
