use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SUPPORT_TICKET_TIMEOUT_SECS: u64 = 8;
const SUPPORT_TICKET_ENDPOINT_DEV: &str = "http://loopbox-web.loopboxweb.localhost/api/support";
const SUPPORT_TICKET_ENDPOINT_PROD: &str = "https://www.loopbox.tech/api/support";

#[derive(Debug, Serialize)]
struct SupportTicketPayload {
    email: String,
    subject: String,
    text: String,
    app_version: String,
    submitted_at_unix_ms: u128,
}

pub fn submit_support_ticket(
    email: &str,
    subject: &str,
    text: &str,
    app_version: &str,
) -> Result<(), String> {
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

    let payload = SupportTicketPayload {
        email: email.to_string(),
        subject: subject.to_string(),
        text: text.to_string(),
        app_version: app_version.to_string(),
        submitted_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    };
    let payload = serde_json::to_value(payload)
        .map_err(|err| format!("Failed to serialize support ticket payload: {err}"))?;

    let app_version = app_version.to_string();
    let join_handle = std::thread::Builder::new()
        .name("loopbox-support-ticket".to_string())
        .spawn(move || submit_support_ticket_blocking(&endpoint, payload, &app_version))
        .map_err(|err| format!("Failed to start support ticket worker: {err}"))?;

    join_handle
        .join()
        .map_err(|_| "Support ticket worker panicked.".to_string())?
}

fn submit_support_ticket_blocking(
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
