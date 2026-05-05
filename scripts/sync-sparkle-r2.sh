#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

ENV_FILE="$ROOT_DIR/.env.sparkle.local"
UPDATES_DIR_OVERRIDE=""
DELETE_MISSING="false"

usage() {
  cat <<'EOF'
Sync Sparkle updater artifacts from Cloudflare R2 into local updates workspace.

Usage:
  scripts/sync-sparkle-r2.sh [options]

Options:
  --env-file <path>     Path to env file (default: .env.sparkle.local in repo root).
  --updates-dir <path>  Override LOOPBOX_UPDATES_DIR for this run.
  --delete              Delete local files missing from bucket/prefix.
  -h, --help            Show help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --updates-dir)
      UPDATES_DIR_OVERRIDE="${2:-}"
      shift 2
      ;;
    --delete)
      DELETE_MISSING="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Env file not found: $ENV_FILE" >&2
  echo "Create it from .env.sparkle.local.example." >&2
  exit 1
fi

# shellcheck disable=SC1090
set -a
source "$ENV_FILE"
set +a

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Required command not found: $cmd" >&2
    exit 1
  fi
}

require_command aws

required_vars=(
  CLOUDFLARE_R2_BUCKET
  CLOUDFLARE_R2_ACCESS_KEY_ID
  CLOUDFLARE_R2_SECRET_ACCESS_KEY
)

for var_name in "${required_vars[@]}"; do
  if [[ -z "${!var_name:-}" ]]; then
    echo "Missing required env var: $var_name" >&2
    exit 1
  fi
done

if [[ -n "${CLOUDFLARE_R2_ENDPOINT:-}" ]]; then
  R2_ENDPOINT="$CLOUDFLARE_R2_ENDPOINT"
else
  if [[ -z "${CLOUDFLARE_R2_ACCOUNT_ID:-}" ]]; then
    echo "Missing required env var: CLOUDFLARE_R2_ACCOUNT_ID (or set CLOUDFLARE_R2_ENDPOINT)." >&2
    exit 1
  fi
  R2_ENDPOINT="https://${CLOUDFLARE_R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
fi

UPDATES_DIR_RAW="$UPDATES_DIR_OVERRIDE"
if [[ -z "$UPDATES_DIR_RAW" ]]; then
  UPDATES_DIR_RAW="${LOOPBOX_UPDATES_DIR:-}"
fi

if [[ -z "$UPDATES_DIR_RAW" ]]; then
  echo "Missing updates dir. Set LOOPBOX_UPDATES_DIR in env or pass --updates-dir." >&2
  exit 1
fi

UPDATES_DIR_EXPANDED="$UPDATES_DIR_RAW"
if [[ "$UPDATES_DIR_EXPANDED" == ~* ]]; then
  UPDATES_DIR_EXPANDED="${UPDATES_DIR_EXPANDED/#\~/$HOME}"
fi
if [[ "$UPDATES_DIR_EXPANDED" != /* ]]; then
  UPDATES_DIR_EXPANDED="$ROOT_DIR/$UPDATES_DIR_EXPANDED"
fi
mkdir -p "$UPDATES_DIR_EXPANDED"

prefix_clean="${CLOUDFLARE_R2_PREFIX:-}"
prefix_clean="${prefix_clean#/}"
prefix_clean="${prefix_clean%/}"

source_uri="s3://$CLOUDFLARE_R2_BUCKET"
if [[ -n "$prefix_clean" ]]; then
  source_uri="$source_uri/$prefix_clean"
fi

export AWS_ACCESS_KEY_ID="$CLOUDFLARE_R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$CLOUDFLARE_R2_SECRET_ACCESS_KEY"
export AWS_REGION="auto"
export AWS_DEFAULT_REGION="auto"
export AWS_EC2_METADATA_DISABLED="true"

count_files() {
  find "$1" -type f | wc -l | tr -d '[:space:]'
}

before_count="$(count_files "$UPDATES_DIR_EXPANDED")"

sync_args=(
  s3 sync
  "$source_uri"
  "$UPDATES_DIR_EXPANDED"
  --endpoint-url "$R2_ENDPOINT"
)
if [[ "$DELETE_MISSING" == "true" ]]; then
  sync_args+=(--delete)
fi

echo ">> Sync source:      $source_uri"
echo ">> Local updates dir: $UPDATES_DIR_EXPANDED"
aws "${sync_args[@]}"

after_count="$(count_files "$UPDATES_DIR_EXPANDED")"

echo ">> Sync complete. Local file count: $before_count -> $after_count"
