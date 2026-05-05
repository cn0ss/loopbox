#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/dioxus-cli.sh"

ENV_FILE="$ROOT_DIR/.env.sparkle.local"
PROJECT_DIR=""
RELEASE_FEATURES=""
GITHUB_RELEASE_REPO=""
VERSION=""
SKIP_BUILD="false"
SKIP_NOTARIZE="false"
COPY_MODE="copy"
SKIP_UPLOAD="false"
SKIP_R2_SYNC="false"

usage() {
  cat <<'EOF'
Local one-shot Sparkle release pipeline with Cloudflare R2 upload.

What it does:
  1) Sync existing updater artifacts from Cloudflare R2 (default)
  2) Build/sign/notarize Sparkle-enabled macOS app
  3) Generate appcast + delta files
  4) Upload ONLY changed files to Cloudflare R2

Usage:
  scripts/release-sparkle-cloudflare.sh [options]

Options:
  --project-dir <path>   Dioxus project directory to build/publish from (default: repo root).
  --features <list>      Build features for dx build (default: none).
  --github-repo <o/r>    Optional GitHub repo for release publish override.
  --version <tag>       Release version/tag (example: v0.1.4). If omitted, uses Cargo version with leading "v".
  --env-file <path>     Path to env file (default: .env.sparkle.local in repo root).
  --skip-build          Skip build and package currently built app bundle.
  --skip-notarize       Skip notarization/stapling.
  --copy-mode <mode>    copy (default) or move for publish staging.
  --skip-r2-sync        Skip syncing existing updater files from R2 before publish.
  --skip-upload         Build and generate appcast locally, but do not upload to R2.
  -h, --help            Show help.

Environment overrides:
  DIOXUS_CLI_BIN        Path to Dioxus dx when another dx is earlier in PATH.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --project-dir)
      PROJECT_DIR="${2:-}"
      shift 2
      ;;
    --features)
      RELEASE_FEATURES="${2-}"
      shift 2
      ;;
    --github-repo)
      GITHUB_RELEASE_REPO="${2:-}"
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD="true"
      shift
      ;;
    --skip-notarize)
      SKIP_NOTARIZE="true"
      shift
      ;;
    --copy-mode)
      COPY_MODE="${2:-}"
      shift 2
      ;;
    --skip-r2-sync)
      SKIP_R2_SYNC="true"
      shift
      ;;
    --skip-upload)
      SKIP_UPLOAD="true"
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

if [[ "$COPY_MODE" != "copy" && "$COPY_MODE" != "move" ]]; then
  echo "--copy-mode must be one of: copy, move" >&2
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Env file not found: $ENV_FILE" >&2
  echo "Create it from .env.sparkle.local.example." >&2
  exit 1
fi

# shellcheck disable=SC1090
set -a
source "$ENV_FILE"
set +a

if [[ -z "$PROJECT_DIR" ]]; then
  PROJECT_DIR="$ROOT_DIR"
fi
if [[ "$PROJECT_DIR" != /* ]]; then
  PROJECT_DIR="$ROOT_DIR/$PROJECT_DIR"
fi
if [[ ! -d "$PROJECT_DIR" ]]; then
  echo "Project directory not found: $PROJECT_DIR" >&2
  exit 1
fi
PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"
if [[ ! -f "$PROJECT_DIR/Cargo.toml" ]]; then
  echo "Cargo.toml not found in project directory: $PROJECT_DIR" >&2
  echo "Pass --project-dir pointing at the Loopbox app workspace." >&2
  exit 1
fi

if [[ -z "$VERSION" ]]; then
  CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n1)"
  if [[ -z "$CARGO_VERSION" ]]; then
    echo "Failed to detect version from Cargo.toml. Pass --version explicitly." >&2
    exit 1
  fi
  VERSION="v${CARGO_VERSION}"
fi

required_vars=(
  LOOPBOX_RELEASE_IDENTITY
  LOOPBOX_NOTARY_PROFILE
  SPARKLE_FEED_URL
  SPARKLE_PUBLIC_KEY
  LOOPBOX_UPDATES_DIR
  CLOUDFLARE_R2_ACCOUNT_ID
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

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Required command not found: $cmd" >&2
    exit 1
  fi
}

require_command aws
if [[ "$SKIP_BUILD" == "false" ]]; then
  resolve_dioxus_cli >/dev/null
fi

if [[ "$SKIP_NOTARIZE" == "false" ]]; then
  require_command xcrun
fi

normalize_bool() {
  local raw="${1:-}"
  local lowered
  lowered="$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')"
  case "$lowered" in
    true|yes|1|on) echo "true" ;;
    false|no|0|off|"") echo "false" ;;
    *)
      echo "Invalid boolean: $raw" >&2
      exit 1
      ;;
  esac
}

if [[ -z "$GITHUB_RELEASE_REPO" ]]; then
  GITHUB_RELEASE_REPO="${PUBLISH_GITHUB_RELEASE_REPO:-cn0ss/loopbox}"
fi

SPARKLE_AUTO_CHECKS="${SPARKLE_AUTO_CHECKS:-}"
if [[ -n "$SPARKLE_AUTO_CHECKS" ]]; then
  SPARKLE_AUTO_CHECKS="$(normalize_bool "$SPARKLE_AUTO_CHECKS")"
fi
AUTO_INSTALL_SPARKLE="$(normalize_bool "${AUTO_INSTALL_SPARKLE:-true}")"
AUTO_CLEAR_SPARKLE_QUARANTINE="$(normalize_bool "${AUTO_CLEAR_SPARKLE_QUARANTINE:-true}")"
SYNC_FROM_R2_BEFORE_PUBLISH="$(normalize_bool "${SYNC_FROM_R2_BEFORE_PUBLISH:-true}")"
R2_SYNC_DELETE_LOCAL_MISSING="$(normalize_bool "${R2_SYNC_DELETE_LOCAL_MISSING:-false}")"
if [[ "$SKIP_R2_SYNC" == "true" ]]; then
  SYNC_FROM_R2_BEFORE_PUBLISH="false"
fi

UPLOAD_APPCAST_CACHE_CONTROL="${UPLOAD_APPCAST_CACHE_CONTROL:-no-store, no-cache, must-revalidate, max-age=0}"
UPLOAD_ASSET_CACHE_CONTROL="${UPLOAD_ASSET_CACHE_CONTROL:-public, max-age=31536000, immutable}"
UPLOAD_OTHER_CACHE_CONTROL="${UPLOAD_OTHER_CACHE_CONTROL:-public, max-age=3600}"
UPLOAD_PURGE_APPCAST="$(normalize_bool "${UPLOAD_PURGE_APPCAST:-false}")"
CLOUDFLARE_R2_PREFIX="${CLOUDFLARE_R2_PREFIX:-}"
CLOUDFLARE_R2_ENDPOINT="${CLOUDFLARE_R2_ENDPOINT:-https://${CLOUDFLARE_R2_ACCOUNT_ID}.r2.cloudflarestorage.com}"

UPDATES_DIR_EXPANDED="$LOOPBOX_UPDATES_DIR"
if [[ "$UPDATES_DIR_EXPANDED" == ~* ]]; then
  UPDATES_DIR_EXPANDED="${UPDATES_DIR_EXPANDED/#\~/$HOME}"
fi
if [[ "$UPDATES_DIR_EXPANDED" != /* ]]; then
  UPDATES_DIR_EXPANDED="$ROOT_DIR/$UPDATES_DIR_EXPANDED"
fi
mkdir -p "$UPDATES_DIR_EXPANDED"

if [[ "$SYNC_FROM_R2_BEFORE_PUBLISH" == "true" ]]; then
  sync_args=(
    --env-file "$ENV_FILE"
    --updates-dir "$UPDATES_DIR_EXPANDED"
  )
  if [[ "$R2_SYNC_DELETE_LOCAL_MISSING" == "true" ]]; then
    sync_args+=(--delete)
  fi

  echo ">> Syncing updater history from Cloudflare R2"
  "$SCRIPT_DIR/sync-sparkle-r2.sh" "${sync_args[@]}"
fi

bootstrap_output="$(
  "$SCRIPT_DIR/bootstrap-sparkle.sh" \
    --framework-path "${SPARKLE_FRAMEWORK_PATH:-}" \
    --generate-appcast "${LOOPBOX_GENERATE_APPCAST_BIN:-}" \
    --auto-install "$AUTO_INSTALL_SPARKLE" \
    --clear-quarantine "$AUTO_CLEAR_SPARKLE_QUARANTINE"
)"
SPARKLE_FRAMEWORK_RESOLVED="$(echo "$bootstrap_output" | sed -n 's/^framework=//p' | head -n1)"
GENERATE_APPCAST_RESOLVED="$(echo "$bootstrap_output" | sed -n 's/^generate_appcast=//p' | head -n1)"

if [[ -z "$SPARKLE_FRAMEWORK_RESOLVED" || -z "$GENERATE_APPCAST_RESOLVED" ]]; then
  echo "Failed to resolve Sparkle paths from bootstrap-sparkle.sh output:" >&2
  echo "$bootstrap_output" >&2
  exit 1
fi

echo ">> Sparkle framework: $SPARKLE_FRAMEWORK_RESOLVED"
echo ">> Sparkle tool:      $GENERATE_APPCAST_RESOLVED"

MARKER_FILE="$(mktemp)"
touch "$MARKER_FILE"

release_args=(
  --project-dir "$PROJECT_DIR"
  --features "$RELEASE_FEATURES"
  --version "$VERSION"
  --identity "$LOOPBOX_RELEASE_IDENTITY"
  --notary-profile "$LOOPBOX_NOTARY_PROFILE"
  --sparkle-framework "$SPARKLE_FRAMEWORK_RESOLVED"
  --sparkle-feed-url "$SPARKLE_FEED_URL"
  --sparkle-public-key "$SPARKLE_PUBLIC_KEY"
)
if [[ "$SKIP_BUILD" == "true" ]]; then
  release_args+=(--skip-build)
fi
if [[ "$SKIP_NOTARIZE" == "true" ]]; then
  release_args+=(--no-notarize)
fi
if [[ -n "$SPARKLE_AUTO_CHECKS" ]]; then
  release_args+=(--sparkle-auto-checks "$SPARKLE_AUTO_CHECKS")
fi
if [[ "$(normalize_bool "${PUBLISH_GITHUB_RELEASE:-false}")" == "true" ]]; then
  release_args+=(--publish-release --github-repo "$GITHUB_RELEASE_REPO")
fi

echo ">> Building/signing release $VERSION"
"$SCRIPT_DIR/release-macos.sh" "${release_args[@]}"

ARCHIVE_PATH="$PROJECT_DIR/release-artifacts/Loopbox-${VERSION}-macos.zip"
if [[ ! -f "$ARCHIVE_PATH" ]]; then
  echo "Expected release archive not found: $ARCHIVE_PATH" >&2
  exit 1
fi

publish_args=(
  --updates-dir "$UPDATES_DIR_EXPANDED"
  --archive "$ARCHIVE_PATH"
  --copy-mode "$COPY_MODE"
)
if [[ -n "${LOOPBOX_RELEASE_NOTES_FILE:-}" ]]; then
  notes_path="$LOOPBOX_RELEASE_NOTES_FILE"
  if [[ "$notes_path" != /* ]]; then
    notes_path="$ROOT_DIR/$notes_path"
  fi
  publish_args+=(--release-notes "$notes_path")
fi
publish_args+=(--generate-appcast "$GENERATE_APPCAST_RESOLVED")

echo ">> Generating appcast and deltas"
"$SCRIPT_DIR/publish-sparkle.sh" "${publish_args[@]}"

appcast_path="$UPDATES_DIR_EXPANDED/appcast.xml"
if [[ -f "$appcast_path" ]]; then
  latest_appcast_url="$(sed -n 's/.*<enclosure[^>]*url="\([^"]*\)".*/\1/p' "$appcast_path" | head -n1)"
  latest_appcast_version="$(sed -n 's/.*<sparkle:version>\([^<]*\)<\/sparkle:version>.*/\1/p' "$appcast_path" | head -n1)"
  current_archive_name="$(basename "$ARCHIVE_PATH")"

  if [[ -n "$latest_appcast_url" ]]; then
    echo ">> Appcast latest enclosure: $latest_appcast_url"
  fi
  if [[ -n "$latest_appcast_version" ]]; then
    echo ">> Appcast latest sparkle:version: $latest_appcast_version"
  fi
  if [[ -n "$latest_appcast_url" && "$latest_appcast_url" != *"$current_archive_name" ]]; then
    echo "WARNING: appcast latest enclosure does not reference current archive '$current_archive_name'." >&2
    echo "This usually means CFBundleVersion did not increase between releases." >&2
  fi
fi

changed_files=()
while IFS= read -r file_path; do
  changed_files+=("$file_path")
done < <(find "$UPDATES_DIR_EXPANDED" -type f -newer "$MARKER_FILE" | sort)
rm -f "$MARKER_FILE"

if [[ "${#changed_files[@]}" -eq 0 ]]; then
  echo "No changed updater files detected after publish step. Nothing to upload."
  exit 0
fi

if [[ "$SKIP_UPLOAD" == "true" ]]; then
  echo ">> Upload skipped (--skip-upload). Changed files:"
  printf '%s\n' "${changed_files[@]}"
  exit 0
fi

export AWS_ACCESS_KEY_ID="$CLOUDFLARE_R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$CLOUDFLARE_R2_SECRET_ACCESS_KEY"
export AWS_REGION="auto"
export AWS_DEFAULT_REGION="auto"
export AWS_EC2_METADATA_DISABLED="true"

prefix_clean="${CLOUDFLARE_R2_PREFIX#/}"
prefix_clean="${prefix_clean%/}"

upload_one() {
  local abs_path="$1"
  local rel_path="${abs_path#"$UPDATES_DIR_EXPANDED"/}"
  local object_key="$rel_path"
  if [[ -n "$prefix_clean" ]]; then
    object_key="$prefix_clean/$rel_path"
  fi

  local cache_control="$UPLOAD_OTHER_CACHE_CONTROL"
  local content_type=""
  case "$rel_path" in
    appcast.xml|*.xml)
      cache_control="$UPLOAD_APPCAST_CACHE_CONTROL"
      content_type="application/xml"
      ;;
    *.zip)
      cache_control="$UPLOAD_ASSET_CACHE_CONTROL"
      content_type="application/zip"
      ;;
    *.dmg)
      cache_control="$UPLOAD_ASSET_CACHE_CONTROL"
      content_type="application/x-apple-diskimage"
      ;;
    *.delta)
      cache_control="$UPLOAD_ASSET_CACHE_CONTROL"
      content_type="application/octet-stream"
      ;;
    *.html|*.htm)
      cache_control="$UPLOAD_OTHER_CACHE_CONTROL"
      content_type="text/html; charset=utf-8"
      ;;
    *.md)
      cache_control="$UPLOAD_OTHER_CACHE_CONTROL"
      content_type="text/markdown; charset=utf-8"
      ;;
  esac

  local destination="s3://$CLOUDFLARE_R2_BUCKET/$object_key"
  if [[ -n "$content_type" ]]; then
    aws s3 cp "$abs_path" "$destination" \
      --endpoint-url "$CLOUDFLARE_R2_ENDPOINT" \
      --cache-control "$cache_control" \
      --content-type "$content_type"
  else
    aws s3 cp "$abs_path" "$destination" \
      --endpoint-url "$CLOUDFLARE_R2_ENDPOINT" \
      --cache-control "$cache_control"
  fi

  echo "Uploaded: $destination"
}

echo ">> Uploading ${#changed_files[@]} changed file(s) to Cloudflare R2"
for file_path in "${changed_files[@]}"; do
  upload_one "$file_path"
done

if [[ "$UPLOAD_PURGE_APPCAST" == "true" ]]; then
  if [[ -z "${CLOUDFLARE_ZONE_ID:-}" || -z "${CLOUDFLARE_API_TOKEN:-}" ]]; then
    echo "UPLOAD_PURGE_APPCAST=true requires CLOUDFLARE_ZONE_ID and CLOUDFLARE_API_TOKEN." >&2
    exit 1
  fi
  require_command curl
  require_command jq

  echo ">> Purging Cloudflare cache for feed URL: $SPARKLE_FEED_URL"
  purge_response="$(curl -sS -X POST "https://api.cloudflare.com/client/v4/zones/$CLOUDFLARE_ZONE_ID/purge_cache" \
    -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
    -H "Content-Type: application/json" \
    --data "{\"files\":[\"$SPARKLE_FEED_URL\"]}")"
  purge_success="$(echo "$purge_response" | jq -r '.success // false')"
  if [[ "$purge_success" != "true" ]]; then
    echo "Cloudflare purge failed: $purge_response" >&2
    exit 1
  fi
fi

echo
echo "Sparkle release pipeline finished."
echo "Version: $VERSION"
echo "Feed URL: $SPARKLE_FEED_URL"
echo "Changed files uploaded:"
printf ' - %s\n' "${changed_files[@]}"
