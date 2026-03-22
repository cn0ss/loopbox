use super::*;

pub(super) fn push_proxy_traffic_event(event: ProxyTrafficEvent) {
    let max_events = proxy_traffic_max_events();
    super::super::features::push_proxy_traffic_event(event, max_events);
}

pub(super) fn proxy_traffic_max_events() -> usize {
    reverse_proxy_state()
        .lock()
        .map(|state| state.proxy_traffic_max_events)
        .unwrap_or(DEFAULT_PROXY_TRAFFIC_MAX_EVENTS)
}

pub(super) fn sanitize_proxy_traffic_limit(limit: usize) -> usize {
    if limit == 0 {
        return DEFAULT_PROXY_TRAFFIC_MAX_EVENTS;
    }
    limit.clamp(MIN_PROXY_TRAFFIC_MAX_EVENTS, MAX_PROXY_TRAFFIC_MAX_EVENTS)
}

pub(super) fn sanitize_proxy_writer_queue_size(limit: usize) -> usize {
    if limit == 0 {
        return DEFAULT_PROXY_TRAFFIC_WRITER_QUEUE_SIZE;
    }
    limit.clamp(
        MIN_PROXY_TRAFFIC_WRITER_QUEUE_SIZE,
        MAX_PROXY_TRAFFIC_WRITER_QUEUE_SIZE,
    )
}

pub(super) fn sanitize_proxy_retention_days(limit: u16) -> u16 {
    if limit == 0 {
        return DEFAULT_PROXY_TRAFFIC_RETENTION_DAYS;
    }
    limit.clamp(
        MIN_PROXY_TRAFFIC_RETENTION_DAYS,
        MAX_PROXY_TRAFFIC_RETENTION_DAYS,
    )
}

pub(super) fn sanitize_proxy_max_storage_mb(limit: usize) -> usize {
    if limit == 0 {
        return DEFAULT_PROXY_TRAFFIC_MAX_STORAGE_MB;
    }
    limit.clamp(
        MIN_PROXY_TRAFFIC_MAX_STORAGE_MB,
        MAX_PROXY_TRAFFIC_MAX_STORAGE_MB,
    )
}

pub(super) fn sanitize_proxy_body_preview_limit(limit: usize, default_limit: usize) -> usize {
    if limit == 0 {
        return default_limit;
    }
    limit.clamp(
        MIN_PROXY_BODY_PREVIEW_MAX_BYTES,
        MAX_PROXY_BODY_PREVIEW_MAX_BYTES,
    )
}
