use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IGNORED_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build"];
const PRIORITIZED_SCRIPTS: &[&str] = &["dev", "start", "serve"];
const COMPOSE_FILE_CANDIDATES: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverySuggestion {
    pub package_name: Option<String>,
    pub script_name: String,
    pub package_manager: String,
    pub origin: String,
    pub command: String,
    pub workdir: String,
    pub confidence: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeDiscovery {
    pub compose_file: String,
    pub services: Vec<ComposeServiceSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeServiceSuggestion {
    pub service_name: String,
    pub image: Option<String>,
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub env_files: Vec<String>,
    pub volumes: Vec<String>,
    pub depends_on: Vec<String>,
    pub ports: Vec<ComposePortSuggestion>,
    pub uses_build: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePortSuggestion {
    pub published_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectBlueprintKind {
    Expo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBlueprintSuggestion {
    pub kind: ProjectBlueprintKind,
    pub package_name: Option<String>,
    pub package_manager: String,
    pub workdir: String,
    pub command: String,
    pub reason: String,
    pub confidence: u16,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    scripts: HashMap<String, String>,
    #[serde(default)]
    dependencies: HashMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: HashMap<String, String>,
}

pub fn discover_project_commands(project_dir: &str) -> Result<Vec<DiscoverySuggestion>, String> {
    let root = PathBuf::from(project_dir);
    if !root.exists() {
        return Err(format!(
            "Project directory '{}' does not exist.",
            root.display()
        ));
    }

    let mut package_files = Vec::new();
    collect_package_json_files(&root, &mut package_files)?;

    let mut suggestions = Vec::new();
    for package_file in package_files {
        let package_dir = package_file
            .parent()
            .unwrap_or_else(|| Path::new(project_dir))
            .to_path_buf();
        let package = read_package_json(&package_file)?;
        let workspace_root = detect_workspace_root(&package_dir, &root);
        let (manager, origin) = detect_package_manager(&package_dir, &workspace_root);

        let mut scripts: Vec<(String, String)> = package.scripts.into_iter().collect();
        scripts.sort_by_key(|(name, _)| script_priority(name));

        for (script_name, _) in scripts {
            if script_name.trim().is_empty() {
                continue;
            }

            let command = manager_command(&manager, &script_name);
            let confidence = confidence_for_script(&script_name);
            suggestions.push(DiscoverySuggestion {
                package_name: package.name.clone(),
                script_name,
                package_manager: manager.clone(),
                origin: origin.clone(),
                command,
                workdir: package_dir.to_string_lossy().to_string(),
                confidence,
            });
        }
    }

    suggestions.sort_by_key(|suggestion| {
        (
            Reverse(suggestion.confidence),
            suggestion.workdir.clone(),
            suggestion.script_name.clone(),
        )
    });
    Ok(suggestions)
}

pub fn discover_compose_services(project_dir: &str) -> Result<Option<ComposeDiscovery>, String> {
    let root = PathBuf::from(project_dir);
    if !root.exists() {
        return Err(format!(
            "Project directory '{}' does not exist.",
            root.display()
        ));
    }

    let Some(compose_file) = find_compose_file(&root) else {
        return Ok(None);
    };

    let raw_config = read_compose_config_json(&root, &compose_file)?;
    let services = parse_compose_services_from_json(&raw_config, &compose_file)?;

    Ok(Some(ComposeDiscovery {
        compose_file: compose_file.to_string_lossy().to_string(),
        services,
    }))
}

pub fn best_command_for_service(
    service_name: &str,
    suggestions: &[DiscoverySuggestion],
) -> Option<DiscoverySuggestion> {
    let needle = service_name.trim().to_lowercase();
    suggestions
        .iter()
        .map(|suggestion| {
            let mut scored = suggestion.clone();
            scored.confidence = score_suggestion(&needle, suggestion);
            scored
        })
        .max_by_key(|suggestion| suggestion.confidence)
}

pub fn detect_project_blueprint(
    project_dir: &str,
) -> Result<Option<ProjectBlueprintSuggestion>, String> {
    let root = PathBuf::from(project_dir);
    if !root.exists() {
        return Err(format!(
            "Project directory '{}' does not exist.",
            root.display()
        ));
    }

    let mut package_files = Vec::new();
    collect_package_json_files(&root, &mut package_files)?;

    let mut detected = Vec::new();
    for package_file in package_files {
        let package_dir = package_file
            .parent()
            .unwrap_or_else(|| Path::new(project_dir))
            .to_path_buf();
        let package = read_package_json(&package_file)?;
        let workspace_root = detect_workspace_root(&package_dir, &root);
        let (manager, _) = detect_package_manager(&package_dir, &workspace_root);

        if let Some(suggestion) = detect_expo_blueprint(&root, &package_dir, &package, &manager) {
            detected.push(suggestion);
        }
    }

    detected.sort_by_key(|suggestion| {
        (
            Reverse(suggestion.confidence),
            suggestion.workdir.clone(),
            suggestion.command.clone(),
        )
    });
    Ok(detected.into_iter().next())
}

fn score_suggestion(service_name: &str, suggestion: &DiscoverySuggestion) -> u16 {
    let mut score = suggestion.confidence;
    let script = suggestion.script_name.to_lowercase();
    let workdir = suggestion.workdir.to_lowercase();
    let package_name = suggestion
        .package_name
        .as_ref()
        .map(|name| name.to_lowercase())
        .unwrap_or_default();

    if !service_name.is_empty() && script.contains(service_name) {
        score = score.saturating_add(35);
    }
    if !service_name.is_empty() && package_name.contains(service_name) {
        score = score.saturating_add(30);
    }
    if !service_name.is_empty() && workdir.contains(service_name) {
        score = score.saturating_add(25);
    }

    if service_name == "frontend" && script == "dev" {
        score = score.saturating_add(20);
    }
    if service_name == "backend" && (script == "dev" || script == "start") {
        score = score.saturating_add(20);
    }

    score
}

fn detect_expo_blueprint(
    project_root: &Path,
    package_dir: &Path,
    package: &PackageJson,
    manager: &str,
) -> Option<ProjectBlueprintSuggestion> {
    let mut confidence = 0_u16;
    let mut reasons = Vec::new();

    if package_has_dependency(package, "expo") {
        confidence = confidence.saturating_add(120);
        reasons.push("expo dependency");
    }

    let mut selected_script = None::<&str>;
    for candidate in ["start", "dev", "android", "ios"] {
        let Some(script_value) = package.scripts.get(candidate) else {
            continue;
        };
        if script_value.to_ascii_lowercase().contains("expo") {
            selected_script = Some(candidate);
            confidence = confidence.saturating_add(match candidate {
                "start" => 60,
                "dev" => 45,
                _ => 30,
            });
            reasons.push("expo script");
            break;
        }
    }

    if confidence == 0 {
        return None;
    }

    if package_dir == project_root {
        confidence = confidence.saturating_add(35);
        reasons.push("root package");
    }

    let command = expo_command_for_manager(manager, selected_script);
    Some(ProjectBlueprintSuggestion {
        kind: ProjectBlueprintKind::Expo,
        package_name: package.name.clone(),
        package_manager: manager.to_string(),
        workdir: package_dir.to_string_lossy().to_string(),
        command,
        reason: reasons.join(", "),
        confidence,
    })
}

fn package_has_dependency(package: &PackageJson, dep: &str) -> bool {
    package.dependencies.contains_key(dep) || package.dev_dependencies.contains_key(dep)
}

fn expo_command_for_manager(manager: &str, script_name: Option<&str>) -> String {
    match (manager, script_name) {
        ("pnpm", Some(script_name)) => format!("pnpm {script_name}"),
        ("yarn", Some(script_name)) => format!("yarn {script_name}"),
        ("bun", Some(script_name)) => format!("bun run {script_name}"),
        ("npm", Some(script_name)) => format!("npm run {script_name}"),
        (_, Some(script_name)) => format!("npm run {script_name}"),
        _ => "npx expo start".to_string(),
    }
}

fn find_compose_file(project_root: &Path) -> Option<PathBuf> {
    COMPOSE_FILE_CANDIDATES
        .iter()
        .map(|candidate| project_root.join(candidate))
        .find(|path| path.exists() && path.is_file())
}

fn read_compose_config_json(project_root: &Path, compose_file: &Path) -> Result<String, String> {
    match run_compose_config_command(project_root, compose_file, true) {
        Ok(output) => Ok(output),
        Err(err_with_flag)
            if err_with_flag.contains("unknown flag: --no-interpolate")
                || err_with_flag.contains("unknown shorthand flag")
                || err_with_flag.contains("unknown option: --no-interpolate") =>
        {
            run_compose_config_command(project_root, compose_file, false)
        }
        Err(err) => Err(err),
    }
}

fn run_compose_config_command(
    project_root: &Path,
    compose_file: &Path,
    no_interpolate: bool,
) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("config")
        .arg("--format")
        .arg("json")
        .current_dir(project_root);

    if no_interpolate {
        cmd.arg("--no-interpolate");
    }

    let output = cmd.output().map_err(|err| {
        format!(
            "Failed to inspect compose file '{}': {err}",
            compose_file.display()
        )
    })?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "Failed to inspect compose file '{}': {}",
            compose_file.display(),
            if detail.is_empty() {
                "unknown docker compose error".to_string()
            } else {
                detail
            }
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err(format!(
            "Compose file '{}' returned empty config output.",
            compose_file.display()
        ));
    }
    Ok(raw)
}

fn parse_compose_services_from_json(
    raw_json: &str,
    compose_file: &Path,
) -> Result<Vec<ComposeServiceSuggestion>, String> {
    let value: serde_json::Value = serde_json::from_str(raw_json).map_err(|err| {
        format!(
            "Failed to parse docker compose config JSON for '{}': {err}",
            compose_file.display()
        )
    })?;
    let services_value = value.get("services").ok_or_else(|| {
        format!(
            "Compose file '{}' does not define a 'services' block.",
            compose_file.display()
        )
    })?;
    let services_map = services_value.as_object().ok_or_else(|| {
        format!(
            "Compose file '{}' has invalid 'services' format.",
            compose_file.display()
        )
    })?;

    let mut services = Vec::new();
    for (service_name, service_value) in services_map {
        let Some(service) = service_value.as_object() else {
            continue;
        };

        let image = service
            .get("image")
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let command = parse_compose_command(service.get("command"));
        let env = parse_compose_environment(service.get("environment"));
        let env_files = parse_compose_env_files(service.get("env_file"));
        let volumes = parse_compose_volumes(service.get("volumes"));
        let depends_on = parse_compose_depends_on(service.get("depends_on"));
        let ports = parse_compose_ports(service.get("ports"));
        let uses_build = service.get("build").is_some();

        services.push(ComposeServiceSuggestion {
            service_name: service_name.to_string(),
            image,
            command,
            env,
            env_files,
            volumes,
            depends_on,
            ports,
            uses_build,
        });
    }

    services.sort_by_key(|service| service.service_name.clone());
    Ok(services)
}

fn parse_compose_command(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(json_scalar_string)
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_compose_environment(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(json_scalar_string)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        serde_json::Value::Object(map) => {
            let mut pairs = map.iter().collect::<Vec<_>>();
            pairs.sort_by_key(|(key, _)| key.as_str());
            pairs
                .into_iter()
                .map(|(key, item)| {
                    let rendered = json_scalar_string(item)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    format!("{key}={rendered}")
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn parse_compose_env_files(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(path) => Some(path.trim().to_string()),
                serde_json::Value::Object(fields) => fields
                    .get("path")
                    .and_then(json_scalar_string)
                    .map(|path| path.trim().to_string()),
                _ => None,
            })
            .filter(|path| !path.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_compose_volumes(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                serde_json::Value::Object(fields) => {
                    let source = fields
                        .get("source")
                        .and_then(json_scalar_string)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let target = fields
                        .get("target")
                        .and_then(json_scalar_string)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if source.is_empty() || target.is_empty() {
                        return None;
                    }
                    let read_only = fields
                        .get("read_only")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if read_only {
                        Some(format!("{source}:{target}:ro"))
                    } else {
                        Some(format!("{source}:{target}"))
                    }
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_compose_depends_on(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(json_scalar_string)
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect(),
        serde_json::Value::Object(map) => {
            let mut names = map
                .keys()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
            names.sort();
            names
        }
        _ => Vec::new(),
    }
}

fn parse_compose_ports(value: Option<&serde_json::Value>) -> Vec<ComposePortSuggestion> {
    let Some(value) = value else {
        return Vec::new();
    };

    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    let mut ports = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let parsed = if let Some(port) = parse_compose_port_object(item) {
            Some(port)
        } else if let Some(raw) = item.as_str() {
            parse_compose_port_string(raw)
        } else {
            None
        };

        if let Some(port) = parsed {
            if port.published_port == 0 || port.protocol == "udp" {
                continue;
            }
            if seen.insert((port.published_port, port.protocol.clone())) {
                ports.push(port);
            }
        }
    }

    ports
}

fn parse_compose_port_object(value: &serde_json::Value) -> Option<ComposePortSuggestion> {
    let fields = value.as_object()?;
    let published = fields
        .get("published")
        .and_then(parse_compose_port_value)
        .or_else(|| fields.get("target").and_then(parse_compose_port_value))?;
    let protocol = fields
        .get("protocol")
        .and_then(json_scalar_string)
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| value == "tcp" || value == "udp")
        .unwrap_or_else(|| "tcp".to_string());

    Some(ComposePortSuggestion {
        published_port: published,
        protocol,
    })
}

fn parse_compose_port_string(raw: &str) -> Option<ComposePortSuggestion> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (mapping, protocol_raw) = match trimmed.rsplit_once('/') {
        Some((left, right)) => (left.trim(), right.trim()),
        None => (trimmed, "tcp"),
    };
    let protocol = match protocol_raw.to_ascii_lowercase().as_str() {
        "udp" => "udp",
        _ => "tcp",
    };

    let published = parse_published_port_from_mapping(mapping)?;
    Some(ComposePortSuggestion {
        published_port: published,
        protocol: protocol.to_string(),
    })
}

fn parse_published_port_from_mapping(mapping: &str) -> Option<u16> {
    let trimmed = mapping.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_ipv6_host = if trimmed.starts_with('[') {
        if let Some(index) = trimmed.rfind("]:") {
            &trimmed[index + 2..]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    let segments = without_ipv6_host
        .split(':')
        .map(str::trim)
        .collect::<Vec<_>>();

    if segments.len() == 1 {
        return parse_compose_port_token(segments[0]);
    }

    let published = segments
        .get(segments.len().saturating_sub(2))
        .and_then(|segment| parse_compose_port_token(segment))
        .or_else(|| {
            segments
                .last()
                .and_then(|segment| parse_compose_port_token(segment))
        })?;
    Some(published)
}

fn parse_compose_port_value(value: &serde_json::Value) -> Option<u16> {
    if let Some(port) = value.as_u64().and_then(|raw| u16::try_from(raw).ok()) {
        return Some(port);
    }
    let raw = json_scalar_string(value)?;
    parse_compose_port_token(&raw)
}

fn parse_compose_port_token(raw: &str) -> Option<u16> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let range_head = if let Some((head, tail)) = trimmed.split_once('-') {
        if head.chars().all(|ch| ch.is_ascii_digit())
            && tail
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
        {
            head.trim()
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    if let Ok(port) = range_head.parse::<u16>() {
        return Some(port);
    }

    let mut digits = String::new();
    for ch in range_head.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            break;
        }
    }

    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok()
}

fn json_scalar_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some(String::new()),
        serde_json::Value::Bool(raw) => Some(raw.to_string()),
        serde_json::Value::Number(raw) => Some(raw.to_string()),
        serde_json::Value::String(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn collect_package_json_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(root).map_err(|err| format!("Failed to read {}: {err}", root.display()))?;

    for entry_result in entries {
        let entry = match entry_result {
            Ok(value) => value,
            Err(_) => continue,
        };
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if IGNORED_DIRS.iter().any(|ignored| *ignored == file_name) {
                continue;
            }
            let _ = collect_package_json_files(&path, out);
            continue;
        }

        if file_name == "package.json" {
            out.push(path);
        }
    }

    Ok(())
}

fn read_package_json(path: &Path) -> Result<PackageJson, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|err| format!("Failed to parse {}: {err}", path.display()))
}

fn detect_package_manager(dir: &Path, search_root: &Path) -> (String, String) {
    let mut current = Some(dir);
    while let Some(path) = current {
        let origin = if path == dir { "local" } else { "workspace" };
        if path.join("pnpm-lock.yaml").exists() {
            return ("pnpm".to_string(), origin.to_string());
        }
        if path.join("yarn.lock").exists() {
            return ("yarn".to_string(), origin.to_string());
        }
        if path.join("bun.lockb").exists() || path.join("bun.lock").exists() {
            return ("bun".to_string(), origin.to_string());
        }
        if path.join("package-lock.json").exists() {
            return ("npm".to_string(), origin.to_string());
        }

        if path == search_root {
            break;
        }
        current = path.parent();
    }

    ("npm".to_string(), "fallback".to_string())
}

fn detect_workspace_root(start: &Path, project_root: &Path) -> PathBuf {
    let mut selected = project_root.to_path_buf();
    let mut current = Some(start);
    while let Some(path) = current {
        if path.join("pnpm-workspace.yaml").exists() || package_json_has_workspaces(path) {
            selected = path.to_path_buf();
        }
        if path == project_root {
            break;
        }
        current = path.parent();
    }
    selected
}

fn package_json_has_workspaces(dir: &Path) -> bool {
    let package_path = dir.join("package.json");
    let Ok(content) = fs::read_to_string(package_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value.get("workspaces").is_some()
}

fn manager_command(manager: &str, script_name: &str) -> String {
    match manager {
        "pnpm" => format!("pnpm {script_name}"),
        "yarn" => format!("yarn {script_name}"),
        "bun" => format!("bun run {script_name}"),
        _ => format!("npm run {script_name}"),
    }
}

fn script_priority(script_name: &str) -> u8 {
    let lowered = script_name.to_lowercase();
    PRIORITIZED_SCRIPTS
        .iter()
        .position(|candidate| *candidate == lowered)
        .map(|index| index as u8)
        .unwrap_or(PRIORITIZED_SCRIPTS.len() as u8 + 1)
}

fn confidence_for_script(script_name: &str) -> u16 {
    match script_name {
        "dev" => 100,
        "start" => 90,
        "serve" => 80,
        _ => 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("loopbox-discovery-{nonce}-{counter}"))
    }

    #[test]
    fn discover_project_commands_finds_prioritized_scripts() {
        let root = temp_dir();
        let app_dir = root.join("apps").join("frontend");
        fs::create_dir_all(&app_dir).expect("create app dir");

        fs::write(app_dir.join("pnpm-lock.yaml"), "").expect("write lock");
        fs::write(
            app_dir.join("package.json"),
            r#"{
              "name": "@acme/frontend",
              "scripts": {
                "test": "vitest",
                "dev": "vite",
                "start": "node server.js"
              }
            }"#,
        )
        .expect("write package");

        let suggestions = discover_project_commands(root.to_string_lossy().as_ref())
            .expect("discover suggestions");

        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].script_name, "dev");
        assert_eq!(suggestions[0].package_manager, "pnpm");
        assert_eq!(suggestions[0].origin, "local");
        assert_eq!(suggestions[0].command, "pnpm dev");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn best_command_for_service_prefers_matching_package_name() {
        let suggestions = vec![
            DiscoverySuggestion {
                package_name: Some("@acme/backend".to_string()),
                script_name: "dev".to_string(),
                package_manager: "pnpm".to_string(),
                origin: "workspace".to_string(),
                command: "pnpm dev".to_string(),
                workdir: "/repo/apps/backend".to_string(),
                confidence: 100,
            },
            DiscoverySuggestion {
                package_name: Some("@acme/frontend".to_string()),
                script_name: "dev".to_string(),
                package_manager: "pnpm".to_string(),
                origin: "workspace".to_string(),
                command: "pnpm dev".to_string(),
                workdir: "/repo/apps/frontend".to_string(),
                confidence: 100,
            },
        ];

        let chosen = best_command_for_service("frontend", &suggestions)
            .expect("should choose frontend command");
        assert_eq!(chosen.package_name.as_deref(), Some("@acme/frontend"));
    }

    #[test]
    fn detect_package_manager_inherits_from_workspace_root() {
        let root = temp_dir();
        let app_dir = root.join("apps").join("web");
        fs::create_dir_all(&app_dir).expect("create app dir");

        fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n")
            .expect("write workspace file");
        fs::write(root.join("pnpm-lock.yaml"), "").expect("write root lock");
        fs::write(
            app_dir.join("package.json"),
            r#"{
              "name": "@acme/web",
              "scripts": {
                "dev": "vite"
              }
            }"#,
        )
        .expect("write package");

        let suggestions = discover_project_commands(root.to_string_lossy().as_ref())
            .expect("discover suggestions");
        let web = suggestions
            .iter()
            .find(|suggestion| suggestion.package_name.as_deref() == Some("@acme/web"))
            .expect("web suggestion");
        assert_eq!(web.package_manager, "pnpm");
        assert_eq!(web.origin, "workspace");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detect_project_blueprint_prefers_expo_workspace_app() {
        let root = temp_dir();
        let mobile_dir = root.join("apps").join("mobile");
        fs::create_dir_all(&mobile_dir).expect("create mobile dir");

        fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n")
            .expect("write workspace file");
        fs::write(root.join("pnpm-lock.yaml"), "").expect("write root lock");
        fs::write(
            mobile_dir.join("package.json"),
            r#"{
              "name": "@acme/mobile",
              "dependencies": {
                "expo": "^54.0.0"
              },
              "scripts": {
                "start": "expo start",
                "ios": "expo run:ios"
              }
            }"#,
        )
        .expect("write package");

        let detected = detect_project_blueprint(root.to_string_lossy().as_ref())
            .expect("detect blueprint")
            .expect("should detect expo");

        assert_eq!(detected.kind, ProjectBlueprintKind::Expo);
        assert_eq!(detected.package_name.as_deref(), Some("@acme/mobile"));
        assert_eq!(detected.workdir, mobile_dir.to_string_lossy().to_string());
        assert!(detected.command.contains("expo") || detected.command.contains("start"));
        assert!(!detected.command.contains("--localhost"));
        assert!(!detected.command.contains("--port"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_compose_services_from_json_extracts_fields() {
        let compose_path = PathBuf::from("/tmp/compose.yaml");
        let raw = r#"{
          "services": {
            "api": {
              "image": "ghcr.io/acme/api:latest",
              "command": ["node", "server.js"],
              "environment": {
                "NODE_ENV": "development",
                "PORT": "8080"
              },
              "env_file": [
                {"path": ".env"},
                ".env.local"
              ],
              "volumes": [
                "./data:/data",
                {"source": "logs", "target": "/var/log/app", "read_only": true}
              ],
              "depends_on": {
                "db": {"condition": "service_started"},
                "redis": {"condition": "service_started"}
              },
              "ports": [
                "127.0.0.1:8080:8080",
                {"published":"8443","target":8443,"protocol":"tcp"},
                {"published":"53","target":53,"protocol":"udp"}
              ],
              "build": {"context": "."}
            }
          }
        }"#;

        let services = parse_compose_services_from_json(raw, &compose_path)
            .expect("should parse compose json");
        assert_eq!(services.len(), 1);

        let api = &services[0];
        assert_eq!(api.service_name, "api");
        assert_eq!(api.image.as_deref(), Some("ghcr.io/acme/api:latest"));
        assert_eq!(
            api.command,
            vec!["node".to_string(), "server.js".to_string()]
        );
        assert_eq!(
            api.env_files,
            vec![".env".to_string(), ".env.local".to_string()]
        );
        assert_eq!(
            api.volumes,
            vec![
                "./data:/data".to_string(),
                "logs:/var/log/app:ro".to_string()
            ]
        );
        assert_eq!(api.depends_on, vec!["db".to_string(), "redis".to_string()]);
        assert_eq!(api.ports.len(), 2);
        assert_eq!(api.ports[0].published_port, 8080);
        assert_eq!(api.ports[1].published_port, 8443);
        assert!(api.uses_build);
    }

    #[test]
    fn parse_compose_port_token_supports_variable_defaults() {
        assert_eq!(parse_compose_port_token("${PORT:-3001}"), Some(3001));
        assert_eq!(
            parse_compose_port_string("${SERVER_PORT:-3001}:3001").map(|port| port.published_port),
            Some(3001)
        );
        assert_eq!(
            parse_compose_port_string("127.0.0.1:5432:5432").map(|port| port.published_port),
            Some(5432)
        );
    }
}
