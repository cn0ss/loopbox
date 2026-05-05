#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

ENV_FILE="${ENV_FILE:-$PROJECT_DIR/.env.sparkle.local}"
VERSION_INPUT=""
BUMP_MODE="patch"
SKIP_BUILD="false"
SKIP_NOTARIZE="false"
SKIP_UPLOAD="false"
SKIP_R2_SYNC="false"
COPY_MODE="copy"

usage() {
  cat <<'EOF'
One-command public Loopbox release flow.

What it does:
  1) Bumps version in Cargo.toml (default: patch)
  2) Builds/signs/notarizes one public app binary
  3) Generates Sparkle appcast/deltas and uploads changed updater files to R2
  4) Optionally publishes GitHub release when PUBLISH_GITHUB_RELEASE=true

Usage:
  scripts/release-single-binary.sh [options]

Options:
  --version <tag>       Explicit version (for example: v0.3.1). Overrides bump mode.
  --bump <mode>         patch (default), minor, major, none.
  --env-file <path>     Env file for Sparkle/R2/signing config (default: .env.sparkle.local).
  --copy-mode <mode>    copy (default) or move for publish staging.
  --skip-build          Skip dx build/package stage.
  --skip-notarize       Skip notarization/stapling.
  --skip-r2-sync        Skip updater-history sync from R2 before publish.
  --skip-upload         Do not upload changed updater files to R2.
  -h, --help            Show help.

Environment overrides:
  DIOXUS_CLI_BIN
  ENV_FILE
EOF
}

expand_path() {
  local base="$1"
  local value="$2"
  if [[ "$value" == ~* ]]; then
    value="${value/#\~/$HOME}"
  fi
  if [[ "$value" == /* ]]; then
    printf '%s\n' "$value"
  else
    printf '%s/%s\n' "$base" "$value"
  fi
}

normalize_semver() {
  local raw="$1"
  raw="${raw#v}"
  raw="${raw#V}"
  if [[ "$raw" =~ ^[0-9]+(\.[0-9]+){2}$ ]]; then
    printf '%s\n' "$raw"
    return 0
  fi
  return 1
}

read_manifest_version() {
  local manifest="$1"
  sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -n1
}

write_manifest_version() {
  local manifest="$1"
  local version="$2"
  local tmp
  tmp="$(mktemp)"

  awk -v version="$version" '
    BEGIN { replaced = 0 }
    {
      if (replaced == 0 && $0 ~ /^version = "/) {
        print "version = \"" version "\""
        replaced = 1
      } else {
        print $0
      }
    }
    END {
      if (replaced == 0) {
        exit 2
      }
    }
  ' "$manifest" > "$tmp"

  mv "$tmp" "$manifest"
}

bump_semver() {
  local version="$1"
  local mode="$2"
  local major minor patch
  IFS='.' read -r major minor patch <<< "$version"

  case "$mode" in
    patch)
      patch=$((patch + 1))
      ;;
    minor)
      minor=$((minor + 1))
      patch=0
      ;;
    major)
      major=$((major + 1))
      minor=0
      patch=0
      ;;
    none)
      ;;
    *)
      echo "Unsupported bump mode: $mode" >&2
      exit 1
      ;;
  esac

  printf '%d.%d.%d\n' "$major" "$minor" "$patch"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION_INPUT="${2:-}"
      shift 2
      ;;
    --bump)
      BUMP_MODE="${2:-}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --copy-mode)
      COPY_MODE="${2:-}"
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

case "$BUMP_MODE" in
  patch|minor|major|none) ;;
  *)
    echo "--bump must be one of: patch, minor, major, none" >&2
    exit 1
    ;;
esac

if [[ "$COPY_MODE" != "copy" && "$COPY_MODE" != "move" ]]; then
  echo "--copy-mode must be one of: copy, move" >&2
  exit 1
fi

ENV_FILE="$(expand_path "$PROJECT_DIR" "$ENV_FILE")"
MANIFEST="$PROJECT_DIR/Cargo.toml"

if [[ ! -f "$MANIFEST" ]]; then
  echo "Cargo.toml not found: $MANIFEST" >&2
  exit 1
fi
if [[ ! -f "$ENV_FILE" ]]; then
  echo "Env file not found: $ENV_FILE" >&2
  echo "Create it from .env.sparkle.local.example." >&2
  exit 1
fi

current_version="$(read_manifest_version "$MANIFEST")"
if [[ -z "$current_version" ]]; then
  echo "Failed to read version from $MANIFEST" >&2
  exit 1
fi

current_semver="$(normalize_semver "$current_version" || true)"
if [[ -z "$current_semver" ]]; then
  echo "Current version must be semver (X.Y.Z): $current_version" >&2
  exit 1
fi

if [[ -n "$VERSION_INPUT" ]]; then
  target_semver="$(normalize_semver "$VERSION_INPUT" || true)"
  if [[ -z "$target_semver" ]]; then
    echo "--version must be semver (X.Y.Z or vX.Y.Z)." >&2
    exit 1
  fi
else
  target_semver="$(bump_semver "$current_semver" "$BUMP_MODE")"
fi

if [[ "$target_semver" != "$current_semver" ]]; then
  write_manifest_version "$MANIFEST" "$target_semver"
  echo "Updated version: $current_semver -> $target_semver"
else
  echo "Version unchanged: $target_semver"
fi

release_tag="v$target_semver"

pipeline_args=(
  --project-dir "$PROJECT_DIR"
  --env-file "$ENV_FILE"
  --version "$release_tag"
  --copy-mode "$COPY_MODE"
)
if [[ "$SKIP_BUILD" == "true" ]]; then
  pipeline_args+=(--skip-build)
fi
if [[ "$SKIP_NOTARIZE" == "true" ]]; then
  pipeline_args+=(--skip-notarize)
fi
if [[ "$SKIP_R2_SYNC" == "true" ]]; then
  pipeline_args+=(--skip-r2-sync)
fi
if [[ "$SKIP_UPLOAD" == "true" ]]; then
  pipeline_args+=(--skip-upload)
fi

echo ">> Running Sparkle release pipeline ($release_tag)"
echo "   source: $PROJECT_DIR"
"$SCRIPT_DIR/release-sparkle-cloudflare.sh" "${pipeline_args[@]}"

echo
echo "Release completed: $release_tag"
echo "Source: $PROJECT_DIR"
