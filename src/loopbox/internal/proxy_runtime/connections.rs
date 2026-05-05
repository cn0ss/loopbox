use super::*;

pub(super) fn handle_endpoint_proxy_connection(
    mut client: TcpStream,
    listener_key: &ProxyEndpointKey,
    endpoint_routes: Arc<RwLock<HashMap<ProxyEndpointKey, Vec<ProxyEndpointRoute>>>>,
) -> Result<(), String> {
    if !is_loopback_peer(&client) {
        let _ = client.shutdown(Shutdown::Both);
        return Ok(());
    }

    let route_set = {
        let route_guard = endpoint_routes
            .read()
            .map_err(|_| "Proxy endpoint route lock poisoned.".to_string())?;
        route_guard.get(listener_key).cloned()
    };

    let Some(route_set) = route_set else {
        let _ = client.shutdown(Shutdown::Both);
        return Err(format!(
            "No proxy endpoint route configured for {}:{}.",
            listener_key.listen_host, listener_key.listen_port
        ));
    };
    if route_set.is_empty() {
        let _ = client.shutdown(Shutdown::Both);
        return Err(format!(
            "No proxy endpoint route configured for {}:{}.",
            listener_key.listen_host, listener_key.listen_port
        ));
    }

    let protocol = route_set[0].protocol.clone();
    if route_set.iter().any(|route| route.protocol != protocol) {
        let _ = client.shutdown(Shutdown::Both);
        return Err(format!(
            "Mixed endpoint protocols configured for {}:{}.",
            listener_key.listen_host, listener_key.listen_port
        ));
    }

    client
        .set_nodelay(true)
        .map_err(|err| format!("Failed to set endpoint client nodelay: {err}"))?;

    match protocol {
        ProxyEndpointProtocol::GrpcH2c => {
            let runtime = proxy_async_runtime()?;
            return runtime.block_on(proxy_grpc_h2c_connection(client, route_set));
        }
        ProxyEndpointProtocol::Http1 | ProxyEndpointProtocol::TcpPassthrough => {
            if route_set.len() > 1 {
                let _ = client.shutdown(Shutdown::Both);
                return Err(format!(
                    "Multiple endpoint routes on {}:{} require protocol grpc_h2c with authority matching.",
                    listener_key.listen_host, listener_key.listen_port
                ));
            }
            let route = route_set
                .into_iter()
                .next()
                .ok_or_else(|| "Missing endpoint route.".to_string())?;
            let target = resolve_socket_addr(&route.upstream_host, route.upstream_port)?;
            let mut upstream = TcpStream::connect_timeout(&target, Duration::from_secs(2))
                .map_err(|err| format!("Failed to connect endpoint upstream {target}: {err}"))?;
            upstream
                .set_nodelay(true)
                .map_err(|err| format!("Failed to set endpoint upstream nodelay: {err}"))?;
            tunnel_upgraded_connection(&mut client, &mut upstream)?;
        }
    }

    Ok(())
}

async fn proxy_grpc_h2c_connection(
    client: TcpStream,
    route_set: Vec<ProxyEndpointRoute>,
) -> Result<(), String> {
    client
        .set_nonblocking(true)
        .map_err(|err| format!("Failed to configure gRPC client socket: {err}"))?;
    let client_io = tokio::net::TcpStream::from_std(client)
        .map_err(|err| format!("Failed to convert gRPC client socket: {err}"))?;

    let mut server = h2::server::handshake(client_io)
        .await
        .map_err(|err| format!("gRPC server handshake failed: {err}"))?;
    let routes = Arc::new(route_set);

    while let Some(accepted) = server.accept().await {
        let (request, mut respond) =
            accepted.map_err(|err| format!("Failed to accept gRPC stream: {err}"))?;
        let routes = routes.clone();
        tokio::spawn(async move {
            let authority = grpc_request_authority(&request);
            let selected_route =
                select_grpc_route_for_authority(&routes, authority.as_deref()).cloned();
            if let Some(route) = selected_route {
                proxy_grpc_h2c_stream(request, respond, route, authority).await;
            } else {
                respond.send_reset(h2::Reason::REFUSED_STREAM);
            }
        });
    }

    Ok(())
}

async fn proxy_grpc_h2c_stream(
    request: HttpRequest<h2::RecvStream>,
    mut respond: h2::server::SendResponse<Bytes>,
    route: ProxyEndpointRoute,
    request_authority: Option<String>,
) {
    let started_at = SystemTime::now();
    let started = Instant::now();

    let stream_id = Some(request.body().stream_id().as_u32());
    let request_end_stream = request.body().is_end_stream();
    let raw_path = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let path = redact_path_query(&raw_path, &route.redacted_query_keys);
    let (grpc_service, grpc_method) = parse_grpc_service_method(&raw_path);

    let capture_headers = route.capture_mode != ProxyCaptureMode::Metadata;
    let capture_body_preview = route.capture_mode == ProxyCaptureMode::BodyPreview;

    let mut request_headers = if capture_headers {
        header_map_to_redacted_headers(request.headers(), &route.redacted_header_names)
    } else {
        Vec::new()
    };
    let mut response_headers = Vec::new();

    let mut request_header_bytes = estimate_http2_header_bytes(request.headers());
    let mut request_body_bytes = 0_u64;
    let mut response_header_bytes = 0_u64;
    let mut response_body_bytes = 0_u64;
    let mut status_code = None;
    let mut grpc_status = None;
    let mut grpc_message = None;
    let mut request_preview_capture = capture_body_preview.then(|| {
        // gRPC payloads are protobuf-framed binary. Always capture a raw preview so
        // operators can inspect bytes even when global text-only capture is enabled.
        PreviewCapture::new(route.request_body_preview_max_bytes, false)
    });
    let mut response_preview_capture = capture_body_preview.then(|| {
        // Same rationale as request preview above for downstream gRPC payloads.
        PreviewCapture::new(route.response_body_preview_max_bytes, false)
    });
    let mut captured_error = None;

    let result: Result<(), String> = async {
        let target = resolve_socket_addr(&route.upstream_host, route.upstream_port)?;
        let upstream = tokio::net::TcpStream::connect(target)
            .await
            .map_err(|err| format!("Failed to connect gRPC upstream {target}: {err}"))?;
        upstream
            .set_nodelay(true)
            .map_err(|err| format!("Failed to configure gRPC upstream socket: {err}"))?;
        let (upstream_sender, upstream_connection) = h2::client::handshake(upstream)
            .await
            .map_err(|err| format!("gRPC upstream handshake failed: {err}"))?;
        tokio::spawn(async move {
            let _ = upstream_connection.await;
        });

        let (request_parts, mut request_body_stream) = request.into_parts();
        let mut upstream_request_builder = HttpRequest::builder()
            .method(request_parts.method.clone())
            .uri(request_parts.uri.clone())
            .version(Version::HTTP_2);
        for (name, value) in &request_parts.headers {
            upstream_request_builder = upstream_request_builder.header(name, value);
        }
        let upstream_request = upstream_request_builder
            .body(())
            .map_err(|err| format!("Failed to build upstream gRPC request: {err}"))?;

        let mut sender_ready = upstream_sender
            .ready()
            .await
            .map_err(|err| format!("gRPC upstream sender not ready: {err}"))?;
        let (response_future, mut upstream_stream) = sender_ready
            .send_request(upstream_request, request_end_stream)
            .map_err(|err| format!("Failed to send gRPC request headers upstream: {err}"))?;

        if !request_end_stream {
            while let Some(data_result) = request_body_stream.data().await {
                let data = data_result
                    .map_err(|err| format!("Failed reading gRPC request body: {err}"))?;
                let data_len = data.len();
                request_body_bytes = request_body_bytes.saturating_add(data_len as u64);
                if let Some(capture) = request_preview_capture.as_mut() {
                    capture.ingest(&data);
                }
                let _ = request_body_stream
                    .flow_control()
                    .release_capacity(data_len);
                upstream_stream
                    .send_data(data, false)
                    .map_err(|err| format!("Failed forwarding gRPC request body: {err}"))?;
            }

            if let Some(trailers) = request_body_stream
                .trailers()
                .await
                .map_err(|err| format!("Failed reading gRPC request trailers: {err}"))?
            {
                request_header_bytes =
                    request_header_bytes.saturating_add(estimate_http2_header_bytes(&trailers));
                if capture_headers {
                    request_headers.extend(header_map_to_redacted_headers(
                        &trailers,
                        &route.redacted_header_names,
                    ));
                }
                upstream_stream
                    .send_trailers(trailers)
                    .map_err(|err| format!("Failed forwarding gRPC request trailers: {err}"))?;
            } else {
                upstream_stream
                    .send_data(Bytes::new(), true)
                    .map_err(|err| format!("Failed closing gRPC request stream: {err}"))?;
            }
        }

        let response = response_future
            .await
            .map_err(|err| format!("Failed receiving gRPC upstream response: {err}"))?;
        let (response_parts, mut response_body_stream) = response.into_parts();

        status_code = Some(response_parts.status.as_u16());
        grpc_status = parse_grpc_status(&response_parts.headers);
        grpc_message = parse_grpc_message(&response_parts.headers);
        response_header_bytes = response_header_bytes
            .saturating_add(estimate_http2_header_bytes(&response_parts.headers));
        if capture_headers {
            response_headers = header_map_to_redacted_headers(
                &response_parts.headers,
                &route.redacted_header_names,
            );
        }

        let response_end_stream = response_body_stream.is_end_stream();
        let mut downstream_response_builder = HttpResponse::builder()
            .status(response_parts.status)
            .version(Version::HTTP_2);
        for (name, value) in &response_parts.headers {
            downstream_response_builder = downstream_response_builder.header(name, value);
        }
        let downstream_response = downstream_response_builder
            .body(())
            .map_err(|err| format!("Failed to build gRPC downstream response: {err}"))?;

        let mut downstream_stream = respond
            .send_response(downstream_response, response_end_stream)
            .map_err(|err| format!("Failed sending gRPC response headers: {err}"))?;

        if !response_end_stream {
            while let Some(data_result) = response_body_stream.data().await {
                let data = data_result
                    .map_err(|err| format!("Failed reading gRPC response body: {err}"))?;
                let data_len = data.len();
                response_body_bytes = response_body_bytes.saturating_add(data_len as u64);
                if let Some(capture) = response_preview_capture.as_mut() {
                    capture.ingest(&data);
                }
                let _ = response_body_stream
                    .flow_control()
                    .release_capacity(data_len);
                downstream_stream
                    .send_data(data, false)
                    .map_err(|err| format!("Failed forwarding gRPC response body: {err}"))?;
            }

            if let Some(trailers) = response_body_stream
                .trailers()
                .await
                .map_err(|err| format!("Failed reading gRPC response trailers: {err}"))?
            {
                response_header_bytes =
                    response_header_bytes.saturating_add(estimate_http2_header_bytes(&trailers));
                grpc_status = parse_grpc_status(&trailers);
                grpc_message = parse_grpc_message(&trailers);
                if capture_headers {
                    response_headers.extend(header_map_to_redacted_headers(
                        &trailers,
                        &route.redacted_header_names,
                    ));
                }
                downstream_stream
                    .send_trailers(trailers)
                    .map_err(|err| format!("Failed forwarding gRPC response trailers: {err}"))?;
            } else {
                downstream_stream
                    .send_data(Bytes::new(), true)
                    .map_err(|err| format!("Failed closing gRPC response stream: {err}"))?;
            }
        }

        Ok(())
    }
    .await;

    if let Err(err) = result {
        captured_error = Some(err);
        respond.send_reset(h2::Reason::INTERNAL_ERROR);
    }

    if grpc_status.is_some_and(|status| status != 0) && captured_error.is_none() {
        let mut message = format!("gRPC status {}", grpc_status.unwrap_or_default());
        if let Some(detail) = grpc_message.clone() {
            if !detail.trim().is_empty() {
                message.push_str(": ");
                message.push_str(detail.trim());
            }
        }
        captured_error = Some(message);
    }

    let current_grpc_proto_paths =
        current_project_grpc_proto_paths(&route.project_name, &route.grpc_proto_paths);
    let request_body = finalize_grpc_optional_preview(
        request_preview_capture,
        &current_grpc_proto_paths,
        grpc_service.as_deref(),
        grpc_method.as_deref(),
        true,
    );
    let response_body = finalize_grpc_optional_preview(
        response_preview_capture,
        &current_grpc_proto_paths,
        grpc_service.as_deref(),
        grpc_method.as_deref(),
        false,
    );

    if route.capture_enabled {
        let request_bytes = request_header_bytes.saturating_add(request_body_bytes);
        let response_bytes = response_header_bytes.saturating_add(response_body_bytes);
        let host = request_authority
            .unwrap_or_else(|| format!("{}:{}", route.upstream_host, route.upstream_port));
        push_proxy_traffic_event(ProxyTrafficEvent {
            id: 0,
            started_at_utc: format_system_time_utc(started_at),
            project_name: route.project_name,
            service_name: route.service_name,
            protocol: "grpc_h2c".to_string(),
            host,
            method: "GRPC".to_string(),
            path,
            status_code,
            stream_id,
            grpc_service,
            grpc_method,
            grpc_status,
            grpc_message,
            duration_ms: started.elapsed().as_millis() as u64,
            request_bytes,
            response_bytes,
            request_header_bytes,
            request_body_bytes,
            response_header_bytes,
            response_body_bytes,
            request_headers,
            response_headers,
            request_body_preview: request_body.preview,
            response_body_preview: response_body.preview,
            request_body_truncated: request_body.truncated,
            response_body_truncated: response_body.truncated,
            request_body_binary: request_body.binary,
            response_body_binary: response_body.binary,
            error: captured_error,
        });
    }
}

pub(super) fn handle_proxy_connection(
    mut client: TcpStream,
    routes: Arc<RwLock<HashMap<String, ProxyRoute>>>,
) -> Result<(), String> {
    if !is_loopback_peer(&client) {
        write_http_error(
            &mut client,
            "403 Forbidden",
            "Loopbox reverse proxy only accepts loopback clients.",
        )?;
        return Ok(());
    }

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| format!("Failed to configure client read timeout: {err}"))?;

    let started_at = SystemTime::now();
    let started = Instant::now();
    let request_preamble = read_http_preamble(&mut client)?;
    let host =
        parse_request_host(&request_preamble).ok_or_else(|| "Missing Host header.".to_string())?;
    let (method, raw_path) = parse_request_line(&request_preamble);

    let route = {
        let route_guard = routes
            .read()
            .map_err(|_| "Reverse proxy route lock poisoned.".to_string())?;
        route_guard.get(&host).cloned()
    };

    let Some(route) = route else {
        write_http_error(
            &mut client,
            "502 Bad Gateway",
            &format!("No route configured for host '{host}'."),
        )?;
        return Ok(());
    };
    let capture_headers = route.capture_mode != ProxyCaptureMode::Metadata;
    let capture_body_preview = route.capture_mode == ProxyCaptureMode::BodyPreview;
    let path = redact_path_query(&raw_path, &route.redacted_query_keys);
    let request_headers = if capture_headers {
        parse_and_redact_headers(&request_preamble, &route.redacted_header_names)
    } else {
        Vec::new()
    };

    let target = resolve_socket_addr(&route.target_ip, route.target_port)?;
    let mut upstream = TcpStream::connect_timeout(&target, Duration::from_secs(2))
        .map_err(|err| format!("Failed to connect upstream {target}: {err}"))?;
    upstream
        .set_nodelay(true)
        .map_err(|err| format!("Failed to set upstream nodelay: {err}"))?;
    client
        .set_nodelay(true)
        .map_err(|err| format!("Failed to set client nodelay: {err}"))?;
    client
        .set_read_timeout(None)
        .map_err(|err| format!("Failed to clear client read timeout: {err}"))?;

    let forwarding = forward_http_exchange(
        &mut client,
        &mut upstream,
        &request_preamble,
        &method,
        capture_headers,
        capture_body_preview,
        route.capture_text_only,
        &route.redacted_header_names,
        route.request_body_preview_max_bytes,
        route.response_body_preview_max_bytes,
    );
    let (
        status_code,
        request_bytes,
        response_bytes,
        request_header_bytes,
        request_body_bytes,
        response_header_bytes,
        response_body_bytes,
        response_headers,
        request_body_preview,
        response_body_preview,
        request_body_truncated,
        response_body_truncated,
        request_body_binary,
        response_body_binary,
        error,
    ) = match forwarding {
        Ok(metrics) => (
            metrics.status_code,
            metrics.request_bytes,
            metrics.response_bytes,
            metrics.request_header_bytes,
            metrics.request_body_bytes,
            metrics.response_header_bytes,
            metrics.response_body_bytes,
            metrics.response_headers,
            metrics.request_body.preview,
            metrics.response_body.preview,
            metrics.request_body.truncated,
            metrics.response_body.truncated,
            metrics.request_body.binary,
            metrics.response_body.binary,
            None,
        ),
        Err(err) => {
            let request_header_bytes =
                header_end_index(&request_preamble).unwrap_or(request_preamble.len()) as u64;
            let request_body_bytes = request_preamble
                .len()
                .saturating_sub(request_header_bytes as usize)
                as u64;
            let _ = write_http_error(
                &mut client,
                "502 Bad Gateway",
                "Upstream request forwarding failed.",
            );
            (
                None,
                request_preamble.len() as u64,
                0,
                request_header_bytes,
                request_body_bytes,
                0,
                0,
                Vec::new(),
                None,
                None,
                false,
                false,
                false,
                false,
                Some(err),
            )
        }
    };

    if route.capture_enabled {
        push_proxy_traffic_event(ProxyTrafficEvent {
            id: 0,
            started_at_utc: format_system_time_utc(started_at),
            project_name: route.project_name,
            service_name: route.service_name,
            protocol: "http1".to_string(),
            host,
            method,
            path,
            status_code,
            stream_id: None,
            grpc_service: None,
            grpc_method: None,
            grpc_status: None,
            grpc_message: None,
            duration_ms: started.elapsed().as_millis() as u64,
            request_bytes,
            response_bytes,
            request_header_bytes,
            request_body_bytes,
            response_header_bytes,
            response_body_bytes,
            request_headers,
            response_headers,
            request_body_preview,
            response_body_preview,
            request_body_truncated,
            response_body_truncated,
            request_body_binary,
            response_body_binary,
            error: error.clone(),
        });
    }

    if let Some(err) = error {
        return Err(err);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn forward_http_exchange(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    request_preamble: &[u8],
    request_method: &str,
    capture_headers: bool,
    capture_body_preview: bool,
    capture_text_only: bool,
    redacted_header_names: &[String],
    request_body_preview_max_bytes: usize,
    response_body_preview_max_bytes: usize,
) -> Result<ForwardMetrics, String> {
    let request_header_end = header_end_index(request_preamble).unwrap_or(request_preamble.len());
    let request_headers_only = &request_preamble[..request_header_end];
    let request_body_prefetched = &request_preamble[request_header_end..];
    let mut request_preview = capture_body_preview
        .then(|| PreviewCapture::new(request_body_preview_max_bytes, capture_text_only));
    upstream
        .write_all(request_headers_only)
        .map_err(|err| format!("Failed to write request headers to upstream: {err}"))?;

    let request_header_bytes = request_headers_only.len() as u64;
    let mut request_body_bytes = 0_u64;
    let request_content_length = parse_content_length_from_headers(request_headers_only);
    let request_chunked = has_chunked_transfer_encoding(request_headers_only);

    if let Some(content_length) = request_content_length {
        let body_bytes = copy_fixed_body_with_optional_preview(
            client,
            upstream,
            request_body_prefetched,
            content_length,
            request_preview.as_mut(),
            "client request body",
        )?;
        request_body_bytes = request_body_bytes.saturating_add(body_bytes);
    } else if request_chunked {
        let body_bytes = copy_chunked_body_with_optional_preview(
            client,
            upstream,
            request_body_prefetched,
            request_preview.as_mut(),
            "client request body",
        )?;
        request_body_bytes = request_body_bytes.saturating_add(body_bytes);
    }
    // Do not half-close upstream here: some dev servers treat early FIN as
    // aborted request and close without sending a response.
    let request_body = finalize_optional_preview(request_preview);

    let response_preamble =
        read_http_preamble_with_limit(upstream, MAX_RESPONSE_HEADER_BYTES, "upstream response")?;
    let response_header_end =
        header_end_index(&response_preamble).unwrap_or(response_preamble.len());
    let response_headers_only = &response_preamble[..response_header_end];
    let response_body_prefetched = &response_preamble[response_header_end..];

    let status_code = parse_response_status(response_headers_only);
    let response_headers = if capture_headers {
        parse_and_redact_headers(response_headers_only, redacted_header_names)
    } else {
        Vec::new()
    };
    let mut response_preview = capture_body_preview
        .then(|| PreviewCapture::new(response_body_preview_max_bytes, capture_text_only));
    client
        .write_all(response_headers_only)
        .map_err(|err| format!("Failed to write response headers to client: {err}"))?;

    let response_header_bytes = response_headers_only.len() as u64;
    let mut response_body_bytes = 0_u64;
    if is_protocol_upgrade_response(status_code, response_headers_only) {
        if !response_body_prefetched.is_empty() {
            client
                .write_all(response_body_prefetched)
                .map_err(|err| format!("Failed to write upgrade response to client: {err}"))?;
            response_body_bytes =
                response_body_bytes.saturating_add(response_body_prefetched.len() as u64);
        }
        tunnel_upgraded_connection(client, upstream)?;
        let response_body = finalize_optional_preview(response_preview);
        let request_bytes = request_header_bytes.saturating_add(request_body_bytes);
        let response_bytes = response_header_bytes.saturating_add(response_body_bytes);
        return Ok(ForwardMetrics {
            status_code,
            request_bytes,
            response_bytes,
            request_header_bytes,
            request_body_bytes,
            response_header_bytes,
            response_body_bytes,
            response_headers,
            request_body,
            response_body,
        });
    }

    let response_content_length = parse_content_length_from_headers(response_headers_only);
    let response_chunked = has_chunked_transfer_encoding(response_headers_only);
    let response_connection_close = has_connection_close_header(response_headers_only);

    if !response_should_not_have_body(status_code, request_method) {
        if let Some(content_length) = response_content_length {
            let body_bytes = copy_fixed_body_with_optional_preview(
                upstream,
                client,
                response_body_prefetched,
                content_length,
                response_preview.as_mut(),
                "upstream response body",
            )?;
            response_body_bytes = response_body_bytes.saturating_add(body_bytes);
        } else if response_chunked {
            let body_bytes = copy_chunked_body_with_optional_preview(
                upstream,
                client,
                response_body_prefetched,
                response_preview.as_mut(),
                "upstream response body",
            )?;
            response_body_bytes = response_body_bytes.saturating_add(body_bytes);
        } else if !response_body_prefetched.is_empty() || response_connection_close {
            // Preserve prefetched body bytes for unframed responses and only
            // continue reading until EOF when the upstream declares connection close.
            let body_bytes = copy_stream_with_optional_preview(
                upstream,
                client,
                response_body_prefetched,
                response_connection_close,
                response_preview.as_mut(),
                "upstream response body",
            )?;
            response_body_bytes = response_body_bytes.saturating_add(body_bytes);
        }
    }
    let _ = client.shutdown(Shutdown::Write);
    let response_body = finalize_optional_preview(response_preview);
    let request_bytes = request_header_bytes.saturating_add(request_body_bytes);
    let response_bytes = response_header_bytes.saturating_add(response_body_bytes);

    Ok(ForwardMetrics {
        status_code,
        request_bytes,
        response_bytes,
        request_header_bytes,
        request_body_bytes,
        response_header_bytes,
        response_body_bytes,
        response_headers,
        request_body,
        response_body,
    })
}
