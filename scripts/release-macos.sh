#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Loopbox"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/dioxus-cli.sh"
PROJECT_DIR="$(pwd)"
APP_PATH=""
APP_PATH_EXPLICIT="false"
OUT_DIR=""
CARGO_TARGET_DIR=""
FEATURES=""
GITHUB_REPO=""

VERSION=""
IDENTITY=""
NOTARY_PROFILE="loopbox-notary"
SKIP_BUILD="false"
NO_SIGN="false"
NO_NOTARIZE="false"
PUBLISH_RELEASE="false"
SPARKLE_FRAMEWORK=""
SPARKLE_FEED_URL=""
SPARKLE_PUBLIC_KEY=""
SPARKLE_AUTO_CHECKS=""
SPARKLE_ENABLED="false"
NORMALIZED_SHORT_VERSION=""
DERIVED_BUNDLE_VERSION=""

PLIST_BUDDY="/usr/libexec/PlistBuddy"

normalize_release_version() {
  local raw="$1"
  raw="${raw#v}"
  raw="${raw#V}"

  if [[ "$raw" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]]; then
    echo "$raw"
    return 0
  fi

  echo ""
}

derive_bundle_version() {
  local short_version="$1"

  if [[ -z "$short_version" ]]; then
    echo ""
    return 0
  fi

  local major=0
  local minor=0
  local patch=0
  IFS='.' read -r major minor patch <<< "$short_version"
  major="${major:-0}"
  minor="${minor:-0}"
  patch="${patch:-0}"

  if ! [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ && "$patch" =~ ^[0-9]+$ ]]; then
    echo ""
    return 0
  fi

  # CFBundleVersion must be monotonic and should stay above legacy low values.
  # Encode semver into a sortable integer with a high base.
  # Example: 0.1.6 -> 100601, 1.0.0 -> 100000001.
  printf '%d' "$((10#$major * 100000000 + 10#$minor * 100000 + 10#$patch * 100 + 1))"
}

plist_set_string() {
  local plist_path="$1"
  local key="$2"
  local value="$3"
  local escaped="$value"
  escaped="${escaped//\\/\\\\}"
  escaped="${escaped//\"/\\\"}"

  if "$PLIST_BUDDY" -c "Print :$key" "$plist_path" >/dev/null 2>&1; then
    "$PLIST_BUDDY" -c "Set :$key \"$escaped\"" "$plist_path"
  else
    "$PLIST_BUDDY" -c "Add :$key string \"$escaped\"" "$plist_path"
  fi
}

set_bundle_versions() {
  local app_path="$1"
  local short_version="$2"
  local bundle_version="$3"
  local plist_path="$app_path/Contents/Info.plist"

  if [[ ! -x "$PLIST_BUDDY" ]]; then
    echo "PlistBuddy not found at: $PLIST_BUDDY" >&2
    exit 1
  fi

  if [[ ! -f "$plist_path" ]]; then
    echo "Info.plist not found at: $plist_path" >&2
    exit 1
  fi

  plist_set_string "$plist_path" "CFBundleShortVersionString" "$short_version"
  plist_set_string "$plist_path" "CFBundleVersion" "$bundle_version"
}

usage() {
  cat <<'EOF'
Build, optionally sign + notarize, package, and optionally publish a macOS release.

Usage:
  scripts/release-macos.sh [options]

Options:
  --project-dir <path>    Dioxus project root (default: current directory).
  --app-path <path>       App bundle path (default: <project-dir>/dist/Loopbox.app).
  --out-dir <path>        Release artifact output directory (default: <project-dir>/release-artifacts).
  --cargo-target-dir <p>  Optional CARGO_TARGET_DIR for dx bundle.
  --features <list>       Cargo features for dx bundle (default: none).
  --version <tag>          Release/tag label (example: v0.1.1). Defaults to local-<timestamp>.
  --identity <name>        Code signing identity (Developer ID Application: ...).
  --notary-profile <name>  notarytool keychain profile (default: loopbox-notary).
  --skip-build             Skip dx bundle and package current app bundle.
  --no-sign                Skip codesign step.
  --no-notarize            Skip notary submit/staple.
  --publish-release        Create or update GitHub Release via gh CLI.
  --github-repo <owner/repo>
                         GitHub repository for release publish. Required when --publish-release
                         is used from a non-git project directory.
  --sparkle-framework <p>  Path to Sparkle.framework or directory containing it.
  --sparkle-feed-url <u>   Sparkle appcast URL (sets SUFeedURL in Info.plist).
  --sparkle-public-key <k> Sparkle EdDSA public key (sets SUPublicEDKey in Info.plist).
  --sparkle-auto-checks <b>Set SUEnableAutomaticChecks (true/false).
  -h, --help               Show help.

Examples:
  scripts/release-macos.sh --version v0.1.1 --identity "Developer ID Application: <REDACTED>"
  DIOXUS_CLI_BIN="$HOME/.cargo/bin/dx" scripts/release-macos.sh --version v0.1.1 --no-sign --no-notarize
  scripts/release-macos.sh --version v0.1.1 --identity "Developer ID Application: <REDACTED>" --publish-release
  scripts/release-macos.sh --version v0.1.1 --identity "Developer ID Application: <REDACTED>" \
    --sparkle-framework /path/to/Sparkle.framework \
    --sparkle-feed-url https://updates.example.com/appcast.xml \
    --sparkle-public-key '<PUBLIC_KEY>'
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --project-dir)
      PROJECT_DIR="${2:-}"
      shift 2
      ;;
    --app-path)
      APP_PATH="${2:-}"
      APP_PATH_EXPLICIT="true"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --cargo-target-dir)
      CARGO_TARGET_DIR="${2:-}"
      shift 2
      ;;
    --features)
      FEATURES="${2-}"
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --identity)
      IDENTITY="${2:-}"
      shift 2
      ;;
    --notary-profile)
      NOTARY_PROFILE="${2:-}"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD="true"
      shift
      ;;
    --no-sign)
      NO_SIGN="true"
      shift
      ;;
    --no-notarize)
      NO_NOTARIZE="true"
      shift
      ;;
    --publish-release)
      PUBLISH_RELEASE="true"
      shift
      ;;
    --github-repo)
      GITHUB_REPO="${2:-}"
      shift 2
      ;;
    --sparkle-framework)
      SPARKLE_FRAMEWORK="${2:-}"
      shift 2
      ;;
    --sparkle-feed-url)
      SPARKLE_FEED_URL="${2:-}"
      shift 2
      ;;
    --sparkle-public-key)
      SPARKLE_PUBLIC_KEY="${2:-}"
      shift 2
      ;;
    --sparkle-auto-checks)
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

if [[ -z "$PROJECT_DIR" ]]; then
  echo "--project-dir cannot be empty." >&2
  exit 1
fi

if [[ ! -d "$PROJECT_DIR" ]]; then
  echo "Project directory not found: $PROJECT_DIR" >&2
  exit 1
fi

PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"

if [[ ! -f "$PROJECT_DIR/Cargo.toml" ]]; then
  echo "Cargo.toml not found in project directory: $PROJECT_DIR" >&2
  exit 1
fi

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$PROJECT_DIR/dist/Loopbox.app"
elif [[ "$APP_PATH" != /* ]]; then
  APP_PATH="$PROJECT_DIR/$APP_PATH"
fi

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$PROJECT_DIR/release-artifacts"
elif [[ "$OUT_DIR" != /* ]]; then
  OUT_DIR="$PROJECT_DIR/$OUT_DIR"
fi

if [[ -n "$CARGO_TARGET_DIR" && "$CARGO_TARGET_DIR" != /* ]]; then
  CARGO_TARGET_DIR="$PROJECT_DIR/$CARGO_TARGET_DIR"
fi

if [[ -z "$VERSION" ]]; then
  VERSION="local-$(date +%Y%m%d-%H%M%S)"
fi

NORMALIZED_SHORT_VERSION="$(normalize_release_version "$VERSION")"
DERIVED_BUNDLE_VERSION="$(derive_bundle_version "$NORMALIZED_SHORT_VERSION")"

if [[ "$NO_SIGN" == "true" && "$NO_NOTARIZE" == "false" ]]; then
  echo "Cannot notarize without signing. Remove --no-sign or add --no-notarize." >&2
  exit 1
fi

if [[ "$PUBLISH_RELEASE" == "true" && -z "$GITHUB_REPO" && ! -d "$PROJECT_DIR/.git" ]]; then
  echo "Publishing from non-git project requires --github-repo <owner/repo>." >&2
  exit 1
fi

if [[ -n "$SPARKLE_FRAMEWORK$SPARKLE_FEED_URL$SPARKLE_PUBLIC_KEY$SPARKLE_AUTO_CHECKS" ]]; then
  SPARKLE_ENABLED="true"
fi

if [[ "$SPARKLE_ENABLED" == "true" ]]; then
  if [[ -z "$SPARKLE_FRAMEWORK" || -z "$SPARKLE_FEED_URL" || -z "$SPARKLE_PUBLIC_KEY" ]]; then
    echo "Sparkle integration requires --sparkle-framework, --sparkle-feed-url, and --sparkle-public-key." >&2
    exit 1
  fi
fi

if [[ "$NO_SIGN" == "false" && -z "$IDENTITY" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning | awk -F\" '/Developer ID Application:/{print $2; exit}')"
  if [[ -z "$IDENTITY" ]]; then
    echo "No Developer ID Application identity found. Pass --identity explicitly." >&2
    exit 1
  fi
fi

mkdir -p "$OUT_DIR"

if [[ "$SKIP_BUILD" == "false" ]]; then
  DIOXUS_CLI="$(resolve_dioxus_cli)"
  DX_BUILD_ARGS=(bundle --platform macos --package-types macos --release)
  if [[ -n "$FEATURES" ]]; then
    DX_BUILD_ARGS+=(--features "$FEATURES")
  fi

  if [[ -n "$CARGO_TARGET_DIR" ]]; then
    (
      cd "$PROJECT_DIR"
      CARGO_TARGET_DIR="$CARGO_TARGET_DIR" LOOPBOX_RELEASE_VERSION="$VERSION" "$DIOXUS_CLI" "${DX_BUILD_ARGS[@]}"
    )
  else
    (
      cd "$PROJECT_DIR"
      LOOPBOX_RELEASE_VERSION="$VERSION" "$DIOXUS_CLI" "${DX_BUILD_ARGS[@]}"
    )
  fi
fi

if [[ ! -d "$APP_PATH" ]]; then
  legacy_app_path="$PROJECT_DIR/target/dx/loopbox/release/macos/Loopbox.app"
  if [[ "$APP_PATH_EXPLICIT" == "false" && -d "$legacy_app_path" ]]; then
    APP_PATH="$legacy_app_path"
    echo "Using legacy app bundle path: $APP_PATH"
  else
    echo "App bundle not found at $APP_PATH" >&2
    exit 1
  fi
fi

if [[ -n "$NORMALIZED_SHORT_VERSION" && -n "$DERIVED_BUNDLE_VERSION" ]]; then
  set_bundle_versions "$APP_PATH" "$NORMALIZED_SHORT_VERSION" "$DERIVED_BUNDLE_VERSION"
  echo "Set CFBundleShortVersionString=$NORMALIZED_SHORT_VERSION CFBundleVersion=$DERIVED_BUNDLE_VERSION"
else
  echo "Warning: could not derive numeric bundle version from '$VERSION'."
  echo "Keeping existing CFBundleVersion/CFBundleShortVersionString."
fi

if [[ "$SPARKLE_ENABLED" == "true" ]]; then
  PREPARE_ARGS=(
    --app "$APP_PATH"
    --framework "$SPARKLE_FRAMEWORK"
    --feed-url "$SPARKLE_FEED_URL"
    --public-key "$SPARKLE_PUBLIC_KEY"
  )
  if [[ -n "$SPARKLE_AUTO_CHECKS" ]]; then
    PREPARE_ARGS+=(--auto-checks "$SPARKLE_AUTO_CHECKS")
  fi
  "$SCRIPT_DIR/prepare-sparkle.sh" "${PREPARE_ARGS[@]}"
fi

if [[ "$NO_SIGN" == "false" ]]; then
  codesign --remove-signature "$APP_PATH" 2>/dev/null || true
  codesign --force --deep --options runtime --timestamp --sign "$IDENTITY" "$APP_PATH"
  codesign --verify --deep --strict --verbose=2 "$APP_PATH"
fi

if [[ "$NO_NOTARIZE" == "false" ]]; then
  SUBMIT_ZIP="$OUT_DIR/${APP_NAME}-${VERSION}-notary-submit.zip"
  ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$SUBMIT_ZIP"
  xcrun notarytool submit "$SUBMIT_ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP_PATH"
  xcrun stapler validate "$APP_PATH"
  rm -f "$SUBMIT_ZIP"
  spctl -a -t exec -vv "$APP_PATH"
fi

FINAL_ZIP="$OUT_DIR/${APP_NAME}-${VERSION}-macos.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$FINAL_ZIP"
echo "Release artifact: $FINAL_ZIP"

if [[ "$PUBLISH_RELEASE" == "true" ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "gh CLI not found. Install from https://cli.github.com/ and run 'gh auth login'." >&2
    exit 1
  fi

  GH_REPO_ARGS=()
  if [[ -n "$GITHUB_REPO" ]]; then
    GH_REPO_ARGS=(--repo "$GITHUB_REPO")
  fi

  (
    cd "$PROJECT_DIR"
    if gh release view "$VERSION" "${GH_REPO_ARGS[@]}" >/dev/null 2>&1; then
      gh release upload "$VERSION" "$FINAL_ZIP" --clobber "${GH_REPO_ARGS[@]}"
    else
      gh release create "$VERSION" "$FINAL_ZIP" --generate-notes "${GH_REPO_ARGS[@]}"
    fi
  )
fi
