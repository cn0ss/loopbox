use super::*;

pub(super) fn parse_content_length_from_headers(headers: &[u8]) -> Option<usize> {
    header_value_from_headers(headers, "content-length")?
        .split(',')
        .next()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
}

pub(super) fn has_chunked_transfer_encoding(headers: &[u8]) -> bool {
    header_value_from_headers(headers, "transfer-encoding")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
        })
        .unwrap_or(false)
}

pub(super) fn has_connection_close_header(headers: &[u8]) -> bool {
    header_value_from_headers(headers, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("close"))
        })
        .unwrap_or(false)
}

pub(super) fn has_connection_upgrade_header(headers: &[u8]) -> bool {
    header_value_from_headers(headers, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false)
}

pub(super) fn is_protocol_upgrade_response(status_code: Option<u16>, headers: &[u8]) -> bool {
    if status_code == Some(101) {
        return true;
    }
    has_connection_upgrade_header(headers)
        && header_value_from_headers(headers, "upgrade").is_some()
}

pub(super) fn header_value_from_headers(headers: &[u8], name: &str) -> Option<String> {
    let end_idx = header_end_index(headers)?;
    let text = String::from_utf8_lossy(&headers[..end_idx]);
    for raw_line in text.lines().skip(1) {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((raw_name, raw_value)) = line.split_once(':') else {
            continue;
        };
        if raw_name.trim().eq_ignore_ascii_case(name) {
            return Some(raw_value.trim().to_string());
        }
    }
    None
}

pub(super) fn response_should_not_have_body(
    status_code: Option<u16>,
    request_method: &str,
) -> bool {
    if request_method.eq_ignore_ascii_case("HEAD") {
        return true;
    }
    matches!(status_code, Some(code) if (100..200).contains(&code) || code == 204 || code == 304)
}

pub(super) fn parse_request_host(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("host") {
            continue;
        }
        let host = value.trim();
        if host.is_empty() {
            return None;
        }
        let clean = strip_host_port(host).to_lowercase();
        if clean.is_empty() {
            return None;
        }
        return Some(clean);
    }
    None
}

pub(super) fn parse_request_line(bytes: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(bytes);
    let Some(first_line_raw) = text.lines().next() else {
        return ("UNKNOWN".to_string(), "/".to_string());
    };
    let first_line = first_line_raw.trim_end_matches('\r');
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("UNKNOWN").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    (method, path)
}

pub(super) fn redact_path_query(path: &str, redacted_query_keys: &[String]) -> String {
    let Some((base, query)) = path.split_once('?') else {
        return path.to_string();
    };
    if query.is_empty() {
        return format!("{base}?");
    }
    let redacted_query = query
        .split('&')
        .map(|pair| {
            let Some((raw_key, raw_value)) = pair.split_once('=') else {
                return pair.to_string();
            };
            if is_sensitive_query_key(raw_key, redacted_query_keys) {
                format!("{raw_key}={REDACTED_HEADER_VALUE}")
            } else {
                format!("{raw_key}={raw_value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted_query}")
}

pub(super) fn parse_response_status(bytes: &[u8]) -> Option<u16> {
    let text = String::from_utf8_lossy(bytes);
    let first_line = text.lines().next()?.trim_end_matches('\r');
    let mut parts = first_line.split_whitespace();
    let _http = parts.next()?;
    parts.next()?.parse::<u16>().ok()
}

pub(super) fn parse_and_redact_headers(
    bytes: &[u8],
    redacted_header_names: &[String],
) -> Vec<ProxyTrafficHeader> {
    let Some(end_idx) = header_end_index(bytes) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes[..end_idx]);
    let mut headers = Vec::new();
    for raw_line in text.lines().skip(1) {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((raw_name, raw_value)) = line.split_once(':') else {
            continue;
        };
        let name = raw_name.trim();
        if name.is_empty() {
            continue;
        }
        let name_truncated = truncate_for_capture(name, MAX_CAPTURED_HEADER_NAME_LEN);
        let value = raw_value.trim();
        let value_redacted = if is_sensitive_header_name(name, redacted_header_names) {
            REDACTED_HEADER_VALUE.to_string()
        } else {
            truncate_for_capture(value, MAX_CAPTURED_HEADER_VALUE_LEN)
        };
        headers.push(ProxyTrafficHeader {
            name: name_truncated,
            value: value_redacted,
        });
        if headers.len() >= MAX_CAPTURED_HEADERS_PER_EVENT {
            break;
        }
    }
    headers
}

pub(super) fn header_map_to_redacted_headers(
    headers: &HeaderMap,
    redacted_header_names: &[String],
) -> Vec<ProxyTrafficHeader> {
    let mut captured = Vec::new();
    for (name, value) in headers {
        let header_name = name.as_str();
        if header_name.is_empty() {
            continue;
        }
        let name_truncated = truncate_for_capture(header_name, MAX_CAPTURED_HEADER_NAME_LEN);
        let raw_value = String::from_utf8_lossy(value.as_bytes());
        let value_trimmed = raw_value.trim();
        let value_redacted = if is_sensitive_header_name(header_name, redacted_header_names) {
            REDACTED_HEADER_VALUE.to_string()
        } else {
            truncate_for_capture(value_trimmed, MAX_CAPTURED_HEADER_VALUE_LEN)
        };
        captured.push(ProxyTrafficHeader {
            name: name_truncated,
            value: value_redacted,
        });
        if captured.len() >= MAX_CAPTURED_HEADERS_PER_EVENT {
            break;
        }
    }
    captured
}

pub(super) fn estimate_http2_header_bytes(headers: &HeaderMap) -> u64 {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().len() + value.as_bytes().len() + 4) as u64)
        .sum::<u64>()
}

pub(super) fn parse_grpc_service_method(path: &str) -> (Option<String>, Option<String>) {
    let path_only = path.split_once('?').map(|(value, _)| value).unwrap_or(path);
    let mut segments = path_only.trim().trim_start_matches('/').split('/');
    let service = segments
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let method = segments
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (service, method)
}

pub(super) fn parse_grpc_status(headers: &HeaderMap) -> Option<i32> {
    headers
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i32>().ok())
}

pub(super) fn parse_grpc_message(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("grpc-message")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let decoded = decode_percent_escaped_value(raw);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_for_capture(trimmed, MAX_CAPTURED_HEADER_VALUE_LEN))
}
