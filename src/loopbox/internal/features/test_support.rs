#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GrpcFrameMetaForTest {
    pub compressed: bool,
    pub declared_len: usize,
    pub complete: bool,
}

pub(crate) fn split_grpc_frames_for_test(
    bytes: &[u8],
) -> (Vec<(GrpcFrameMetaForTest, Vec<u8>)>, bool) {
    let (frames, trailing) = super::grpc::split_grpc_frames(bytes);
    let converted = frames
        .into_iter()
        .map(|(meta, payload)| {
            (
                GrpcFrameMetaForTest {
                    compressed: meta.compressed,
                    declared_len: meta.declared_len,
                    complete: meta.complete,
                },
                payload.to_vec(),
            )
        })
        .collect::<Vec<_>>();
    (converted, trailing)
}

pub(crate) fn beautify_protoc_text_output_for_test(raw: &str) -> String {
    super::grpc::beautify_protoc_text_output(raw)
}

pub(crate) fn parse_day_key_for_test(day_key: &str) -> Option<i64> {
    parse_day_key_test_impl(day_key)
}

pub(crate) fn parse_day_from_traffic_filename_for_test(name: &str) -> Option<i64> {
    parse_day_from_traffic_filename_test_impl(name)
}

pub(crate) fn proxy_event_to_har_entry_for_test(
    event: &super::super::ProxyTrafficEvent,
) -> serde_json::Value {
    proxy_event_to_har_entry_test_impl(event)
}

fn parse_day_from_traffic_filename_test_impl(name: &str) -> Option<i64> {
    if !name.starts_with("events-") || !name.ends_with(".jsonl") {
        return None;
    }
    let day_key = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
    parse_day_key_test_impl(day_key)
}

fn parse_day_key_test_impl(day_key: &str) -> Option<i64> {
    let mut parts = day_key.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let day = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let day_serial = days_from_civil_test(year, month, day);
    let (check_year, check_month, check_day) = civil_from_days_test(day_serial);
    if (year, month, day) != (check_year, check_month, check_day) {
        return None;
    }
    Some(day_serial)
}

fn civil_from_days_test(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn days_from_civil_test(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn proxy_event_to_har_entry_test_impl(
    event: &super::super::ProxyTrafficEvent,
) -> serde_json::Value {
    let request_url = har_url_for_event_test(event);
    let response_mime = har_mime_type_from_headers_test(event.response_headers.as_slice());
    serde_json::json!({
        "startedDateTime": har_started_at_iso8601_test(&event.started_at_utc),
        "request": {
            "url": request_url,
        },
        "response": {
            "status": event.status_code.unwrap_or(0),
            "content": {
                "mimeType": response_mime,
            }
        }
    })
}

fn har_started_at_iso8601_test(raw: &str) -> String {
    if let Some(stripped) = raw.strip_suffix(" UTC") {
        return format!("{}Z", stripped.replace(' ', "T"));
    }
    raw.to_string()
}

fn har_url_for_event_test(event: &super::super::ProxyTrafficEvent) -> String {
    if event.path.starts_with("http://") || event.path.starts_with("https://") {
        return event.path.clone();
    }
    format!("http://{}{}", event.host, event.path)
}

fn har_mime_type_from_headers_test(headers: &[super::super::ProxyTrafficHeader]) -> String {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
        .map(|header| header.value.clone())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}
