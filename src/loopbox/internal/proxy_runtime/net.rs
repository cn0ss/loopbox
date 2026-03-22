use super::*;

pub(super) fn header_end_index(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

pub(super) fn strip_host_port(host: &str) -> String {
    if host.starts_with('[') {
        return host
            .split(']')
            .next()
            .unwrap_or(host)
            .trim_start_matches('[')
            .to_string();
    }
    host.split(':').next().unwrap_or(host).trim().to_string()
}

pub(super) fn resolve_socket_addr(ip: &str, port: u16) -> Result<SocketAddr, String> {
    (ip, port)
        .to_socket_addrs()
        .map_err(|err| format!("Failed to resolve upstream address {ip}:{port}: {err}"))?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| format!("No IPv4 upstream address resolved for {ip}:{port}."))
}

pub(super) fn write_http_error(
    stream: &mut TcpStream,
    status: &str,
    message: &str,
) -> Result<(), String> {
    let body = format!("{message}\n");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| format!("Failed to write proxy error response: {err}"))
}
