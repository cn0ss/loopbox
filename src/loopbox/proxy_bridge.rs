use super::LoopboxConfig;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReverseProxyStatus {
    pub running: bool,
    pub bind_port: u16,
    pub using_fallback_port: bool,
    pub note: Option<String>,
    pub listener_count: usize,
    pub endpoint_listener_count: usize,
    pub source: String,
    pub last_error: Option<String>,
}

impl Default for ReverseProxyStatus {
    fn default() -> Self {
        Self {
            running: false,
            bind_port: 0,
            using_fallback_port: false,
            note: None,
            listener_count: 0,
            endpoint_listener_count: 0,
            source: "none".to_string(),
            last_error: None,
        }
    }
}

fn convert<T, U>(value: T) -> Result<U, String>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|err| format!("Failed to serialize proxy bridge payload: {err}"))?;
    serde_json::from_value(json)
        .map_err(|err| format!("Failed to decode proxy bridge payload: {err}"))
}

fn map_internal_status(
    status: super::internal::proxy_runtime::ReverseProxyStatus,
) -> ReverseProxyStatus {
    ReverseProxyStatus {
        running: status.running,
        bind_port: status.bind_port,
        using_fallback_port: status.using_fallback_port,
        note: status.note,
        listener_count: status.listener_count,
        endpoint_listener_count: status.endpoint_listener_count,
        source: status.source,
        last_error: status.last_error,
    }
}

pub(crate) fn override_enabled() -> bool {
    super::internal::proxy_runtime::override_enabled()
}

pub(crate) fn sync_reverse_proxy(config: &LoopboxConfig) -> Result<ReverseProxyStatus, String> {
    let cfg = convert::<_, super::internal::proxy_runtime::LoopboxConfig>(config)?;
    let status = super::internal::proxy_runtime::sync_reverse_proxy(&cfg)?;
    Ok(map_internal_status(status))
}

pub(crate) fn reverse_proxy_status() -> ReverseProxyStatus {
    map_internal_status(super::internal::proxy_runtime::reverse_proxy_status())
}

pub(crate) fn effective_reverse_proxy_status(config: &LoopboxConfig) -> ReverseProxyStatus {
    let cfg = match convert::<_, super::internal::proxy_runtime::LoopboxConfig>(config) {
        Ok(cfg) => cfg,
        Err(err) => {
            return ReverseProxyStatus {
                last_error: Some(err),
                ..ReverseProxyStatus::default()
            };
        }
    };
    map_internal_status(super::internal::proxy_runtime::effective_reverse_proxy_status(&cfg))
}

pub(crate) fn reverse_proxy_url_for_host(host: &str) -> Option<String> {
    super::internal::proxy_runtime::reverse_proxy_url_for_host(host)
}

pub(crate) fn effective_reverse_proxy_url_for_host(
    config: &LoopboxConfig,
    host: &str,
) -> Option<String> {
    let cfg = convert::<_, super::internal::proxy_runtime::LoopboxConfig>(config).ok()?;
    super::internal::proxy_runtime::effective_reverse_proxy_url_for_host(&cfg, host)
}

pub(crate) fn record_reverse_proxy_sidecar_status(
    status: &super::proxy::ReverseProxyStatus,
    last_error: Option<String>,
) -> Result<(), String> {
    let status = super::internal::proxy_runtime::ReverseProxyStatus {
        running: status.running,
        bind_port: status.bind_port,
        using_fallback_port: status.using_fallback_port,
        note: status.note.clone(),
        listener_count: status.listener_count,
        endpoint_listener_count: status.endpoint_listener_count,
        source: status.source.clone(),
        last_error: status.last_error.clone(),
    };
    super::internal::proxy_runtime::record_reverse_proxy_sidecar_status(&status, last_error)
}

pub(crate) fn clear_reverse_proxy_sidecar_status() -> Result<(), String> {
    super::internal::proxy_runtime::clear_reverse_proxy_sidecar_status()
}
