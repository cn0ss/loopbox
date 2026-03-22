use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;

const GITHUB_OWNER: &str = "cn0ss";
const GITHUB_REPO: &str = "loopbox";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestReleaseInfo {
    pub tag: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReleasePayload {
    tag_name: String,
    html_url: Option<String>,
}

pub fn app_version_label() -> String {
    normalize_release_tag(
        option_env!("LOOPBOX_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
    )
}

pub fn latest_release_page_url() -> String {
    format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest")
}

pub fn is_newer_release_tag(current: &str, candidate: &str) -> bool {
    let current_normalized = normalize_release_tag(current);
    let candidate_normalized = normalize_release_tag(candidate);
    if current_normalized == candidate_normalized {
        return false;
    }

    match (
        parse_semver_like(&current_normalized),
        parse_semver_like(&candidate_normalized),
    ) {
        (Some(current_semver), Some(candidate_semver)) => candidate_semver > current_semver,
        _ => false,
    }
}

pub async fn fetch_latest_github_release() -> Result<LatestReleaseInfo, String> {
    let endpoint =
        format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|err| format!("Failed to initialize release check client: {err}"))?;

    let response = client
        .get(endpoint)
        .header(USER_AGENT, format!("loopbox/{}", env!("CARGO_PKG_VERSION")))
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| format!("Failed to fetch latest GitHub release: {err}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub latest release check failed with status {}.",
            response.status()
        ));
    }

    let payload = response
        .json::<GitHubReleasePayload>()
        .await
        .map_err(|err| format!("Invalid GitHub latest release response: {err}"))?;
    let tag = normalize_release_tag(&payload.tag_name);
    if tag.trim().is_empty() {
        return Err("GitHub latest release payload had an empty tag name.".to_string());
    }

    let url = payload
        .html_url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(latest_release_page_url);

    Ok(LatestReleaseInfo { tag, url })
}

fn normalize_release_tag(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('v') || trimmed.starts_with('V') {
        return format!("v{}", trimmed[1..].trim());
    }
    if trimmed
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
    {
        return format!("v{trimmed}");
    }
    trimmed.to_string()
}

fn parse_semver_like(tag: &str) -> Option<(u64, u64, u64)> {
    let normalized = normalize_release_tag(tag);
    let version = normalized.strip_prefix('v').unwrap_or(normalized.as_str());
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.');

    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts
        .next()
        .map(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(Some(0))?;
    let patch = parts
        .next()
        .map(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{is_newer_release_tag, normalize_release_tag};

    #[test]
    fn normalize_release_tag_keeps_expected_format() {
        assert_eq!(normalize_release_tag("0.1.1"), "v0.1.1");
        assert_eq!(normalize_release_tag("v0.2.0"), "v0.2.0");
        assert_eq!(normalize_release_tag("V1.0.0"), "v1.0.0");
        assert_eq!(
            normalize_release_tag("release-2026-02-21"),
            "release-2026-02-21"
        );
    }

    #[test]
    fn update_check_uses_semver_order() {
        assert!(is_newer_release_tag("v0.1.0", "v0.1.2"));
        assert!(!is_newer_release_tag("v0.1.2", "v0.1.2"));
        assert!(!is_newer_release_tag("v0.1.3", "v0.1.2"));
        assert!(is_newer_release_tag("0.1.2", "0.2.0"));
        assert!(!is_newer_release_tag("release-local", "v0.1.2"));
    }
}
