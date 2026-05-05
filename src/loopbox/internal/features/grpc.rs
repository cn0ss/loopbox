use super::*;

const MAX_GRPC_PREVIEW_FRAMES: usize = 4;
const MAX_GRPC_HEX_PREVIEW_BYTES: usize = 128;

#[derive(Debug, Clone, Copy)]
pub(super) struct GrpcFrameMeta {
    pub(super) compressed: bool,
    pub(super) declared_len: usize,
    pub(super) complete: bool,
}

pub fn render_grpc_preview(
    bytes: &[u8],
    _proto_paths: &[String],
    _grpc_service: Option<&str>,
    _grpc_method: Option<&str>,
    _is_request: bool,
) -> Option<String> {
    let (frames, trailing_bytes) = split_grpc_frames(bytes);
    if frames.is_empty() {
        return None;
    }

    let catalog = grpc_proto_catalog(_proto_paths);
    let mut blocks = Vec::new();
    for (index, (meta, payload)) in frames
        .iter()
        .copied()
        .take(MAX_GRPC_PREVIEW_FRAMES)
        .enumerate()
    {
        let mut lines = Vec::new();
        let include_header =
            trailing_bytes || index > 0 || payload.len() != meta.declared_len || meta.compressed;
        if include_header {
            lines.push(format!(
                "frame {}: {} bytes{}",
                index + 1,
                meta.declared_len,
                if meta.compressed { " (compressed)" } else { "" }
            ));
        }

        if meta.compressed {
            lines.push("[compressed gRPC payload omitted]".to_string());
            blocks.push(lines.join("\n"));
            continue;
        }

        let typed_decoded = if let (Some(catalog), Some(service), Some(method)) =
            (catalog.as_ref(), _grpc_service, _grpc_method)
        {
            decode_grpc_message_with_catalog(catalog, service, method, _is_request, payload)
        } else {
            None
        };
        let decoded = typed_decoded.or_else(|| decode_grpc_message_raw(payload));

        if let Some(text) = decoded {
            lines.push(text);
        } else if looks_like_text_bytes(payload) {
            lines.push(String::from_utf8_lossy(payload).to_string());
        } else {
            lines.push(format!(
                "[{} bytes protobuf payload]\n{}",
                payload.len(),
                hex_preview(payload, MAX_GRPC_HEX_PREVIEW_BYTES)
            ));
        }

        if !meta.complete {
            lines.push("[incomplete gRPC frame preview]".to_string());
        }
        blocks.push(lines.join("\n"));
    }

    if frames.len() > MAX_GRPC_PREVIEW_FRAMES {
        blocks.push(format!(
            "... {} additional frame(s) omitted ...",
            frames.len().saturating_sub(MAX_GRPC_PREVIEW_FRAMES)
        ));
    }
    if trailing_bytes {
        blocks.push("[trailing bytes without complete gRPC frame header]".to_string());
    }
    Some(blocks.join("\n\n"))
}

pub(super) fn split_grpc_frames(bytes: &[u8]) -> (Vec<(GrpcFrameMeta, &[u8])>, bool) {
    let mut frames = Vec::new();
    let mut offset = 0_usize;
    while offset + 5 <= bytes.len() {
        let compressed = bytes[offset] != 0;
        let declared_len = u32::from_be_bytes([
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
        ]) as usize;
        offset += 5;

        let remaining = bytes.len().saturating_sub(offset);
        let payload_len = remaining.min(declared_len);
        let complete = payload_len == declared_len;
        let payload = &bytes[offset..offset + payload_len];
        frames.push((
            GrpcFrameMeta {
                compressed,
                declared_len,
                complete,
            },
            payload,
        ));
        offset += payload_len;
        if !complete {
            break;
        }
    }
    (frames, offset < bytes.len())
}

#[derive(Debug, Clone)]
struct GrpcProtoMethodBinding {
    service: String,
    method: String,
    request_type: String,
    response_type: String,
    proto_file: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct GrpcProtoCatalog {
    include_dirs: Vec<PathBuf>,
    methods: Vec<GrpcProtoMethodBinding>,
}

static GRPC_PROTO_CATALOG_CACHE: OnceLock<Mutex<HashMap<String, GrpcProtoCatalog>>> =
    OnceLock::new();

fn grpc_proto_catalog(proto_paths: &[String]) -> Option<GrpcProtoCatalog> {
    let normalized = normalize_proto_paths(proto_paths);
    if normalized.is_empty() {
        return None;
    }
    let key = normalized.join("\n");
    let cache = GRPC_PROTO_CATALOG_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&key) {
            return Some(cached.clone());
        }
    }

    let built = build_grpc_proto_catalog(&normalized)?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, built.clone());
    }
    Some(built)
}

fn normalize_proto_paths(proto_paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = proto_paths
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .filter(|value| seen.insert(value.clone()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn build_grpc_proto_catalog(proto_paths: &[String]) -> Option<GrpcProtoCatalog> {
    let (include_dirs, proto_files) = collect_proto_inputs(proto_paths);
    if include_dirs.is_empty() || proto_files.is_empty() {
        return None;
    }

    let mut methods = Vec::new();
    for file in &proto_files {
        methods.extend(parse_grpc_proto_methods(file));
    }
    if methods.is_empty() {
        return None;
    }

    Some(GrpcProtoCatalog {
        include_dirs,
        methods,
    })
}

fn collect_proto_inputs(proto_paths: &[String]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut include_dirs = std::collections::BTreeSet::new();
    let mut proto_files = std::collections::BTreeSet::new();

    for raw in proto_paths {
        let expanded = expand_tilde(raw.trim());
        if expanded.is_dir() {
            include_dirs.insert(expanded.clone());
            collect_proto_files_recursive(&expanded, &mut proto_files);
            continue;
        }
        if expanded.is_file() && expanded.extension().is_some_and(|ext| ext == "proto") {
            if let Some(parent) = expanded.parent() {
                include_dirs.insert(parent.to_path_buf());
            }
            proto_files.insert(expanded);
        }
    }

    (
        include_dirs.into_iter().collect(),
        proto_files.into_iter().collect(),
    )
}

fn expand_tilde(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn collect_proto_files_recursive(dir: &Path, out: &mut std::collections::BTreeSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_proto_files_recursive(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "proto") {
            out.insert(path);
        }
    }
}

fn parse_grpc_proto_methods(file: &Path) -> Vec<GrpcProtoMethodBinding> {
    let Ok(content) = fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut package = String::new();
    let mut current_service: Option<String> = None;
    let mut service_brace_depth = 0_i32;
    let mut rpc_buffer = String::new();
    let mut methods = Vec::new();

    for raw_line in content.lines() {
        let line_no_comment = raw_line
            .split_once("//")
            .map(|(left, _)| left)
            .unwrap_or(raw_line);
        let line = line_no_comment.trim();
        if line.is_empty() {
            continue;
        }

        if package.is_empty() {
            if let Some(pkg) = parse_proto_package(line) {
                package = pkg;
            }
        }

        if current_service.is_none() {
            if let Some(service_name) = parse_proto_service_decl(line) {
                service_brace_depth = brace_delta(line);
                if service_brace_depth <= 0 {
                    service_brace_depth = 1;
                }
                current_service = Some(service_name);
            }
            continue;
        }

        rpc_buffer.push(' ');
        rpc_buffer.push_str(line);
        if line.contains(';') {
            if let Some((rpc_name, request_type, response_type)) = parse_proto_rpc_decl(&rpc_buffer)
            {
                let Some(service_name) = current_service.as_ref() else {
                    rpc_buffer.clear();
                    continue;
                };
                let full_service = if package.is_empty() {
                    service_name.clone()
                } else {
                    format!("{package}.{service_name}")
                };
                let request_type = resolve_proto_type_name(&request_type, &package);
                let response_type = resolve_proto_type_name(&response_type, &package);
                methods.push(GrpcProtoMethodBinding {
                    service: full_service,
                    method: rpc_name,
                    request_type,
                    response_type,
                    proto_file: file.to_path_buf(),
                });
            }
            rpc_buffer.clear();
        }

        service_brace_depth += brace_delta(line);
        if service_brace_depth <= 0 {
            current_service = None;
            rpc_buffer.clear();
            service_brace_depth = 0;
        }
    }

    methods
}

fn parse_proto_package(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("package ")?;
    let value = rest.trim().trim_end_matches(';').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_proto_service_decl(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("service ")?;
    let name = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn parse_proto_rpc_decl(line: &str) -> Option<(String, String, String)> {
    let rpc_pos = line.find("rpc ")?;
    let after_rpc = line.get(rpc_pos + 4..)?.trim_start();

    let method_name = after_rpc
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    if method_name.is_empty() {
        return None;
    }

    let after_method = after_rpc.get(method_name.len()..)?.trim_start();
    let request_start = after_method.find('(')?;
    let request_end = after_method.get(request_start + 1..)?.find(')')? + request_start + 1;
    let request_raw = after_method
        .get(request_start + 1..request_end)?
        .trim()
        .trim_start_matches("stream ")
        .trim()
        .to_string();

    let after_request = after_method.get(request_end + 1..)?;
    let returns_pos = after_request.find("returns")?;
    let after_returns = after_request
        .get(returns_pos + "returns".len()..)?
        .trim_start();
    let response_start = after_returns.find('(')?;
    let response_end = after_returns.get(response_start + 1..)?.find(')')? + response_start + 1;
    let response_raw = after_returns
        .get(response_start + 1..response_end)?
        .trim()
        .trim_start_matches("stream ")
        .trim()
        .to_string();

    if request_raw.is_empty() || response_raw.is_empty() {
        return None;
    }
    Some((method_name, request_raw, response_raw))
}

fn resolve_proto_type_name(raw: &str, package: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if package.is_empty() {
        return trimmed.to_string();
    }
    if trimmed.contains('.') {
        let first_segment = trimmed.split('.').next().unwrap_or_default();
        if first_segment
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            format!("{package}.{trimmed}")
        } else {
            trimmed.to_string()
        }
    } else {
        format!("{package}.{trimmed}")
    }
}

fn brace_delta(line: &str) -> i32 {
    let opens = line.chars().filter(|ch| *ch == '{').count() as i32;
    let closes = line.chars().filter(|ch| *ch == '}').count() as i32;
    opens - closes
}

fn decode_grpc_message_with_catalog(
    catalog: &GrpcProtoCatalog,
    service: &str,
    method: &str,
    is_request: bool,
    payload: &[u8],
) -> Option<String> {
    let binding = catalog.methods.iter().find(|candidate| {
        candidate.method == method && grpc_service_matches(&candidate.service, service)
    })?;
    let message_type = if is_request {
        binding.request_type.trim()
    } else {
        binding.response_type.trim()
    };
    if message_type.is_empty() {
        return None;
    }
    run_protoc_decode(
        message_type,
        &binding.proto_file,
        catalog.include_dirs.as_slice(),
        payload,
    )
}

fn grpc_service_matches(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    left.ends_with(&format!(".{right}")) || right.ends_with(&format!(".{left}"))
}

fn run_protoc_decode(
    message_type: &str,
    proto_file: &Path,
    include_dirs: &[PathBuf],
    payload: &[u8],
) -> Option<String> {
    let mut command = Command::new("protoc");
    for include_dir in include_dirs {
        command.arg("-I").arg(include_dir);
    }
    command
        .arg(format!("--decode={message_type}"))
        .arg(proto_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().ok()?;
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(payload).is_err() {
            return None;
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let decoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if decoded.is_empty() {
        None
    } else {
        Some(beautify_protoc_text_output(&decoded))
    }
}

fn decode_grpc_message_raw(payload: &[u8]) -> Option<String> {
    let mut child = Command::new("protoc")
        .arg("--decode_raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(payload).is_err() {
            return None;
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let decoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if decoded.is_empty() {
        None
    } else {
        Some(beautify_protoc_text_output(&decoded))
    }
}

pub(super) fn beautify_protoc_text_output(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let Some((prefix, inner)) = parse_proto_quoted_field_line(line) else {
            lines.push(line.to_string());
            continue;
        };
        let decoded_inner = decode_proto_escaped_string(inner);
        let trimmed_inner = decoded_inner.trim();
        if looks_like_json_text(trimmed_inner) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed_inner) {
                lines.push(prefix.to_string());
                if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                    for pretty_line in pretty.lines() {
                        lines.push(format!("  {pretty_line}"));
                    }
                } else {
                    lines.push(format!("  {trimmed_inner}"));
                }
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines.join("\n")
}

fn parse_proto_quoted_field_line(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let (left, right) = line.split_at(colon + 1);
    let value = right.trim_start();
    if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
        return None;
    }
    Some((left, &value[1..value.len() - 1]))
}

fn decode_proto_escaped_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(escaped) = chars.next() else {
            break;
        };
        match escaped {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'x' => {
                let hi = chars.next();
                let lo = chars.next();
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    let hex = format!("{hi}{lo}");
                    if let Ok(value) = u8::from_str_radix(&hex, 16) {
                        out.push(value as char);
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn looks_like_json_text(value: &str) -> bool {
    (value.starts_with('{') && value.ends_with('}'))
        || (value.starts_with('[') && value.ends_with(']'))
}

fn hex_preview(bytes: &[u8], max_bytes: usize) -> String {
    let shown = bytes.len().min(max_bytes);
    let mut output = String::new();
    for (index, value) in bytes.iter().take(shown).enumerate() {
        if index > 0 {
            output.push(' ');
        }
        output.push_str(&format!("{value:02x}"));
    }
    if bytes.len() > shown {
        output.push_str(" ...");
    }
    output
}

fn looks_like_text_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if bytes.contains(&0) {
        return false;
    }
    for b in bytes {
        let is_control = matches!(*b, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F);
        if is_control {
            return false;
        }
    }
    true
}
