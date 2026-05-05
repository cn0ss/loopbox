use super::*;

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

pub(super) fn parse_response_status(bytes: &[u8]) -> Option<u16> {
    let text = String::from_utf8_lossy(bytes);
    let first_line = text.lines().next()?.trim_end_matches('\r');
    let mut parts = first_line.split_whitespace();
    let _http = parts.next()?;
    parts.next()?.parse::<u16>().ok()
}

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
