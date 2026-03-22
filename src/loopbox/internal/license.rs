use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildChannel {
    Community,
    Commercial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LicenseTier {
    None,
    Commercial,
}

#[derive(Debug, Clone, Copy)]
struct LicenseState {
    tier: LicenseTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredLicense {
    key: String,
    #[serde(default)]
    activation_id: Option<String>,
    #[serde(default)]
    customer_id: Option<String>,
}

impl Default for LicenseState {
    fn default() -> Self {
        Self {
            tier: LicenseTier::None,
        }
    }
}

static LICENSE_STATE: OnceLock<RwLock<LicenseState>> = OnceLock::new();
static LICENSE_REVALIDATION_STARTED: AtomicBool = AtomicBool::new(false);

const LICENSE_REVALIDATE_INTERVAL_DEFAULT_SECS: u64 = 6 * 60 * 60;
const LICENSE_REVALIDATE_INTERVAL_MIN_SECS: u64 = 60 * 60;
const LICENSE_REVALIDATE_INTERVAL_MAX_SECS: u64 = 24 * 60 * 60;

fn config_path() -> PathBuf {
    if let Some(xdg_config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config_home)
            .join("loopbox")
            .join("config.toml");
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("loopbox")
            .join("config.toml");
    }

    PathBuf::from(".loopbox").join("config.toml")
}

fn license_state() -> &'static RwLock<LicenseState> {
    LICENSE_STATE.get_or_init(|| RwLock::new(LicenseState::default()))
}

pub fn build_channel() -> BuildChannel {
    BuildChannel::Commercial
}

pub fn license_activation_available() -> bool {
    build_channel() == BuildChannel::Commercial && license_verifier_available()
}

pub fn init_license_state_at_startup() -> Result<(), String> {
    if build_channel() == BuildChannel::Community {
        return set_current_license_tier(LicenseTier::None);
    }
    if !license_activation_available() {
        // Paid-enabled binary without Polar configuration: run unlicensed.
        return set_current_license_tier(LicenseTier::None);
    }

    if let Some(raw_env_key) = std::env::var_os("LOOPBOX_LICENSE_KEY") {
        let key = raw_env_key.to_string_lossy().trim().to_string();
        if key.is_empty() {
            return set_current_license_tier(LicenseTier::None);
        }
        let activation_id = std::env::var("LOOPBOX_LICENSE_ACTIVATION_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let tier = match resolve_license_tier_from_provider(&key, activation_id.as_deref()) {
            Ok(tier) => tier,
            Err(reason) => {
                set_current_license_tier(LicenseTier::None)?;
                return Err(format!(
                    "LOOPBOX_LICENSE_KEY could not be verified: {reason}"
                ));
            }
        };
        return set_current_license_tier(tier);
    }

    let stored_license = match load_persisted_license()? {
        Some(stored) => stored,
        None => return set_current_license_tier(LicenseTier::None),
    };
    if stored_license.key.trim().is_empty() {
        return set_current_license_tier(LicenseTier::None);
    }
    let tier = match resolve_license_tier_from_provider(
        stored_license.key.trim(),
        stored_license.activation_id.as_deref(),
    ) {
        Ok(tier) => tier,
        Err(reason) => {
            set_current_license_tier(LicenseTier::None)?;
            return Err(format!(
                "Persisted license in {} could not be verified: {reason}",
                license_key_path().display()
            ));
        }
    };
    set_current_license_tier(tier)
}

pub fn start_periodic_license_revalidation() -> Result<(), String> {
    if build_channel() != BuildChannel::Commercial {
        return Ok(());
    }
    if !license_activation_available() || license_revalidation_disabled() {
        return Ok(());
    }
    if LICENSE_REVALIDATION_STARTED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let interval_secs = revalidation_interval_secs();
    let spawn_result = thread::Builder::new()
        .name("loopbox-license-revalidation".to_string())
        .spawn(move || loop {
            thread::sleep(Duration::from_secs(interval_secs));
            if let Err(err) = init_license_state_at_startup() {
                eprintln!("Loopbox license periodic revalidation warning: {err}");
            }
        });

    match spawn_result {
        Ok(_) => Ok(()),
        Err(err) => {
            LICENSE_REVALIDATION_STARTED.store(false, Ordering::SeqCst);
            Err(format!(
                "Failed to start periodic license revalidation worker: {err}"
            ))
        }
    }
}

#[allow(dead_code)]
pub fn activate_license_key(key: &str) -> Result<(), String> {
    if build_channel() == BuildChannel::Community {
        return Err("License activation is not available in Community build.".to_string());
    }

    let normalized = key.trim();
    if normalized.is_empty() {
        return Err("License key cannot be empty.".to_string());
    }

    let activated = activate_license_with_provider(normalized)?;
    persist_license(&StoredLicense {
        key: normalized.to_string(),
        activation_id: activated.activation_id.clone(),
        customer_id: activated.customer_id.clone(),
    })?;
    set_current_license_tier(activated.tier)
}

pub fn current_license_tier() -> LicenseTier {
    license_state()
        .read()
        .map(|state| state.tier)
        .unwrap_or(LicenseTier::None)
}

#[allow(dead_code)]
pub fn license_status_label() -> String {
    match (
        build_channel(),
        license_activation_available(),
        current_license_tier(),
    ) {
        (BuildChannel::Community, _, _) => "community".to_string(),
        (BuildChannel::Commercial, false, _) => "paid-no-polar-config".to_string(),
        (BuildChannel::Commercial, true, LicenseTier::None) => "unlicensed".to_string(),
        (BuildChannel::Commercial, true, LicenseTier::Commercial) => "commercial".to_string(),
    }
}

pub fn active_license_identity_for_support() -> Option<(String, Option<String>)> {
    let env_activation_id = license_activation_id_from_env();

    if let Some(raw_env_customer_id) = std::env::var_os("LOOPBOX_LICENSE_CUSTOMER_ID") {
        let customer_id = raw_env_customer_id.to_string_lossy().trim().to_string();
        if !customer_id.is_empty() {
            return Some((customer_id, env_activation_id));
        }
    }

    if let Some(raw_env_key) = std::env::var_os("LOOPBOX_LICENSE_KEY") {
        let env_key = raw_env_key.to_string_lossy().trim().to_string();
        if !env_key.is_empty() {
            if let Some(customer_id) =
                resolve_customer_id_from_provider(&env_key, env_activation_id.as_deref())
            {
                return Some((customer_id, env_activation_id));
            }
        }
    }

    let mut stored = load_persisted_license().ok().flatten()?;
    if let Some(customer_id) = stored
        .customer_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        return Some((customer_id, stored.activation_id));
    }

    let customer_id =
        resolve_customer_id_from_provider(stored.key.as_str(), stored.activation_id.as_deref())?;
    stored.customer_id = Some(customer_id.clone());
    let _ = persist_license(&stored);
    Some((customer_id, stored.activation_id))
}

fn set_current_license_tier(tier: LicenseTier) -> Result<(), String> {
    let mut state = license_state()
        .write()
        .map_err(|_| "License state lock poisoned.".to_string())?;
    state.tier = tier;
    Ok(())
}

#[allow(dead_code)]
fn persist_license(stored: &StoredLicense) -> Result<(), String> {
    let path = license_key_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let serialized = serde_json::to_string(stored)
        .map_err(|err| format!("Failed to serialize persisted license: {err}"))?;
    fs::write(&path, serialized).map_err(|err| {
        format!(
            "Failed to write persisted license to {}: {err}",
            path.display()
        )
    })
}

fn license_key_path() -> PathBuf {
    if let Some(override_file) = optional_env("LOOPBOX_LICENSE_FILE") {
        return PathBuf::from(override_file);
    }

    let config_file = config_path();
    let base_dir = config_file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".loopbox"));
    base_dir.join("license.key")
}

fn load_persisted_license() -> Result<Option<StoredLicense>, String> {
    let path = license_key_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|err| {
        format!(
            "Failed to read persisted license from {}: {err}",
            path.display()
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if let Ok(stored) = serde_json::from_str::<StoredLicense>(trimmed) {
        let key = stored.key.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let activation_id = stored
            .activation_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let customer_id = stored
            .customer_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        return Ok(Some(StoredLicense {
            key: key.to_string(),
            activation_id,
            customer_id,
        }));
    }
    Err(format!(
        "Persisted license in {} uses an unsupported legacy format. Re-activate the license with this app version.",
        path.display()
    ))
}

fn resolve_license_tier_from_provider(
    key: &str,
    activation_id: Option<&str>,
) -> Result<LicenseTier, String> {
    if !license_activation_available() {
        return Err(
            "Polar licensing is not configured for this paid-enabled build. Set LOOPBOX_POLAR_* environment variables."
                .to_string(),
        );
    }

    resolve_license_tier_from_key(key, activation_id)
}

fn activate_license_with_provider(key: &str) -> Result<LicenseActivation, String> {
    if !license_activation_available() {
        return Err(
            "Polar licensing is not configured for this paid-enabled build. Set LOOPBOX_POLAR_* environment variables."
                .to_string(),
        );
    }
    activate_license_key_with_provider(key)
}

fn license_activation_id_from_env() -> Option<String> {
    std::env::var("LOOPBOX_LICENSE_ACTIVATION_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_customer_id_from_provider(key: &str, activation_id: Option<&str>) -> Option<String> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    let config = PolarConfig::from_env().ok()?;
    let validation = validate_key_with_polar(&config, key, activation_id).ok()?;
    validation.customer_id
}

fn license_revalidation_disabled() -> bool {
    parse_bool_env("LOOPBOX_DISABLE_LICENSE_REVALIDATION", false)
}

fn revalidation_interval_secs() -> u64 {
    let raw = std::env::var("LOOPBOX_LICENSE_REVALIDATE_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(LICENSE_REVALIDATE_INTERVAL_DEFAULT_SECS);
    raw.clamp(
        LICENSE_REVALIDATE_INTERVAL_MIN_SECS,
        LICENSE_REVALIDATE_INTERVAL_MAX_SECS,
    )
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    let value = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());
    match value.as_deref() {
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        Some(_) => default,
        None => default,
    }
}

#[allow(dead_code)]
fn license_tier_label(tier: LicenseTier) -> &'static str {
    match tier {
        LicenseTier::None => "none",
        LicenseTier::Commercial => "commercial",
    }
}

// EE provider integration (Polar)

#[derive(Debug, Clone)]
pub struct LicenseActivation {
    pub tier: LicenseTier,
    pub activation_id: Option<String>,
    pub customer_id: Option<String>,
}

pub fn license_verifier_available() -> bool {
    PolarConfig::from_env().is_ok()
}

pub fn activate_license_key_with_provider(key: &str) -> Result<LicenseActivation, String> {
    let config = PolarConfig::from_env()?;
    if config.use_activations {
        let activation_id = activate_key_with_polar(&config, key)?;
        let validation = validate_key_with_polar(&config, key, Some(activation_id.as_str()))?;
        return Ok(LicenseActivation {
            tier: validation.tier,
            activation_id: Some(activation_id),
            customer_id: validation.customer_id,
        });
    }

    let validation = validate_key_with_polar(&config, key, None)?;
    Ok(LicenseActivation {
        tier: validation.tier,
        activation_id: None,
        customer_id: validation.customer_id,
    })
}

pub fn resolve_license_tier_from_key(
    key: &str,
    activation_id: Option<&str>,
) -> Result<LicenseTier, String> {
    let config = PolarConfig::from_env()?;
    let validation = validate_key_with_polar(&config, key, activation_id)?;
    Ok(validation.tier)
}

const POLAR_API_BASE_URL_DEFAULT: &str = "https://api.polar.sh";
const POLAR_TIMEOUT_SECS: u64 = 8;
const LOOPBOX_DEVICE_FINGERPRINT_CONDITION_KEY: &str = "loopbox_device_fingerprint";
const LOOPBOX_DEVICE_FINGERPRINT_NAMESPACE: &str = "loopbox-license-device-v1";

#[derive(Debug, Clone)]
struct PolarConfig {
    api_base_url: String,
    organization_id: String,
    pro_benefit_id: String,
    ultimate_benefit_id: Option<String>,
    use_activations: bool,
    activation_label: String,
}

impl PolarConfig {
    fn from_env() -> Result<Self, String> {
        let organization_id = required_polar_env("LOOPBOX_POLAR_ORGANIZATION_ID")?;
        let pro_benefit_id = required_polar_env("LOOPBOX_POLAR_PRO_BENEFIT_ID")?;
        let ultimate_benefit_id = optional_polar_env("LOOPBOX_POLAR_ULTIMATE_BENEFIT_ID");
        let api_base_url = optional_polar_env("LOOPBOX_POLAR_API_BASE_URL")
            .unwrap_or_else(|| POLAR_API_BASE_URL_DEFAULT.to_string());
        let use_activations = parse_polar_bool_env("LOOPBOX_POLAR_USE_ACTIVATIONS", false);
        let activation_label = optional_polar_env("LOOPBOX_POLAR_ACTIVATION_LABEL")
            .unwrap_or_else(default_activation_label);

        Ok(Self {
            api_base_url,
            organization_id,
            pro_benefit_id,
            ultimate_benefit_id,
            use_activations,
            activation_label,
        })
    }
}

#[derive(Debug, Serialize)]
struct PolarValidateRequest<'a> {
    key: &'a str,
    organization_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conditions: Option<Value>,
}

#[derive(Debug, Serialize)]
struct PolarActivateRequest<'a> {
    key: &'a str,
    organization_id: &'a str,
    label: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    conditions: Option<Value>,
}

struct LicenseValidation {
    tier: LicenseTier,
    customer_id: Option<String>,
}

fn validate_key_with_polar(
    config: &PolarConfig,
    key: &str,
    activation_id: Option<&str>,
) -> Result<LicenseValidation, String> {
    let conditions = license_conditions()?;
    let endpoint = format!(
        "{}/v1/customer-portal/license-keys/validate",
        config.api_base_url
    );
    let payload = PolarValidateRequest {
        key,
        organization_id: &config.organization_id,
        activation_id: activation_id.and_then(non_empty),
        conditions,
    };
    let response = polar_post_json(&endpoint, &payload)?;

    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if status != "granted" {
        let fallback = "not granted";
        return Err(format!(
            "Polar license status is {} (expected {}).",
            if status.is_empty() { fallback } else { &status },
            "granted"
        ));
    }

    let benefit_id = response
        .get("benefit_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Polar validate response missing benefit_id.".to_string())?;

    let tier = tier_from_benefit_id(config, benefit_id).ok_or_else(|| {
        format!(
            "License key is valid, but benefit_id {} is not mapped to a Loopbox tier.",
            benefit_id
        )
    })?;

    let customer_id = response
        .get("customer_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            response
                .get("customer")
                .and_then(Value::as_object)
                .and_then(|customer| customer.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    Ok(LicenseValidation { tier, customer_id })
}

fn activate_key_with_polar(config: &PolarConfig, key: &str) -> Result<String, String> {
    let conditions = license_conditions()?;
    let endpoint = format!(
        "{}/v1/customer-portal/license-keys/activate",
        config.api_base_url
    );
    let payload = PolarActivateRequest {
        key,
        organization_id: &config.organization_id,
        label: &config.activation_label,
        conditions,
    };
    let response = polar_post_json(&endpoint, &payload)?;

    response
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            response
                .get("activation")
                .and_then(|entry| entry.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| "Polar activate response missing activation id.".to_string())
}

fn polar_post_json(endpoint: &str, payload: &impl Serialize) -> Result<Value, String> {
    let endpoint = endpoint.to_string();
    let payload = serde_json::to_value(payload)
        .map_err(|err| format!("Failed to serialize Polar request payload: {err}"))?;

    // reqwest::blocking manages an internal Tokio runtime. If dropped from an async
    // context (for example a UI event handler), it can panic. Run the blocking HTTP
    // path on a dedicated worker thread to keep runtime lifecycle outside async contexts.
    let join_handle = std::thread::Builder::new()
        .name("loopbox-polar-http".to_string())
        .spawn(move || polar_post_json_blocking(&endpoint, payload))
        .map_err(|err| format!("Failed to start Polar HTTP worker: {err}"))?;

    join_handle
        .join()
        .map_err(|_| "Polar HTTP worker panicked.".to_string())?
}

fn polar_post_json_blocking(endpoint: &str, payload: Value) -> Result<Value, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(POLAR_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("Failed to initialize Polar HTTP client: {err}"))?;

    let response = client
        .post(endpoint)
        .header(USER_AGENT, format!("loopbox/{}", env!("CARGO_PKG_VERSION")))
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .map_err(|err| format!("Failed to call Polar API ({endpoint}): {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|err| format!("Failed to read Polar response body: {err}"))?;

    let parsed_json = serde_json::from_str::<Value>(&body).ok();
    if !status.is_success() {
        let detail = polar_error_message(parsed_json.as_ref(), &body);
        return Err(format!(
            "Polar API request failed with status {}: {}",
            status, detail
        ));
    }

    parsed_json.ok_or_else(|| "Polar API response was not valid JSON.".to_string())
}

fn polar_error_message(json: Option<&Value>, body: &str) -> String {
    if let Some(json) = json {
        if let Some(detail) = json.get("detail").and_then(Value::as_str) {
            let trimmed = detail.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(error) = json.get("error").and_then(Value::as_str) {
            let trimmed = error.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(message) = json.get("message").and_then(Value::as_str) {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    let compact = body.trim();
    if compact.is_empty() {
        "no additional details".to_string()
    } else {
        compact.to_string()
    }
}

fn tier_from_benefit_id(config: &PolarConfig, benefit_id: &str) -> Option<LicenseTier> {
    if config
        .ultimate_benefit_id
        .as_deref()
        .is_some_and(|id| id == benefit_id)
    {
        return Some(LicenseTier::Commercial);
    }
    if config.pro_benefit_id == benefit_id {
        return Some(LicenseTier::Commercial);
    }
    None
}

fn required_polar_env(name: &str) -> Result<String, String> {
    optional_polar_env(name).ok_or_else(|| {
        format!("Missing required Polar config `{name}`. Set it as runtime env or at build-time.")
    })
}

fn optional_polar_env(name: &str) -> Option<String> {
    optional_env(name).or_else(|| build_time_polar_env(name))
}

fn build_time_polar_env(name: &str) -> Option<String> {
    let raw = match name {
        "LOOPBOX_POLAR_ORGANIZATION_ID" => option_env!("LOOPBOX_POLAR_ORGANIZATION_ID"),
        "LOOPBOX_POLAR_PRO_BENEFIT_ID" => option_env!("LOOPBOX_POLAR_PRO_BENEFIT_ID"),
        "LOOPBOX_POLAR_ULTIMATE_BENEFIT_ID" => option_env!("LOOPBOX_POLAR_ULTIMATE_BENEFIT_ID"),
        "LOOPBOX_POLAR_API_BASE_URL" => option_env!("LOOPBOX_POLAR_API_BASE_URL"),
        "LOOPBOX_POLAR_USE_ACTIVATIONS" => option_env!("LOOPBOX_POLAR_USE_ACTIVATIONS"),
        "LOOPBOX_POLAR_ACTIVATION_LABEL" => option_env!("LOOPBOX_POLAR_ACTIVATION_LABEL"),
        "LOOPBOX_POLAR_REQUIRE_DEVICE_BINDING" => {
            option_env!("LOOPBOX_POLAR_REQUIRE_DEVICE_BINDING")
        }
        _ => None,
    };
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_polar_bool_env(name: &str, default: bool) -> bool {
    let Some(value) = optional_polar_env(name) else {
        return default;
    };

    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn license_conditions() -> Result<Option<Value>, String> {
    if !parse_polar_bool_env("LOOPBOX_POLAR_REQUIRE_DEVICE_BINDING", true) {
        return Ok(None);
    }

    let raw_device_id = resolve_device_identifier().ok_or_else(|| {
        "Unable to derive a stable device identifier for Polar activation binding. Set LOOPBOX_LICENSE_DEVICE_ID to an explicit value or disable strict binding with LOOPBOX_POLAR_REQUIRE_DEVICE_BINDING=false.".to_string()
    })?;
    let fingerprint = hash_device_identifier(&raw_device_id);
    Ok(Some(serde_json::json!({
        LOOPBOX_DEVICE_FINGERPRINT_CONDITION_KEY: fingerprint
    })))
}

fn hash_device_identifier(raw_device_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(LOOPBOX_DEVICE_FINGERPRINT_NAMESPACE.as_bytes());
    hasher.update(b":");
    hasher.update(raw_device_id.as_bytes());
    let digest = hasher.finalize();
    bytes_to_hex_lower(&digest)
}

fn bytes_to_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn resolve_device_identifier() -> Option<String> {
    crate::platform::device::resolve_device_identifier()
}

fn default_activation_label() -> String {
    let host = optional_env("HOSTNAME")
        .or_else(|| optional_env("COMPUTERNAME"))
        .unwrap_or_else(|| "device".to_string());
    let user = optional_env("USER")
        .or_else(|| optional_env("USERNAME"))
        .unwrap_or_else(|| "user".to_string());
    format!("loopbox-{user}@{host}")
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
