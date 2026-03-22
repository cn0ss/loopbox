use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SUPPORT_TICKET_TIMEOUT_SECS: u64 = 8;
const SUPPORT_TICKET_ENDPOINT_DEV: &str =
    "http://loopbox-web.loopboxweb.localhost/api/support/priority";
const SUPPORT_TICKET_ENDPOINT_PROD: &str = "https://www.loopbox.tech/api/support/priority";

#[derive(Debug, Serialize)]
struct PrioritySupportTicketPayload {
    email: String,
    subject: String,
    text: String,
    tier: String,
    app_version: String,
    submitted_at_unix_ms: u128,
    license_customer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    license_activation_id: Option<String>,
    metadata: PrioritySupportTicketMetadata,
}

#[derive(Debug, Serialize)]
struct PrioritySupportTicketMetadata {
    build_channel: &'static str,
    license_status: String,
}

pub fn submit_priority_support_ticket(
    email: &str,
    subject: &str,
    text: &str,
    app_version: &str,
) -> Result<(), String> {
    if !matches!(
        super::license::current_license_tier(),
        super::license::LicenseTier::Commercial
    ) {
        return Err("Priority support requires a Commercial license.".to_string());
    }

    let email = email.trim();
    let subject = subject.trim();
    let text = text.trim();
    let app_version = app_version.trim();
    if email.is_empty() {
        return Err("Email is required.".to_string());
    }
    if subject.is_empty() {
        return Err("Subject is required.".to_string());
    }
    if text.is_empty() {
        return Err("Text is required.".to_string());
    }
    if app_version.is_empty() {
        return Err("App version is required.".to_string());
    }

    let endpoint = optional_env("LOOPBOX_SUPPORT_TICKET_ENDPOINT")
        .unwrap_or_else(|| default_support_ticket_endpoint().to_string());
    let (license_customer_id, license_activation_id) =
        match super::license::active_license_identity_for_support() {
            Some((customer_id, activation_id)) if !customer_id.trim().is_empty() => {
                (customer_id.trim().to_string(), activation_id)
            }
            _ => {
                return Err(
                    "Could not resolve an active license customer id for priority support submission. Set LOOPBOX_LICENSE_CUSTOMER_ID explicitly or re-activate the license on this machine."
                        .to_string(),
                )
            }
        };
    let tier = match super::license::current_license_tier() {
        super::license::LicenseTier::Commercial => "commercial",
        super::license::LicenseTier::None => "none",
    };
    let build_channel = match super::license::build_channel() {
        super::license::BuildChannel::Community => "community",
        super::license::BuildChannel::Commercial => "commercial",
    };

    let payload = PrioritySupportTicketPayload {
        email: email.to_string(),
        subject: subject.to_string(),
        text: text.to_string(),
        tier: tier.to_string(),
        app_version: app_version.to_string(),
        submitted_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        license_customer_id,
        license_activation_id,
        metadata: PrioritySupportTicketMetadata {
            build_channel,
            license_status: super::license::license_status_label(),
        },
    };
    let payload = serde_json::to_value(payload)
        .map_err(|err| format!("Failed to serialize support ticket payload: {err}"))?;

    let app_version = app_version.to_string();
    let join_handle = std::thread::Builder::new()
        .name("loopbox-support-ticket".to_string())
        .spawn(move || submit_priority_support_ticket_blocking(&endpoint, payload, &app_version))
        .map_err(|err| format!("Failed to start support ticket worker: {err}"))?;

    join_handle
        .join()
        .map_err(|_| "Support ticket worker panicked.".to_string())?
}

fn submit_priority_support_ticket_blocking(
    endpoint: &str,
    payload: Value,
    app_version: &str,
) -> Result<(), String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(SUPPORT_TICKET_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("Failed to initialize support ticket HTTP client: {err}"))?;

    let request = client
        .post(endpoint)
        .header(USER_AGENT, format!("loopbox/{app_version}"))
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json");

    let response = request
        .json(&payload)
        .send()
        .map_err(|err| format!("Failed to submit support ticket ({endpoint}): {err}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|err| format!("Failed to read support ticket response body: {err}"))?;
    if !status.is_success() {
        let detail = support_ticket_error_detail(&body);
        return Err(format!(
            "Support ticket submission failed with status {}: {}",
            status, detail
        ));
    }

    Ok(())
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_support_ticket_endpoint() -> &'static str {
    if cfg!(debug_assertions) {
        SUPPORT_TICKET_ENDPOINT_DEV
    } else {
        SUPPORT_TICKET_ENDPOINT_PROD
    }
}

fn support_ticket_error_detail(body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(body) {
        for key in ["message", "detail", "error"] {
            if let Some(value) = parsed.get(key).and_then(Value::as_str) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }

    let trimmed = body.trim();
    if trimmed.is_empty() {
        "no additional details".to_string()
    } else {
        trimmed.to_string()
    }
}
