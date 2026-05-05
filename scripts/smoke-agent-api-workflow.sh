#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

AGENT_PORT="39449"
KEEP_ARTIFACTS="false"
LOOPBOX_BIN="${LOOPBOX_BIN:-}"
AUTO_BUILD="true"

usage() {
  cat <<'USAGE'
Usage: smoke-agent-api-workflow.sh [options]

Runs isolated Agent API workflow smoke tests in headless mode:
  health -> openapi -> doctor -> create project -> start service -> logs -> input -> stop.

The default run covers both auth-disabled and auth-enabled Agent API settings.

Options:
  --loopbox-bin <path>   Loopbox executable to run.
  --agent-port <port>    First Agent API bind port for the isolated run (default: 39449).
  --no-auto-build        Do not run cargo build when no debug binary exists.
  --keep-artifacts       Keep the temporary config/project directory.
  -h, --help             Show this help text.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --loopbox-bin)
      LOOPBOX_BIN="${2:-}"
      shift 2
      ;;
    --agent-port)
      AGENT_PORT="${2:-}"
      shift 2
      ;;
    --no-auto-build)
      AUTO_BUILD="false"
      shift
      ;;
    --keep-artifacts)
      KEEP_ARTIFACTS="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

require_cmd curl
require_cmd python3

if [[ -z "$LOOPBOX_BIN" ]]; then
  LOOPBOX_BIN="$repo_root/target/debug/loopbox"
fi

if [[ ! -x "$LOOPBOX_BIN" ]]; then
  if [[ "$AUTO_BUILD" != "true" ]]; then
    echo "Loopbox binary not found: $LOOPBOX_BIN" >&2
    exit 1
  fi
  cargo build
fi

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/loopbox-agent-api-smoke.XXXXXX")"
LOOPBOX_PIDS=()
CASE_BASE_URLS=()
CASE_TOKENS=()
CASE_PROJECTS=()

cleanup() {
  local idx
  for idx in "${!CASE_BASE_URLS[@]}"; do
    local base="${CASE_BASE_URLS[$idx]}"
    local token="${CASE_TOKENS[$idx]}"
    local project="${CASE_PROJECTS[$idx]}"
    if [[ -n "$base" && -n "$project" ]]; then
      if [[ -n "$token" ]]; then
        curl -fsS -H "Authorization: Bearer $token" -X POST \
          "$base/v1/projects/$project/stop" >/dev/null 2>&1 || true
      else
        curl -fsS -X POST "$base/v1/projects/$project/stop" >/dev/null 2>&1 || true
      fi
    fi
  done

  for pid in "${LOOPBOX_PIDS[@]}"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    fi
  done

  if [[ "$KEEP_ARTIFACTS" != "true" ]]; then
    rm -rf "$TMP_ROOT"
  else
    echo "Kept smoke artifacts at $TMP_ROOT"
  fi
}
trap cleanup EXIT

json_field() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
print(data[sys.argv[2]])
PY
}

write_service_script() {
  local project_dir="$1"
  cat > "$project_dir/service.py" <<'PY'
import sys

print("loopbox-smoke-ready", flush=True)
for line in sys.stdin:
    print("loopbox-smoke-input:" + line.strip(), flush=True)
PY
}

write_payload() {
  local project_name="$1"
  local project_dir="$2"
  local payload_file="$3"
  python3 - "$project_name" "$project_dir" "$payload_file" <<'PY'
import json
import sys

project_name, project_dir, output = sys.argv[1:4]
payload = {
    "name": project_name,
    "dir": project_dir,
    "ip": "127.0.0.2",
    "services": [
        {
            "name": "echo",
            "runtime": "process",
            "command": "python3 -u service.py",
            "workdir": project_dir,
            "ports": []
        }
    ]
}
with open(output, "w", encoding="utf-8") as f:
    json.dump(payload, f)
PY
}

assert_openapi_shape() {
  local openapi_file="$1"
  python3 - "$openapi_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    spec = json.load(f)
paths = spec["paths"]
for path, method in [
    ("/v1/doctor", "get"),
    ("/v1/projects/{project}/resources", "get"),
    ("/v1/projects/{project}/services/{service}/input", "post"),
]:
    if method not in paths.get(path, {}):
        raise SystemExit(f"Missing OpenAPI operation: {method.upper()} {path}")
schemas = spec["components"]["schemas"]
for schema in [
    "DoctorResponse",
    "ProjectRuntimeResponse",
    "ProjectResourcesResponse",
    "ServiceResourceSampleDto",
    "ServiceInputRequest",
]:
    if schema not in schemas:
        raise SystemExit(f"Missing OpenAPI schema: {schema}")
if schemas["ServiceRuntimeDto"]["properties"]["input_attached"]["type"] != "boolean":
    raise SystemExit("ServiceRuntimeDto.input_attached schema mismatch")
if schemas["ProjectResourcesResponse"]["properties"]["latest"]["items"]["$ref"] != "#/components/schemas/ServiceResourceSampleDto":
    raise SystemExit("ProjectResourcesResponse.latest schema mismatch")
PY
}

wait_for_discovery() {
  local discovery_file="$1"
  local pid="$2"
  local log_file="$3"
  for _ in $(seq 1 80); do
    if [[ -s "$discovery_file" ]]; then
      return 0
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      echo "Loopbox exited before writing discovery file." >&2
      cat "$log_file" >&2 || true
      return 1
    fi
    sleep 0.25
  done

  echo "Timed out waiting for Agent API discovery file: $discovery_file" >&2
  cat "$log_file" >&2 || true
  return 1
}

run_case() {
  local case_name="$1"
  local auth_enabled="$2"
  local port="$3"
  local case_root="$TMP_ROOT/$case_name"
  local xdg_config_home_dir="$case_root/xdg"
  local loopbox_config_dir="$xdg_config_home_dir/loopbox"
  local config_file="$loopbox_config_dir/config.toml"
  local discovery_file="$loopbox_config_dir/agent-api.json"
  local token_file="$loopbox_config_dir/agent-api-token"
  local project_dir="$case_root/project"
  local loopbox_log="$case_root/loopbox.log"
  local payload_file="$case_root/project.json"
  local openapi_file="$case_root/openapi.json"
  local logs_file="$case_root/logs.json"
  local resources_file="$case_root/resources.json"
  local project_name="agent-smoke-$case_name"

  mkdir -p "$loopbox_config_dir" "$project_dir"
  cat > "$config_file" <<TOML
[global]
domain_suffix = "localhost"
ip_base = "127.0.0."
ip_range_start = 2
ip_range_end = 254

[global.agent_api]
enabled = true
port = $port
auth_enabled = $auth_enabled

[global.resource_metrics]
enabled = true
sample_interval_secs = 2
retention_days = 1
max_storage_mb = 25
TOML

  write_service_script "$project_dir"
  write_payload "$project_name" "$project_dir" "$payload_file"

  echo "Starting headless Loopbox Agent API ($case_name, auth=$auth_enabled) on 127.0.0.1:$port"
  XDG_CONFIG_HOME="$xdg_config_home_dir" "$LOOPBOX_BIN" __agent_api_server >"$loopbox_log" 2>&1 &
  local loopbox_pid="$!"
  LOOPBOX_PIDS+=("$loopbox_pid")

  wait_for_discovery "$discovery_file" "$loopbox_pid" "$loopbox_log"

  local base_url
  base_url="$(json_field "$discovery_file" "base_url")"
  local token=""
  local auth_args=()
  if [[ "$auth_enabled" == "true" ]]; then
    token="$(cat "$token_file")"
    auth_args=(-H "Authorization: Bearer $token")
    if curl -fsS "$base_url/v1/projects" >/dev/null 2>&1; then
      echo "Protected endpoint accepted missing auth in auth-enabled case." >&2
      exit 1
    fi
    if curl -fsS -H "Authorization: Bearer wrong-token" "$base_url/v1/meta" >/dev/null 2>&1; then
      echo "Protected endpoint accepted an invalid bearer token in auth-enabled case." >&2
      exit 1
    fi
    curl -fsS "$base_url/v1/health" >/dev/null
    curl -fsS "$base_url/v1/openapi.json" >/dev/null
  fi

  CASE_BASE_URLS+=("$base_url")
  CASE_TOKENS+=("$token")
  CASE_PROJECTS+=("$project_name")

  curl -fsS "$base_url/v1/health" >/dev/null
  curl -fsS "${auth_args[@]}" "$base_url/v1/doctor" >/dev/null
  curl -fsS "${auth_args[@]}" "$base_url/v1/openapi.json" -o "$openapi_file"
  assert_openapi_shape "$openapi_file"

  curl -fsS \
    -X POST \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    --data @"$payload_file" \
    "$base_url/v1/projects?apply_system_setup=false" >/dev/null

  curl -fsS "${auth_args[@]}" -X POST \
    "$base_url/v1/projects/$project_name/services/echo/start" >/dev/null

  for _ in $(seq 1 60); do
    if curl -fsS "${auth_args[@]}" "$base_url/v1/projects/$project_name/runtime" | python3 -c \
      'import json,sys; data=json.load(sys.stdin); svc=data["services"][0]; sys.exit(0 if svc["input_attached"] else 1)' >/dev/null 2>&1; then
      break
    fi
    sleep 0.25
  done

  for _ in $(seq 1 100); do
    curl -fsS "${auth_args[@]}" \
      "$base_url/v1/projects/$project_name/resources?service=echo&window=15m&limit=20" \
      -o "$resources_file"
    if python3 - "$resources_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if (
    data.get("service") == "echo"
    and data.get("window") == "15m"
    and data.get("limit") == 20
    and data.get("latest")
    and data.get("samples")
    and data["latest"][0].get("service") == "echo"
    and data["latest"][0].get("runtime") == "process"
):
    raise SystemExit(0)
raise SystemExit(1)
PY
    then
      break
    fi
    sleep 0.25
  done

  python3 - "$resources_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if not data.get("latest"):
    raise SystemExit("Resource metrics latest sample was not captured.")
if not data.get("samples"):
    raise SystemExit("Resource metrics series sample was not captured.")
sample = data["latest"][0]
for key in ["project", "service", "sampled_at_unix_ms", "sampled_at_utc", "runtime", "state"]:
    if key not in sample:
        raise SystemExit(f"Resource metrics sample missing key: {key}")
if sample["service"] != "echo" or sample["runtime"] != "process":
    raise SystemExit("Resource metrics sample did not describe the echo process service.")
PY

  for _ in $(seq 1 60); do
    curl -fsS "${auth_args[@]}" "$base_url/v1/projects/$project_name/logs?service=echo&limit=50" -o "$logs_file"
    if python3 - "$logs_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if any("loopbox-smoke-ready" in line for line in data["lines"]):
    raise SystemExit(0)
raise SystemExit(1)
PY
    then
      break
    fi
    sleep 0.25
  done

  python3 - "$logs_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if not any("loopbox-smoke-ready" in line for line in data["lines"]):
    raise SystemExit("Service ready log line was not captured.")
PY

  curl -fsS \
    -X POST \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    --data '{"text":"ping\n"}' \
    "$base_url/v1/projects/$project_name/services/echo/input" >/dev/null

  for _ in $(seq 1 60); do
    curl -fsS "${auth_args[@]}" "$base_url/v1/projects/$project_name/logs?service=echo&limit=50" -o "$logs_file"
    if python3 - "$logs_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if any("loopbox-smoke-input:ping" in line for line in data["lines"]):
    raise SystemExit(0)
raise SystemExit(1)
PY
    then
      break
    fi
    sleep 0.25
  done

  python3 - "$logs_file" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
if not any("loopbox-smoke-input:ping" in line for line in data["lines"]):
    raise SystemExit("Service input log line was not captured.")
PY

  curl -fsS "${auth_args[@]}" -X POST \
    "$base_url/v1/projects/$project_name/services/echo/stop" >/dev/null

  echo "Loopbox Agent API smoke completed for $case_name."
}

run_case "auth-off" "false" "$AGENT_PORT"
run_case "auth-on" "true" "$((AGENT_PORT + 1))"

echo "Loopbox Agent API smoke completed."
