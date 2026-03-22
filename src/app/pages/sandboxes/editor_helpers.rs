use super::*;

pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn highlight_env_line(line: &str) -> String {
    let trimmed = line.trim_start();

    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.starts_with('#') {
        return format!("<span class=\"syn-comment\">{}</span>", html_escape(line));
    }

    if let Some(eq_pos) = trimmed.find('=') {
        let leading_ws = &line[..line.len() - trimmed.len()];
        let key_raw = &trimmed[..eq_pos];
        let val_raw = &trimmed[eq_pos + 1..];

        let mut out = String::new();
        out.push_str(&html_escape(leading_ws));

        if let Some(stripped) = key_raw.strip_prefix("export ") {
            out.push_str("<span class=\"syn-export\">export </span>");
            out.push_str(&format!(
                "<span class=\"syn-key\">{}</span>",
                html_escape(stripped)
            ));
        } else {
            out.push_str(&format!(
                "<span class=\"syn-key\">{}</span>",
                html_escape(key_raw)
            ));
        }

        out.push_str("<span class=\"syn-eq\">=</span>");

        let vt = val_raw.trim();
        if (vt.starts_with('"') && vt.ends_with('"') && vt.len() > 1)
            || (vt.starts_with('\'') && vt.ends_with('\'') && vt.len() > 1)
        {
            out.push_str(&format!(
                "<span class=\"syn-string\">{}</span>",
                html_escape(val_raw)
            ));
        } else {
            out.push_str(&format!(
                "<span class=\"syn-val\">{}</span>",
                html_escape(val_raw)
            ));
        }

        return out;
    }

    html_escape(line)
}

pub(super) fn highlight_env_content(content: &str) -> String {
    content
        .split('\n')
        .map(highlight_env_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn env_search_match_count(content: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    let q = query.to_lowercase();
    let c = content.to_lowercase();
    let mut count = 0;
    let mut pos = 0;
    while let Some(found) = c[pos..].find(&q) {
        count += 1;
        pos += found + 1;
    }
    count
}

pub(super) fn env_search_match_offset(
    content: &str,
    query: &str,
    index: usize,
) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let q = query.to_lowercase();
    let c = content.to_lowercase();
    let mut i = 0;
    let mut pos = 0;
    while let Some(found) = c[pos..].find(&q) {
        if i == index {
            let start = pos + found;
            return Some((start, start + query.len()));
        }
        i += 1;
        pos += found + 1;
    }
    None
}

// NOTE on document::eval usage below:
// document::eval is the Dioxus-provided API for webview JS interop,
// not JavaScript's eval(). This runs trusted, static JS in the app's own
// desktop webview for scroll sync, tab key handling, and search navigation.
// There is no user-supplied input in these JS strings.

// Helper: run trusted JS snippet in webview via Dioxus document::eval API
pub(super) fn run_webview_js(js: &str) {
    let js = js.to_string();
    spawn(async move {
        let _ = document::eval(&js).await;
    });
}
