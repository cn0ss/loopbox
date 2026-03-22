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
                proxy_grpc_h2c_stream(request, respond, route).await;
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
) {
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

        let request_end_stream = request.body().is_end_stream();
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

    if result.is_err() {
        respond.send_reset(h2::Reason::INTERNAL_ERROR);
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

    let request_preamble = read_http_preamble(&mut client)?;
    let host =
        parse_request_host(&request_preamble).ok_or_else(|| "Missing Host header.".to_string())?;
    let (method, _path) = parse_request_line(&request_preamble);

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

    if let Err(err) = forward_http_exchange(&mut client, &mut upstream, &request_preamble, &method)
    {
        let _ = write_http_error(
            &mut client,
            "502 Bad Gateway",
            "Upstream request forwarding failed.",
        );
        return Err(err);
    }

    Ok(())
}

fn forward_http_exchange(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    request_preamble: &[u8],
    request_method: &str,
) -> Result<(), String> {
    let request_header_end = header_end_index(request_preamble).unwrap_or(request_preamble.len());
    let request_headers_only = &request_preamble[..request_header_end];
    let request_body_prefetched = &request_preamble[request_header_end..];

    upstream
        .write_all(request_headers_only)
        .map_err(|err| format!("Failed to write request headers to upstream: {err}"))?;

    let request_content_length = parse_content_length_from_headers(request_headers_only);
    let request_chunked = has_chunked_transfer_encoding(request_headers_only);

    if let Some(content_length) = request_content_length {
        copy_fixed_body(
            client,
            upstream,
            request_body_prefetched,
            content_length,
            "client request body",
        )?;
    } else if request_chunked {
        copy_chunked_body(
            client,
            upstream,
            request_body_prefetched,
            "client request body",
        )?;
    }

    let response_preamble =
        read_http_preamble_with_limit(upstream, MAX_RESPONSE_HEADER_BYTES, "upstream response")?;
    let response_header_end =
        header_end_index(&response_preamble).unwrap_or(response_preamble.len());
    let response_headers_only = &response_preamble[..response_header_end];
    let response_body_prefetched = &response_preamble[response_header_end..];

    let status_code = parse_response_status(response_headers_only);
    client
        .write_all(response_headers_only)
        .map_err(|err| format!("Failed to write response headers to client: {err}"))?;

    if is_protocol_upgrade_response(status_code, response_headers_only) {
        if !response_body_prefetched.is_empty() {
            client
                .write_all(response_body_prefetched)
                .map_err(|err| format!("Failed to write upgrade response to client: {err}"))?;
        }
        tunnel_upgraded_connection(client, upstream)?;
        return Ok(());
    }

    let response_content_length = parse_content_length_from_headers(response_headers_only);
    let response_chunked = has_chunked_transfer_encoding(response_headers_only);
    let response_connection_close = has_connection_close_header(response_headers_only);

    if !response_should_not_have_body(status_code, request_method) {
        if let Some(content_length) = response_content_length {
            copy_fixed_body(
                upstream,
                client,
                response_body_prefetched,
                content_length,
                "upstream response body",
            )?;
        } else if response_chunked {
            copy_chunked_body(
                upstream,
                client,
                response_body_prefetched,
                "upstream response body",
            )?;
        } else if !response_body_prefetched.is_empty() || response_connection_close {
            copy_stream(
                upstream,
                client,
                response_body_prefetched,
                response_connection_close,
                "upstream response body",
            )?;
        }
    }

    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}
