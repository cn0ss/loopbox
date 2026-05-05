use super::LoopboxConfig;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const IGNORED_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEnvFile {
    pub path: String,
    pub values: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvMergeResult {
    pub files: Vec<String>,
    pub values: BTreeMap<String, String>,
    pub sources: BTreeMap<String, String>,
    pub overrides: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn read_env_file_content(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("Failed to read {path}: {err}"))
}

pub fn write_env_file_content(path: &str, content: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|err| format!("Failed to write {path}: {err}"))
}

pub fn discover_env_files(project_dir: &str) -> Result<Vec<String>, String> {
    let root = PathBuf::from(project_dir);
    if !root.exists() {
        return Err(format!(
            "Project directory '{}' does not exist.",
            root.display()
        ));
    }

    let files = discover_env_files_in(&root)?;
    Ok(files
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect())
}

pub fn parse_env_file(path: &str) -> Result<ParsedEnvFile, String> {
    let file_path = PathBuf::from(path);
    let content = fs::read_to_string(&file_path)
        .map_err(|err| format!("Failed to read {}: {err}", file_path.display()))?;

    let mut values = BTreeMap::new();
    let mut warnings = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((raw_key, raw_value)) = trimmed.split_once('=') else {
            warnings.push(format!(
                "{}:{} ignored invalid env line '{}'",
                file_path.display(),
                index + 1,
                trimmed
            ));
            continue;
        };

        let key = raw_key.trim();
        if !is_valid_env_key(key) {
            warnings.push(format!(
                "{}:{} ignored invalid env key '{}'",
                file_path.display(),
                index + 1,
                key
            ));
            continue;
        }

        let value = normalize_env_value(raw_value);
        values.insert(key.to_string(), value);
    }

    Ok(ParsedEnvFile {
        path: file_path.to_string_lossy().to_string(),
        values,
        warnings,
    })
}

pub fn merge_service_env(
    config: &LoopboxConfig,
    project_name: &str,
    service_name: &str,
) -> Result<EnvMergeResult, String> {
    let project = config
        .projects
        .get(project_name)
        .ok_or_else(|| format!("Project '{project_name}' not found."))?;
    let service = project
        .services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Service '{service_name}' not found in project '{project_name}'.")
        })?;

    let files = resolve_service_env_files(
        Path::new(&project.dir),
        Path::new(&service.workdir),
        &service.env_files,
    )?;
    let mut merged = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut overrides = Vec::new();
    let mut warnings = Vec::new();
    let mut applied_files = Vec::new();

    for path in files {
        let parsed = parse_env_file(path.to_string_lossy().as_ref())?;
        applied_files.push(parsed.path.clone());
        for (key, value) in parsed.values {
            if let Some(previous_source) = sources.insert(key.clone(), parsed.path.clone()) {
                if previous_source != parsed.path {
                    overrides.push(format!(
                        "Key '{}' overridden: '{}' -> '{}'",
                        key, previous_source, parsed.path
                    ));
                }
            }
            merged.insert(key, value);
        }
        warnings.extend(parsed.warnings);
    }

    Ok(EnvMergeResult {
        files: applied_files,
        values: merged,
        sources,
        overrides,
        warnings,
    })
}

fn resolve_service_env_files(
    project_dir: &Path,
    service_dir: &Path,
    configured_files: &[String],
) -> Result<Vec<PathBuf>, String> {
    // Merge order:
    // 1) If `service.env_files` is configured, resolve each entry relative to
    //    service workdir first, then project root (in order).
    // 2) Otherwise, discover all `.env*` files recursively and apply them lexicographically.
    // Later files override keys from earlier files.
    if configured_files.is_empty() {
        return discover_env_files_in(project_dir);
    }

    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    let root_env = project_dir.join(".env");
    if root_env.exists() {
        let normalized = root_env.to_string_lossy().to_string();
        if seen.insert(normalized) {
            resolved.push(root_env);
        }
    }

    for configured in configured_files {
        let trimmed = configured.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = resolve_configured_env_path(project_dir, service_dir, trimmed);
        if !candidate.exists() {
            continue;
        }
        let normalized = candidate.to_string_lossy().to_string();
        if seen.insert(normalized) {
            resolved.push(candidate);
        }
    }

    let root_env_local = project_dir.join(".env.local");
    if root_env_local.exists() {
        let normalized = root_env_local.to_string_lossy().to_string();
        if seen.insert(normalized) {
            resolved.push(root_env_local);
        }
    }

    Ok(resolved)
}

fn resolve_configured_env_path(
    project_dir: &Path,
    service_dir: &Path,
    configured: &str,
) -> PathBuf {
    let raw = Path::new(configured);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }

    let service_relative = service_dir.join(configured);
    if service_relative.exists() {
        return service_relative;
    }

    project_dir.join(configured)
}

fn discover_env_files_in(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    collect_env_files(root, &mut found)?;
    found.sort();
    Ok(found)
}

fn collect_env_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("Failed to read {}: {err}", dir.display()))?;
    for entry_result in entries {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if IGNORED_DIRS.iter().any(|ignored| *ignored == name) {
                continue;
            }
            let _ = collect_env_files(&path, out);
            continue;
        }

        if is_env_filename(&name) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_env_filename(name: &str) -> bool {
    let lowered = name.to_lowercase();
    if !(lowered == ".env" || lowered.starts_with(".env.")) {
        return false;
    }

    !(lowered.ends_with(".example")
        || lowered.ends_with(".sample")
        || lowered.ends_with(".template")
        || lowered.ends_with(".dist")
        || lowered.ends_with(".bak"))
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn normalize_env_value(raw_value: &str) -> String {
    let trimmed = raw_value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        let without_comment = strip_unquoted_inline_comment(trimmed);
        without_comment.trim().to_string()
    }
}

fn strip_unquoted_inline_comment(raw: &str) -> &str {
    let mut previous_is_whitespace = true;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            previous_is_whitespace = ch.is_ascii_whitespace();
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '#' && previous_is_whitespace {
            return &raw[..index];
        }
        previous_is_whitespace = ch.is_ascii_whitespace();
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopbox::{GlobalConfig, ProjectConfig, ServiceConfig};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = ENV_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "loopbox-env-{}-{nonce}-{sequence}",
            std::process::id()
        ))
    }

    fn path_ends_with(path: &str, suffix: &str) -> bool {
        path.replace('\\', "/").ends_with(suffix)
    }

    #[test]
    fn discover_env_files_finds_nested_env_files() {
        let root = temp_dir();
        let nested = root.join("apps").join("api");
        fs::create_dir_all(&nested).expect("create directories");
        fs::write(root.join(".env"), "ROOT_KEY=root").expect("write root env");
        fs::write(nested.join(".env.local"), "API_KEY=api").expect("write nested env");
        fs::write(root.join(".env.local.example"), "API_KEY=example").expect("write example env");

        let found =
            discover_env_files(root.to_string_lossy().as_ref()).expect("discover env files");
        assert!(found.iter().any(|path| path_ends_with(path, "/.env")));
        assert!(found.iter().any(|path| path_ends_with(path, "/.env.local")));
        assert!(!found
            .iter()
            .any(|path| path_ends_with(path, "/.env.local.example")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_service_env_applies_files_in_order() {
        let root = temp_dir();
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join(".env"), "API_URL=http://one\nSHARED=one\n").expect("write .env");
        fs::write(root.join(".env.local"), "SHARED=two\n").expect("write .env.local");

        let config = LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([(
                "demo".to_string(),
                ProjectConfig {
                    dir: root.to_string_lossy().to_string(),
                    ip: "127.0.0.20".to_string(),
                    services: vec![ServiceConfig {
                        name: "backend".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(8080),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "npm run dev".to_string(),
                        workdir: root.to_string_lossy().to_string(),
                        env_files: vec![".env".to_string(), ".env.local".to_string()],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    }],
                    default_open_service: Some("backend".to_string()),
                    proxy_traffic_capture_enabled: None,
                    proxy_traffic_capture_mode: None,
                    grpc_proto_paths: vec![],
                    proxy_endpoints: vec![],
                },
            )]),
        };

        let merged = merge_service_env(&config, "demo", "backend").expect("merge env");
        assert_eq!(
            merged.values.get("API_URL").map(String::as_str),
            Some("http://one")
        );
        assert_eq!(merged.values.get("SHARED").map(String::as_str), Some("two"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_service_env_resolves_relative_env_files_from_service_workdir() {
        let root = temp_dir();
        let api_dir = root.join("apps").join("api");
        fs::create_dir_all(&api_dir).expect("create dirs");
        fs::write(
            api_dir.join(".env"),
            "WORKOS_CLIENT_ID=client_from_backend\n",
        )
        .expect("write backend .env");

        let config = LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([(
                "demo".to_string(),
                ProjectConfig {
                    dir: root.to_string_lossy().to_string(),
                    ip: "127.0.0.20".to_string(),
                    services: vec![ServiceConfig {
                        name: "backend".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(8080),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "go run ./...".to_string(),
                        workdir: api_dir.to_string_lossy().to_string(),
                        env_files: vec![".env".to_string()],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    }],
                    default_open_service: Some("backend".to_string()),
                    proxy_traffic_capture_enabled: None,
                    proxy_traffic_capture_mode: None,
                    grpc_proto_paths: vec![],
                    proxy_endpoints: vec![],
                },
            )]),
        };

        let merged = merge_service_env(&config, "demo", "backend").expect("merge env");
        assert_eq!(
            merged.values.get("WORKOS_CLIENT_ID").map(String::as_str),
            Some("client_from_backend")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_env_file_strips_inline_comments_for_unquoted_values() {
        let root = temp_dir();
        fs::create_dir_all(&root).expect("create root");
        let path = root.join(".env.local");
        fs::write(
            &path,
            "CONVEX_DEPLOYMENT=dev:disciplined-woodpecker-511 # team: niklas, project: vereinsapp\n",
        )
        .expect("write env");

        let parsed = parse_env_file(path.to_string_lossy().as_ref()).expect("parse env");
        assert_eq!(
            parsed.values.get("CONVEX_DEPLOYMENT").map(String::as_str),
            Some("dev:disciplined-woodpecker-511")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_env_file_accepts_leading_whitespace_around_assignment() {
        let root = temp_dir();
        fs::create_dir_all(&root).expect("create root");
        let path = root.join(".env.local");
        fs::write(
            &path,
            "  CONVEX_DEPLOYMENT = dev:disciplined-woodpecker-511  \n",
        )
        .expect("write env");

        let parsed = parse_env_file(path.to_string_lossy().as_ref()).expect("parse env");
        assert_eq!(
            parsed.values.get("CONVEX_DEPLOYMENT").map(String::as_str),
            Some("dev:disciplined-woodpecker-511")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_service_env_combines_root_and_service_specific_env_files() {
        let root = temp_dir();
        let api_dir = root.join("apps").join("api");
        fs::create_dir_all(&api_dir).expect("create dirs");
        fs::write(root.join(".env"), "SHARED=from_root_env\nBASE=1\n").expect("write root .env");
        fs::write(
            api_dir.join(".env.service"),
            "SHARED=from_service\nAPI_ONLY=yes\n",
        )
        .expect("write service env");
        fs::write(root.join(".env.local"), "SHARED=from_root_local\n").expect("write root local");

        let config = LoopboxConfig {
            global: GlobalConfig::default(),
            projects: BTreeMap::from([(
                "demo".to_string(),
                ProjectConfig {
                    dir: root.to_string_lossy().to_string(),
                    ip: "127.0.0.20".to_string(),
                    services: vec![ServiceConfig {
                        name: "backend".to_string(),
                        runtime: crate::loopbox::ServiceRuntimeKind::Process,
                        container: None,
                        ports: vec![],
                        port: Some(8080),
                        protocol: crate::loopbox::ProxyEndpointProtocol::Http1,
                        command: "npm run dev".to_string(),
                        workdir: api_dir.to_string_lossy().to_string(),
                        env_files: vec!["apps/api/.env.service".to_string()],
                        depends_on: vec![],
                        autostart: false,
                        health_path: None,
                    }],
                    default_open_service: Some("backend".to_string()),
                    proxy_traffic_capture_enabled: None,
                    proxy_traffic_capture_mode: None,
                    grpc_proto_paths: vec![],
                    proxy_endpoints: vec![],
                },
            )]),
        };

        let merged = merge_service_env(&config, "demo", "backend").expect("merge env");
        assert_eq!(
            merged.values.get("SHARED").map(String::as_str),
            Some("from_root_local")
        );
        assert_eq!(merged.values.get("BASE").map(String::as_str), Some("1"));
        assert_eq!(
            merged.values.get("API_ONLY").map(String::as_str),
            Some("yes")
        );
        assert!(merged
            .sources
            .get("SHARED")
            .is_some_and(|source| path_ends_with(source, "/.env.local")));
        assert!(!merged.overrides.is_empty());

        let _ = fs::remove_dir_all(&root);
    }
}
