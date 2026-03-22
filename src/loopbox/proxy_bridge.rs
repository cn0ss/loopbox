use super::LoopboxConfig;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ReverseProxyStatus {
    pub running: bool,
    pub bind_port: u16,
    pub using_fallback_port: bool,
    pub note: Option<String>,
    pub listener_count: usize,
    pub endpoint_listener_count: usize,
}

fn convert<T, U>(value: T) -> Result<U, String>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|err| format!("Failed to serialize EE proxy payload: {err}"))?;
    serde_json::from_value(json).map_err(|err| format!("Failed to decode EE proxy payload: {err}"))
}

fn map_private_status(
    status: super::internal::proxy_runtime::ReverseProxyStatus,
) -> ReverseProxyStatus {
    ReverseProxyStatus {
        running: status.running,
        bind_port: status.bind_port,
        using_fallback_port: status.using_fallback_port,
        note: status.note,
        listener_count: status.listener_count,
        endpoint_listener_count: status.endpoint_listener_count,
    }
}

pub(crate) fn override_enabled() -> bool {
    super::internal::proxy_runtime::override_enabled()
}

pub(crate) fn sync_reverse_proxy(config: &LoopboxConfig) -> Result<ReverseProxyStatus, String> {
    let cfg = convert::<_, super::internal::proxy_runtime::LoopboxConfig>(config)?;
    let status = super::internal::proxy_runtime::sync_reverse_proxy(&cfg)?;
    Ok(map_private_status(status))
}

pub(crate) fn reverse_proxy_status() -> ReverseProxyStatus {
    map_private_status(super::internal::proxy_runtime::reverse_proxy_status())
}

pub(crate) fn reverse_proxy_url_for_host(host: &str) -> Option<String> {
    super::internal::proxy_runtime::reverse_proxy_url_for_host(host)
}
