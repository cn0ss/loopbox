#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage: $(basename "$0") [--image <postgres-image>] [--ip-a <127.0.0.x>] [--ip-b <127.0.0.y>] [--port <host-port>] [--keep-containers]

Validates Docker port reuse for loopback-isolated sandboxes by running two Postgres
containers bound to different loopback IPs on the same host port.

Examples:
  $(basename "$0")
  $(basename "$0") --ip-a 127.0.0.61 --ip-b 127.0.0.62 --port 5432
USAGE
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "SKIP: Missing optional command: $1" >&2
    exit 0
  fi
}

wait_for_postgres() {
  local container="$1"
  local attempts=0
  local max_attempts=40
  until docker exec "$container" pg_isready -U postgres >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [[ "$attempts" -ge "$max_attempts" ]]; then
      echo "Timed out waiting for Postgres readiness in container '$container'." >&2
      return 1
    fi
    sleep 1
  done
}

can_connect_tcp() {
  local host="$1"
  local port="$2"
  if command -v nc >/dev/null 2>&1; then
    nc -z "$host" "$port" >/dev/null 2>&1
    return $?
  fi

  python3 - "$host" "$port" <<'PY'
import socket
import sys

host = sys.argv[1]
port = int(sys.argv[2])
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.settimeout(1.5)
try:
    sock.connect((host, port))
    sys.exit(0)
except OSError:
    sys.exit(1)
finally:
    sock.close()
PY
}

can_bind_loopback_ip() {
  local host="$1"
  python3 - "$host" <<'PY'
import socket
import sys

host = sys.argv[1]
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    sock.bind((host, 0))
    sys.exit(0)
except OSError:
    sys.exit(1)
finally:
    sock.close()
PY
}

require_cmd docker
require_cmd python3

IMAGE="postgres:16-alpine"
IP_A="127.0.0.61"
IP_B="127.0.0.62"
HOST_PORT="5432"
KEEP_CONTAINERS="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image)
      IMAGE="$2"
      shift 2
      ;;
    --ip-a)
      IP_A="$2"
      shift 2
      ;;
    --ip-b)
      IP_B="$2"
      shift 2
      ;;
    --port)
      HOST_PORT="$2"
      shift 2
      ;;
    --keep-containers)
      KEEP_CONTAINERS="true"
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

if [[ "$IP_A" == "$IP_B" ]]; then
  echo "--ip-a and --ip-b must differ." >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "SKIP: Docker daemon is not reachable. Start Docker Desktop/Engine to run this optional smoke." >&2
  exit 0
fi

MISSING_ALIASES=()
if ! can_bind_loopback_ip "$IP_A"; then
  MISSING_ALIASES+=("$IP_A")
fi
if ! can_bind_loopback_ip "$IP_B"; then
  MISSING_ALIASES+=("$IP_B")
fi

if [[ "${#MISSING_ALIASES[@]}" -gt 0 ]]; then
  echo "SKIP: Missing loopback alias(es): ${MISSING_ALIASES[*]}." >&2
  echo "Run Loopbox System Setup for sandboxes using these IPs, or manually add the loopback aliases, then rerun this optional smoke." >&2
  exit 0
fi

STAMP="$(date +%s)"
CONTAINER_A="loopbox-smoke-pg-a-${STAMP}"
CONTAINER_B="loopbox-smoke-pg-b-${STAMP}"

cleanup() {
  if [[ "$KEEP_CONTAINERS" == "true" ]]; then
    return
  fi
  docker rm -f "$CONTAINER_A" >/dev/null 2>&1 || true
  docker rm -f "$CONTAINER_B" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Starting smoke test with image '$IMAGE' on ${IP_A}:${HOST_PORT} and ${IP_B}:${HOST_PORT}."

docker run -d \
  --name "$CONTAINER_A" \
  -e POSTGRES_PASSWORD=loopbox \
  -e POSTGRES_DB=sandbox_a \
  -p "${IP_A}:${HOST_PORT}:5432" \
  "$IMAGE" >/dev/null

docker run -d \
  --name "$CONTAINER_B" \
  -e POSTGRES_PASSWORD=loopbox \
  -e POSTGRES_DB=sandbox_b \
  -p "${IP_B}:${HOST_PORT}:5432" \
  "$IMAGE" >/dev/null

wait_for_postgres "$CONTAINER_A"
wait_for_postgres "$CONTAINER_B"

if ! can_connect_tcp "$IP_A" "$HOST_PORT"; then
  echo "TCP probe failed for ${IP_A}:${HOST_PORT}." >&2
  exit 1
fi
if ! can_connect_tcp "$IP_B" "$HOST_PORT"; then
  echo "TCP probe failed for ${IP_B}:${HOST_PORT}." >&2
  exit 1
fi

echo "PASS: Both containers are reachable on distinct loopback IPs using host port ${HOST_PORT}."
echo "  - ${CONTAINER_A} -> ${IP_A}:${HOST_PORT}"
echo "  - ${CONTAINER_B} -> ${IP_B}:${HOST_PORT}"

if [[ "$KEEP_CONTAINERS" == "true" ]]; then
  echo "Containers kept for manual inspection (--keep-containers enabled)."
fi
