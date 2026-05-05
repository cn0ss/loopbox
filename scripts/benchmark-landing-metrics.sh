#!/usr/bin/env bash
set -euo pipefail

# Keep numeric formatting/parsing stable across locales (dot decimal separator).
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

REQUESTS=300
WARMUP=30
LATENCY_TARGET_MS="1.0"
MEMORY_TARGET_MB="15.0"
MEMORY_TOLERANCE_MB="5.0"
AGENT_PORT=39444
PROJECT_NAME="landingbench"
SERVICE_NAME="web"
PROJECT_IP="127.0.0.1"
SERVICE_PORT=19081
AUTO_BUILD=1
KEEP_ARTIFACTS=0
OUTPUT_JSON=""
LOOPBOX_BIN=""

TMP_ROOT=""
XDG_CONFIG_HOME_DIR=""
LOOPBOX_CONFIG_DIR=""
LOOPBOX_CONFIG_FILE=""
DISCOVERY_FILE=""
TOKEN_FILE=""
BENCH_APP_DIR=""
RESULTS_DIR=""
LOOPBOX_STDOUT_LOG=""
BASE_URL=""
AUTH_ENABLED="false"
TOKEN=""
LOOPBOX_PID=""
PROXY_PORT=""

usage() {
  cat <<'USAGE'
Usage: benchmark-landing-metrics.sh [options]

Benchmarks the landing page claims:
  - Latency overhead < 1ms
  - Memory usage ~ 15MB

The script launches an isolated Loopbox instance using a temporary XDG config,
starts a local Python HTTP service via Agent API, and compares:
  direct:  http://<project-ip>:<service-port>
  proxy:   http://127.0.0.1:<proxy-port> with Host header

Options:
  --loopbox-bin <path>         Loopbox executable to run.
  --requests <n>               Benchmark request pairs (default: 300).
  --warmup <n>                 Warmup request pairs (default: 30).
  --agent-port <port>          Agent API bind port for isolated run (default: 39444).
  --project-ip <ip>            Loopback IP for benchmark project (default: 127.0.0.1).
  --service-port <port>        HTTP service port (default: 19081).
  --latency-target-ms <value>  Claim threshold for overhead (default: 1.0).
  --memory-target-mb <value>   Claim center for memory usage (default: 15.0).
  --memory-tolerance-mb <val>  Allowed +/- range around memory target (default: 5.0).
  --output-json <path>         Write a JSON report file.
  --keep-artifacts             Keep temporary artifacts for inspection.
  --no-auto-build              Do not run cargo build --release if binary is missing.
  -h, --help                   Show this help text.

Examples:
  ./scripts/benchmark-landing-metrics.sh
  ./scripts/benchmark-landing-metrics.sh --requests 500 --output-json ./build/landing-bench.json
  ./scripts/benchmark-landing-metrics.sh --loopbox-bin ./target/release/loopbox
USAGE
}

log() {
  printf '>> %s\n' "$*"
}

die() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "Missing required command: $cmd"
}

is_positive_int() {
  [[ "$1" =~ ^[0-9]+$ ]] && (( "$1" > 0 ))
}

loopbox_process_alive() {
  local pid="$1"
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    return 1
  fi

  local stat
  stat="$(ps -o stat= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
  [[ -n "$stat" ]] || return 1
  [[ "$stat" == Z* ]] && return 1
  return 0
}

toml_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '"%s"' "$value"
}

float_sub() {
  awk -v a="$1" -v b="$2" 'BEGIN { printf "%.6f", a - b }'
}

float_add() {
  awk -v a="$1" -v b="$2" 'BEGIN { printf "%.6f", a + b }'
}

calc_stats_from_file() {
  local source_file="$1"
  local sorted_file="$TMP_ROOT/$(basename "$source_file").sorted"
  sort -n "$source_file" > "$sorted_file"

  local count
  count="$(wc -l < "$sorted_file" | tr -d '[:space:]')"
  if [[ "$count" -eq 0 ]]; then
    printf '0 nan nan nan nan nan nan\n'
    return
  fi

  local avg min max
  avg="$(awk '{sum += $1} END {printf "%.6f", sum / NR}' "$sorted_file")"
  min="$(head -n 1 "$sorted_file")"
  max="$(tail -n 1 "$sorted_file")"

  local p50_idx p95_idx p99_idx
  p50_idx=$(( (count + 1) / 2 ))
  p95_idx=$(( (count * 95 + 99) / 100 ))
  p99_idx=$(( (count * 99 + 99) / 100 ))
  (( p95_idx > count )) && p95_idx="$count"
  (( p99_idx > count )) && p99_idx="$count"

  local p50 p95 p99
  p50="$(sed -n "${p50_idx}p" "$sorted_file")"
  p95="$(sed -n "${p95_idx}p" "$sorted_file")"
  p99="$(sed -n "${p99_idx}p" "$sorted_file")"

  printf '%s %s %s %s %s %s %s\n' "$count" "$avg" "$p50" "$p95" "$p99" "$min" "$max"
}

capture_time_ms() {
  local url="$1"
  local host_header="${2:-}"
  local time_seconds

  if [[ -n "$host_header" ]]; then
    time_seconds="$(
      curl -sS --fail \
        --http1.1 \
        --connect-timeout 1 \
        --max-time 3 \
        -H "Host: ${host_header}" \
        -o /dev/null \
        -w '%{time_total}' \
        "$url"
    )"
  else
    time_seconds="$(
      curl -sS --fail \
        --http1.1 \
        --connect-timeout 1 \
        --max-time 3 \
        -o /dev/null \
        -w '%{time_total}' \
        "$url"
    )"
  fi

  awk -v sec="$time_seconds" 'BEGIN { printf "%.6f", sec * 1000.0 }'
}

run_interleaved_samples() {
  local count="$1"
  local direct_url="$2"
  local proxy_url="$3"
  local direct_out="$4"
  local proxy_out="$5"
  local proxy_host="$6"

  : > "$direct_out"
  : > "$proxy_out"

  local direct_ms proxy_ms i
  for (( i = 1; i <= count; i++ )); do
    if (( i % 2 == 1 )); then
      direct_ms="$(capture_time_ms "$direct_url")"
      proxy_ms="$(capture_time_ms "$proxy_url" "$proxy_host")"
    else
      proxy_ms="$(capture_time_ms "$proxy_url" "$proxy_host")"
      direct_ms="$(capture_time_ms "$direct_url")"
    fi
    printf '%s\n' "$direct_ms" >> "$direct_out"
    printf '%s\n' "$proxy_ms" >> "$proxy_out"
  done
}

sample_memory_mb() {
  local pid="$1"
  local output_file="$2"
  local samples="$3"
  local sleep_seconds="$4"

  : > "$output_file"

  local i rss_kb
  for (( i = 1; i <= samples; i++ )); do
    rss_kb="$(ps -o rss= -p "$pid" | tr -d '[:space:]')"
    [[ -n "$rss_kb" ]] || die "Failed to read RSS for pid ${pid}."
    awk -v kb="$rss_kb" 'BEGIN { printf "%.6f\n", kb / 1024.0 }' >> "$output_file"
    sleep "$sleep_seconds"
  done
}

api_request() {
  local method="$1"
  local path="$2"
  local url="${BASE_URL}${path}"

  if [[ "$AUTH_ENABLED" == "true" ]]; then
    curl -sS --fail -X "$method" -H "Authorization: Bearer ${TOKEN}" "$url"
  else
    curl -sS --fail -X "$method" "$url"
  fi
}

api_request_capture_status() {
  local method="$1"
  local path="$2"
  local response_file="$3"
  local url="${BASE_URL}${path}"

  if [[ "$AUTH_ENABLED" == "true" ]]; then
    curl -sS -X "$method" -H "Authorization: Bearer ${TOKEN}" -o "$response_file" -w '%{http_code}' "$url"
  else
    curl -sS -X "$method" -o "$response_file" -w '%{http_code}' "$url"
  fi
}

api_error_message_from_body() {
  local response_file="$1"
  jq -r '.error.message // empty' "$response_file" 2>/dev/null || true
}

read_service_logs_excerpt() {
  local response_file http_code
  response_file="$(mktemp "$TMP_ROOT/logs-response.XXXXXX")"
  http_code="$(api_request_capture_status GET "/v1/projects/${PROJECT_NAME}/logs?service=${SERVICE_NAME}&limit=25" "$response_file" 2>/dev/null || true)"
  if [[ "$http_code" != "200" ]]; then
    rm -f "$response_file"
    return 1
  fi

  jq -r '.lines[]?' "$response_file" 2>/dev/null | tail -n 8
  rm -f "$response_file"
}

start_benchmark_project() {
  # Ensure a clean service state from any previous interrupted runs.
  api_request POST "/v1/projects/${PROJECT_NAME}/stop" >/dev/null 2>&1 || true

  local response_file http_code error_message body_preview
  response_file="$(mktemp "$TMP_ROOT/start-response.XXXXXX")"

  http_code="$(api_request_capture_status POST "/v1/projects/${PROJECT_NAME}/start" "$response_file")"
  if [[ "$http_code" == "200" ]]; then
    rm -f "$response_file"
    return 0
  fi

  if [[ "$http_code" == "409" ]]; then
    error_message="$(api_error_message_from_body "$response_file")"
    if [[ -n "$error_message" ]]; then
      log "Project start returned 409: $error_message"
    else
      log "Project start returned 409. Retrying once after stop."
    fi
    api_request POST "/v1/projects/${PROJECT_NAME}/stop" >/dev/null 2>&1 || true
    sleep 0.2
    http_code="$(api_request_capture_status POST "/v1/projects/${PROJECT_NAME}/start" "$response_file")"
    if [[ "$http_code" == "200" ]]; then
      rm -f "$response_file"
      return 0
    fi
  fi

  error_message="$(api_error_message_from_body "$response_file")"
  body_preview="$(tr '\n' ' ' < "$response_file" | awk '{gsub(/[[:space:]]+/, " "); print substr($0, 1, 400)}')"
  rm -f "$response_file"

  local logs_excerpt=""
  logs_excerpt="$(read_service_logs_excerpt || true)"
  if [[ -n "$error_message" ]]; then
    if [[ -n "$logs_excerpt" ]]; then
      die "Failed to start benchmark project (HTTP ${http_code}): ${error_message}
Service log excerpt:
${logs_excerpt}"
    fi
    die "Failed to start benchmark project (HTTP ${http_code}): ${error_message}"
  fi
  if [[ -n "$logs_excerpt" ]]; then
    die "Failed to start benchmark project (HTTP ${http_code}): ${body_preview:-<empty response>}
Service log excerpt:
${logs_excerpt}"
  fi
  die "Failed to start benchmark project (HTTP ${http_code}): ${body_preview:-<empty response>}"
}

wait_for_file() {
  local path="$1"
  local timeout_seconds="$2"
  local sleep_seconds=0.25
  local attempts
  attempts="$(awk -v t="$timeout_seconds" -v s="$sleep_seconds" 'BEGIN { printf "%d", t / s }')"
  (( attempts < 1 )) && attempts=1

  local i
  for (( i = 1; i <= attempts; i++ )); do
    if [[ -f "$path" ]]; then
      return 0
    fi
    sleep "$sleep_seconds"
  done
  return 1
}

wait_for_discovery_file() {
  local timeout_seconds="$1"
  local sleep_seconds=0.25
  local attempts
  attempts="$(awk -v t="$timeout_seconds" -v s="$sleep_seconds" 'BEGIN { printf "%d", t / s }')"
  (( attempts < 1 )) && attempts=1

  local i
  for (( i = 1; i <= attempts; i++ )); do
    if [[ -f "$DISCOVERY_FILE" ]]; then
      return 0
    fi
    if [[ -f "$LOOPBOX_STDOUT_LOG" ]]; then
      if grep -q "Loopbox agent API startup warning: Failed to bind local agent API server" "$LOOPBOX_STDOUT_LOG"; then
        return 3
      fi
    fi
    if [[ -n "${LOOPBOX_PID:-}" ]] && ! loopbox_process_alive "$LOOPBOX_PID"; then
      return 2
    fi
    sleep "$sleep_seconds"
  done
  return 1
}

wait_for_health_endpoint() {
  local health_url="${BASE_URL}/v1/health"
  local i
  for (( i = 1; i <= 120; i++ )); do
    if curl -sS --fail "$health_url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

wait_for_service_running() {
  local i state
  for (( i = 1; i <= 120; i++ )); do
    state="$(
      api_request GET "/v1/projects/${PROJECT_NAME}/runtime" \
        | jq -r --arg service "$SERVICE_NAME" '.services[] | select(.service == $service) | .state' \
        | head -n 1
    )"
    if [[ "$state" == "running" ]]; then
      return 0
    fi
    if [[ "$state" == "crashed" || "$state" == "unhealthy" || "$state" == "stopped" ]]; then
      return 1
    fi
    sleep 0.25
  done
  return 1
}

find_loopbox_bin() {
  if [[ -n "$LOOPBOX_BIN" ]]; then
    return 0
  fi

  local release_bin="$PROJECT_DIR/target/release/loopbox"
  if [[ -x "$release_bin" ]]; then
    LOOPBOX_BIN="$release_bin"
  fi

  local bundle_bin="$PROJECT_DIR/dist/Loopbox.app/Contents/MacOS/loopbox"
  if [[ -z "$LOOPBOX_BIN" && -x "$bundle_bin" ]]; then
    LOOPBOX_BIN="$bundle_bin"
  fi
}

normalize_loopbox_bin() {
  [[ -n "$LOOPBOX_BIN" ]] || return 0
  if [[ "$LOOPBOX_BIN" == *.app ]]; then
    LOOPBOX_BIN="$LOOPBOX_BIN/Contents/MacOS/loopbox"
  fi
}

cleanup() {
  local exit_code=$?
  set +e

  if [[ -n "${BASE_URL:-}" ]]; then
    api_request POST "/v1/projects/${PROJECT_NAME}/stop" >/dev/null 2>&1 || true
  fi

  if [[ -n "${LOOPBOX_PID:-}" ]] && kill -0 "$LOOPBOX_PID" >/dev/null 2>&1; then
    kill "$LOOPBOX_PID" >/dev/null 2>&1 || true
    wait "$LOOPBOX_PID" >/dev/null 2>&1 || true
  fi

  if [[ -n "${TMP_ROOT:-}" && "$KEEP_ARTIFACTS" -eq 0 ]]; then
    rm -rf "$TMP_ROOT"
  fi

  if [[ -n "${TMP_ROOT:-}" && "$KEEP_ARTIFACTS" -eq 1 ]]; then
    printf 'Artifacts kept at %s\n' "$TMP_ROOT" >&2
  fi

  exit "$exit_code"
}
trap cleanup EXIT INT TERM

while [[ $# -gt 0 ]]; do
  case "$1" in
    --loopbox-bin)
      LOOPBOX_BIN="${2:-}"
      shift 2
      ;;
    --requests)
      REQUESTS="${2:-}"
      shift 2
      ;;
    --warmup)
      WARMUP="${2:-}"
      shift 2
      ;;
    --agent-port)
      AGENT_PORT="${2:-}"
      shift 2
      ;;
    --project-ip)
      PROJECT_IP="${2:-}"
      shift 2
      ;;
    --service-port)
      SERVICE_PORT="${2:-}"
      shift 2
      ;;
    --latency-target-ms)
      LATENCY_TARGET_MS="${2:-}"
      shift 2
      ;;
    --memory-target-mb)
      MEMORY_TARGET_MB="${2:-}"
      shift 2
      ;;
    --memory-tolerance-mb)
      MEMORY_TOLERANCE_MB="${2:-}"
      shift 2
      ;;
    --output-json)
      OUTPUT_JSON="${2:-}"
      shift 2
      ;;
    --keep-artifacts)
      KEEP_ARTIFACTS=1
      shift
      ;;
    --no-auto-build)
      AUTO_BUILD=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "Unknown argument: $1"
      ;;
  esac
done

require_cmd bash
require_cmd curl
require_cmd jq
require_cmd python3
require_cmd awk
require_cmd sort
require_cmd ps

is_positive_int "$REQUESTS" || die "--requests must be a positive integer."
is_positive_int "$WARMUP" || die "--warmup must be a positive integer."
is_positive_int "$AGENT_PORT" || die "--agent-port must be a positive integer."
is_positive_int "$SERVICE_PORT" || die "--service-port must be a positive integer."

find_loopbox_bin
if [[ -z "$LOOPBOX_BIN" && "$AUTO_BUILD" -eq 1 ]]; then
  log "No Loopbox binary found. Building one via cargo build --release..."
  (
    cd "$PROJECT_DIR"
    cargo build --release
  )
  find_loopbox_bin
fi

normalize_loopbox_bin
[[ -n "$LOOPBOX_BIN" ]] || die "Loopbox binary not found. Provide --loopbox-bin or build first."
[[ -x "$LOOPBOX_BIN" ]] || die "Loopbox binary is not executable: $LOOPBOX_BIN"

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/loopbox-landing-bench.XXXXXX")"
XDG_CONFIG_HOME_DIR="$TMP_ROOT/config"
LOOPBOX_CONFIG_DIR="$XDG_CONFIG_HOME_DIR/loopbox"
LOOPBOX_CONFIG_FILE="$LOOPBOX_CONFIG_DIR/config.toml"
DISCOVERY_FILE="$LOOPBOX_CONFIG_DIR/agent-api.json"
TOKEN_FILE="$LOOPBOX_CONFIG_DIR/agent-api-token"
BENCH_APP_DIR="$TMP_ROOT/bench-app"
RESULTS_DIR="$TMP_ROOT/results"
LOOPBOX_STDOUT_LOG="$TMP_ROOT/loopbox.log"

mkdir -p "$LOOPBOX_CONFIG_DIR" "$BENCH_APP_DIR" "$RESULTS_DIR"

cat > "$BENCH_APP_DIR/index.html" <<'HTML'
<!doctype html>
<html><head><meta charset="utf-8"><title>loopbox benchmark</title></head>
<body>loopbox benchmark payload</body>
</html>
HTML

SERVICE_COMMAND="python3 -m http.server ${SERVICE_PORT} --bind ${PROJECT_IP}"
WORKDIR_TOML="$(toml_quote "$BENCH_APP_DIR")"
COMMAND_TOML="$(toml_quote "$SERVICE_COMMAND")"

cat > "$LOOPBOX_CONFIG_FILE" <<EOF
[global]
domain_suffix = "localhost"
ip_base = "127.0.0."
ip_range_start = 2
ip_range_end = 254

[global.agent_api]
enabled = true
port = ${AGENT_PORT}
auth_enabled = true

[projects.${PROJECT_NAME}]
dir = ${WORKDIR_TOML}
ip = "${PROJECT_IP}"

[[projects.${PROJECT_NAME}.services]]
name = "${SERVICE_NAME}"
command = ${COMMAND_TOML}
workdir = ${WORKDIR_TOML}

[[projects.${PROJECT_NAME}.services.ports]]
port = ${SERVICE_PORT}
protocol = "http1"
EOF

log "Launching isolated Loopbox instance"
log "  binary: $LOOPBOX_BIN"
log "  xdg_config_home: $XDG_CONFIG_HOME_DIR"
XDG_CONFIG_HOME="$XDG_CONFIG_HOME_DIR" "$LOOPBOX_BIN" >"$LOOPBOX_STDOUT_LOG" 2>&1 &
LOOPBOX_PID="$!"

if ! wait_for_discovery_file 30; then
  status=$?
  if [[ "$status" -eq 2 ]]; then
    die "Loopbox process exited before writing agent API discovery file. See log: $LOOPBOX_STDOUT_LOG"
  fi
  if [[ "$status" -eq 3 ]]; then
    die "Loopbox could not bind the Agent API port. Choose a different --agent-port or review permissions. See log: $LOOPBOX_STDOUT_LOG"
  fi
  die "Timed out waiting for agent API discovery file. See log: $LOOPBOX_STDOUT_LOG"
fi

BASE_URL="$(jq -r '.base_url // empty' "$DISCOVERY_FILE")"
AUTH_ENABLED="$(jq -r '.auth_enabled // false' "$DISCOVERY_FILE")"
[[ -n "$BASE_URL" ]] || die "Discovery file missing base_url: $DISCOVERY_FILE"

if [[ "$AUTH_ENABLED" == "true" ]]; then
  wait_for_file "$TOKEN_FILE" 10 || die "Timed out waiting for agent API token file."
  TOKEN="$(cat "$TOKEN_FILE")"
  [[ -n "$TOKEN" ]] || die "Agent API token file is empty: $TOKEN_FILE"
fi

wait_for_health_endpoint || die "Agent API health endpoint did not become ready."

HEALTH_JSON="$(curl -sS --fail "${BASE_URL}/v1/health")"
PROXY_PORT="$(printf '%s' "$HEALTH_JSON" | jq -r '.reverse_proxy.bind_port')"
PROXY_RUNNING="$(printf '%s' "$HEALTH_JSON" | jq -r '.reverse_proxy.running')"

if [[ "$PROXY_RUNNING" != "true" || "$PROXY_PORT" == "0" || -z "$PROXY_PORT" ]]; then
  die "Reverse proxy is not running (port=${PROXY_PORT}). Stop any existing Loopbox instance using ports 80/18080 and retry. See log: $LOOPBOX_STDOUT_LOG"
fi

log "Starting benchmark project via Agent API"
start_benchmark_project
wait_for_service_running || die "Benchmark service failed to reach running state."

DIRECT_URL="http://${PROJECT_IP}:${SERVICE_PORT}/index.html"
PROXY_URL="http://127.0.0.1:${PROXY_PORT}/index.html"
PROXY_HOST="${SERVICE_NAME}.${PROJECT_NAME}.localhost"

log "Collecting memory baseline samples"
sample_memory_mb "$LOOPBOX_PID" "$RESULTS_DIR/memory_idle_mb.txt" 9 0.2

log "Running warmup (${WARMUP} direct/proxy pairs)"
run_interleaved_samples \
  "$WARMUP" \
  "$DIRECT_URL" \
  "$PROXY_URL" \
  "$RESULTS_DIR/warmup_direct_ms.txt" \
  "$RESULTS_DIR/warmup_proxy_ms.txt" \
  "$PROXY_HOST"

log "Running benchmark (${REQUESTS} direct/proxy pairs)"
run_interleaved_samples \
  "$REQUESTS" \
  "$DIRECT_URL" \
  "$PROXY_URL" \
  "$RESULTS_DIR/direct_ms.txt" \
  "$RESULTS_DIR/proxy_ms.txt" \
  "$PROXY_HOST"

log "Collecting post-benchmark memory samples"
sample_memory_mb "$LOOPBOX_PID" "$RESULTS_DIR/memory_post_mb.txt" 9 0.2

read -r DIRECT_N DIRECT_AVG DIRECT_P50 DIRECT_P95 DIRECT_P99 DIRECT_MIN DIRECT_MAX < <(
  calc_stats_from_file "$RESULTS_DIR/direct_ms.txt"
)
read -r PROXY_N PROXY_AVG PROXY_P50 PROXY_P95 PROXY_P99 PROXY_MIN PROXY_MAX < <(
  calc_stats_from_file "$RESULTS_DIR/proxy_ms.txt"
)
read -r MEM_IDLE_N MEM_IDLE_AVG MEM_IDLE_P50 MEM_IDLE_P95 MEM_IDLE_P99 MEM_IDLE_MIN MEM_IDLE_MAX < <(
  calc_stats_from_file "$RESULTS_DIR/memory_idle_mb.txt"
)
read -r MEM_POST_N MEM_POST_AVG MEM_POST_P50 MEM_POST_P95 MEM_POST_P99 MEM_POST_MIN MEM_POST_MAX < <(
  calc_stats_from_file "$RESULTS_DIR/memory_post_mb.txt"
)

OVERHEAD_AVG="$(float_sub "$PROXY_AVG" "$DIRECT_AVG")"
OVERHEAD_P50="$(float_sub "$PROXY_P50" "$DIRECT_P50")"
OVERHEAD_P95="$(float_sub "$PROXY_P95" "$DIRECT_P95")"
OVERHEAD_P99="$(float_sub "$PROXY_P99" "$DIRECT_P99")"

MEMORY_LOWER_BOUND="$(float_sub "$MEMORY_TARGET_MB" "$MEMORY_TOLERANCE_MB")"
MEMORY_UPPER_BOUND="$(float_add "$MEMORY_TARGET_MB" "$MEMORY_TOLERANCE_MB")"
MEMORY_DELTA_IDLE="$(float_sub "$MEM_IDLE_P50" "$MEMORY_TARGET_MB")"
MEMORY_DELTA_POST="$(float_sub "$MEM_POST_P50" "$MEMORY_TARGET_MB")"

LATENCY_PASS="FAIL"
if awk -v p50="$OVERHEAD_P50" -v avg="$OVERHEAD_AVG" -v target="$LATENCY_TARGET_MS" 'BEGIN { exit !((p50 < target) && (avg < target)) }'; then
  LATENCY_PASS="PASS"
fi

MEMORY_PASS="FAIL"
if awk \
  -v idle="$MEM_IDLE_P50" \
  -v post="$MEM_POST_P50" \
  -v low="$MEMORY_LOWER_BOUND" \
  -v high="$MEMORY_UPPER_BOUND" \
  'BEGIN { exit !((idle >= low) && (idle <= high) && (post >= low) && (post <= high)) }'
then
  MEMORY_PASS="PASS"
fi

printf '\n'
printf 'Landing page benchmark report\n'
printf '============================\n'
printf 'Loopbox binary:         %s\n' "$LOOPBOX_BIN"
printf 'Loopbox pid:            %s\n' "$LOOPBOX_PID"
printf 'Agent API:              %s\n' "$BASE_URL"
printf 'Reverse proxy port:     %s\n' "$PROXY_PORT"
printf 'Requests per side:      %s\n' "$REQUESTS"
printf '\n'
printf 'Latency (ms)\n'
printf '  direct: avg=%s p50=%s p95=%s p99=%s min=%s max=%s\n' \
  "$DIRECT_AVG" "$DIRECT_P50" "$DIRECT_P95" "$DIRECT_P99" "$DIRECT_MIN" "$DIRECT_MAX"
printf '  proxy:  avg=%s p50=%s p95=%s p99=%s min=%s max=%s\n' \
  "$PROXY_AVG" "$PROXY_P50" "$PROXY_P95" "$PROXY_P99" "$PROXY_MIN" "$PROXY_MAX"
printf '  delta:  avg=%s p50=%s p95=%s p99=%s\n' \
  "$OVERHEAD_AVG" "$OVERHEAD_P50" "$OVERHEAD_P95" "$OVERHEAD_P99"
printf '  claim:  overhead < %sms -> %s\n' "$LATENCY_TARGET_MS" "$LATENCY_PASS"
printf '\n'
printf 'Memory RSS (MB)\n'
printf '  idle: avg=%s p50=%s p95=%s p99=%s min=%s max=%s\n' \
  "$MEM_IDLE_AVG" "$MEM_IDLE_P50" "$MEM_IDLE_P95" "$MEM_IDLE_P99" "$MEM_IDLE_MIN" "$MEM_IDLE_MAX"
printf '  post: avg=%s p50=%s p95=%s p99=%s min=%s max=%s\n' \
  "$MEM_POST_AVG" "$MEM_POST_P50" "$MEM_POST_P95" "$MEM_POST_P99" "$MEM_POST_MIN" "$MEM_POST_MAX"
printf '  claim: ~%sMB (+/-%sMB) -> idle p50 delta=%sMB, post p50 delta=%sMB -> %s\n' \
  "$MEMORY_TARGET_MB" "$MEMORY_TOLERANCE_MB" "$MEMORY_DELTA_IDLE" "$MEMORY_DELTA_POST" "$MEMORY_PASS"
printf '\n'
printf 'Raw artifacts:          %s\n' "$TMP_ROOT"
printf 'Loopbox stdout/stderr:  %s\n' "$LOOPBOX_STDOUT_LOG"

if [[ -n "$OUTPUT_JSON" ]]; then
  mkdir -p "$(dirname "$OUTPUT_JSON")"
  jq -n \
    --arg timestamp_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg loopbox_bin "$LOOPBOX_BIN" \
    --arg loopbox_pid "$LOOPBOX_PID" \
    --arg base_url "$BASE_URL" \
    --arg proxy_port "$PROXY_PORT" \
    --arg requests "$REQUESTS" \
    --arg warmup "$WARMUP" \
    --arg direct_avg_ms "$DIRECT_AVG" \
    --arg direct_p50_ms "$DIRECT_P50" \
    --arg direct_p95_ms "$DIRECT_P95" \
    --arg direct_p99_ms "$DIRECT_P99" \
    --arg proxy_avg_ms "$PROXY_AVG" \
    --arg proxy_p50_ms "$PROXY_P50" \
    --arg proxy_p95_ms "$PROXY_P95" \
    --arg proxy_p99_ms "$PROXY_P99" \
    --arg overhead_avg_ms "$OVERHEAD_AVG" \
    --arg overhead_p50_ms "$OVERHEAD_P50" \
    --arg overhead_p95_ms "$OVERHEAD_P95" \
    --arg overhead_p99_ms "$OVERHEAD_P99" \
    --arg latency_target_ms "$LATENCY_TARGET_MS" \
    --arg latency_pass "$LATENCY_PASS" \
    --arg memory_idle_p50_mb "$MEM_IDLE_P50" \
    --arg memory_post_p50_mb "$MEM_POST_P50" \
    --arg memory_target_mb "$MEMORY_TARGET_MB" \
    --arg memory_tolerance_mb "$MEMORY_TOLERANCE_MB" \
    --arg memory_pass "$MEMORY_PASS" \
    --arg artifacts_dir "$TMP_ROOT" \
    --arg loopbox_log "$LOOPBOX_STDOUT_LOG" \
    '{
      timestamp_utc: $timestamp_utc,
      loopbox: {
        bin: $loopbox_bin,
        pid: $loopbox_pid,
        agent_api: $base_url,
        reverse_proxy_port: $proxy_port
      },
      benchmark: {
        requests: $requests,
        warmup: $warmup
      },
      latency_ms: {
        direct: {avg: $direct_avg_ms, p50: $direct_p50_ms, p95: $direct_p95_ms, p99: $direct_p99_ms},
        proxy: {avg: $proxy_avg_ms, p50: $proxy_p50_ms, p95: $proxy_p95_ms, p99: $proxy_p99_ms},
        overhead: {avg: $overhead_avg_ms, p50: $overhead_p50_ms, p95: $overhead_p95_ms, p99: $overhead_p99_ms},
        claim: {threshold_ms: $latency_target_ms, result: $latency_pass}
      },
      memory_mb: {
        idle_p50: $memory_idle_p50_mb,
        post_p50: $memory_post_p50_mb,
        claim: {target_mb: $memory_target_mb, tolerance_mb: $memory_tolerance_mb, result: $memory_pass}
      },
      artifacts: {
        dir: $artifacts_dir,
        loopbox_log: $loopbox_log
      }
    }' > "$OUTPUT_JSON"
  log "Wrote JSON report: $OUTPUT_JSON"
fi
