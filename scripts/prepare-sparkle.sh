#!/usr/bin/env bash
set -euo pipefail

APP_PATH=""
SPARKLE_FRAMEWORK_INPUT=""
SPARKLE_FEED_URL=""
SPARKLE_PUBLIC_KEY=""
SPARKLE_AUTO_CHECKS=""

usage() {
  cat <<'EOF'
Embed Sparkle.framework into a macOS app bundle and patch Sparkle Info.plist keys.

Usage:
  scripts/prepare-sparkle.sh \
    --app <path-to-App.app> \
    --framework <path-to-Sparkle.framework | path-containing-Sparkle.framework> \
    --feed-url <https://updates.example.com/appcast.xml> \
    --public-key <sparkle-ed25519-public-key> \
    [--auto-checks true|false]

Required:
  --app         App bundle path (for example: target/dx/loopbox/release/macos/Loopbox.app)
  --framework   Sparkle framework directory or parent directory containing Sparkle.framework
  --feed-url    Sparkle appcast URL
  --public-key  Sparkle EdDSA public key

Optional:
  --auto-checks Set SUEnableAutomaticChecks (true or false)
  -h, --help    Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      APP_PATH="${2:-}"
      shift 2
      ;;
    --framework)
      SPARKLE_FRAMEWORK_INPUT="${2:-}"
      shift 2
      ;;
    --feed-url)
      SPARKLE_FEED_URL="${2:-}"
      shift 2
      ;;
    --public-key)
      SPARKLE_PUBLIC_KEY="${2:-}"
      shift 2
      ;;
    --auto-checks)
      SPARKLE_AUTO_CHECKS="${2:-}"
      shift 2
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

if [[ -z "$APP_PATH" || -z "$SPARKLE_FRAMEWORK_INPUT" || -z "$SPARKLE_FEED_URL" || -z "$SPARKLE_PUBLIC_KEY" ]]; then
  echo "Missing required arguments." >&2
  usage
  exit 1
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "App bundle not found: $APP_PATH" >&2
  exit 1
fi

resolve_framework_path() {
  local input="$1"

  if [[ -d "$input" && "$(basename "$input")" == "Sparkle.framework" ]]; then
    echo "$input"
    return 0
  fi

  if [[ -d "$input/Sparkle.framework" ]]; then
    echo "$input/Sparkle.framework"
    return 0
  fi

  return 1
}

SPARKLE_FRAMEWORK_PATH="$(resolve_framework_path "$SPARKLE_FRAMEWORK_INPUT" || true)"
if [[ -z "$SPARKLE_FRAMEWORK_PATH" || ! -d "$SPARKLE_FRAMEWORK_PATH" ]]; then
  echo "Sparkle.framework not found from input: $SPARKLE_FRAMEWORK_INPUT" >&2
  exit 1
fi

PLIST_PATH="$APP_PATH/Contents/Info.plist"
if [[ ! -f "$PLIST_PATH" ]]; then
  echo "Info.plist not found at: $PLIST_PATH" >&2
  exit 1
fi

PLIST_BUDDY="/usr/libexec/PlistBuddy"
if [[ ! -x "$PLIST_BUDDY" ]]; then
  echo "PlistBuddy not found at: $PLIST_BUDDY" >&2
  exit 1
fi

plist_set_string() {
  local key="$1"
  local value="$2"
  local escaped="$value"
  escaped="${escaped//\\/\\\\}"
  escaped="${escaped//\"/\\\"}"

  if "$PLIST_BUDDY" -c "Print :$key" "$PLIST_PATH" >/dev/null 2>&1; then
    "$PLIST_BUDDY" -c "Set :$key \"$escaped\"" "$PLIST_PATH"
  else
    "$PLIST_BUDDY" -c "Add :$key string \"$escaped\"" "$PLIST_PATH"
  fi
}

plist_set_bool() {
  local key="$1"
  local raw="$2"
  local lowered
  lowered="$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')"
  local normalized=""

  case "$lowered" in
    true|yes|1)
      normalized="true"
      ;;
    false|no|0)
      normalized="false"
      ;;
    *)
      echo "Invalid boolean for --auto-checks: $raw" >&2
      exit 1
      ;;
  esac

  if "$PLIST_BUDDY" -c "Print :$key" "$PLIST_PATH" >/dev/null 2>&1; then
    "$PLIST_BUDDY" -c "Set :$key $normalized" "$PLIST_PATH"
  else
    "$PLIST_BUDDY" -c "Add :$key bool $normalized" "$PLIST_PATH"
  fi
}

FRAMEWORKS_DIR="$APP_PATH/Contents/Frameworks"
DEST_FRAMEWORK="$FRAMEWORKS_DIR/Sparkle.framework"
mkdir -p "$FRAMEWORKS_DIR"
rm -rf "$DEST_FRAMEWORK"
ditto "$SPARKLE_FRAMEWORK_PATH" "$DEST_FRAMEWORK"

plist_set_string "SUFeedURL" "$SPARKLE_FEED_URL"
plist_set_string "SUPublicEDKey" "$SPARKLE_PUBLIC_KEY"

if [[ -n "$SPARKLE_AUTO_CHECKS" ]]; then
  plist_set_bool "SUEnableAutomaticChecks" "$SPARKLE_AUTO_CHECKS"
fi

echo "Sparkle prepared:"
echo "  app: $APP_PATH"
echo "  framework: $DEST_FRAMEWORK"
echo "  feed: $SPARKLE_FEED_URL"
echo "  public key: set"
if [[ -n "$SPARKLE_AUTO_CHECKS" ]]; then
  echo "  SUEnableAutomaticChecks: $SPARKLE_AUTO_CHECKS"
fi
