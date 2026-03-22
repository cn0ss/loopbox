pub(super) fn is_sensitive_header_name(name: &str, redacted_header_names: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    redacted_header_names
        .iter()
        .any(|sensitive| lower == *sensitive)
}

pub(super) fn is_sensitive_query_key(key: &str, redacted_query_keys: &[String]) -> bool {
    let lower = key.to_ascii_lowercase();
    redacted_query_keys
        .iter()
        .any(|sensitive| lower == *sensitive || lower.contains(sensitive))
}

pub(super) fn sanitize_redaction_list(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut sanitized = Vec::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            sanitized.push(normalized);
        }
    }
    sanitized
}

pub(super) fn truncate_for_capture(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut truncated = input.chars().take(max_chars).collect::<String>();
    truncated.push_str("...(truncated)");
    truncated
}
