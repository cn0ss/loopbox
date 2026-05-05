#!/usr/bin/env bash
set -euo pipefail

UPDATES_DIR=""
ARCHIVE_PATH=""
RELEASE_NOTES=""
GENERATE_APPCAST_BIN=""
COPY_MODE="copy"
PLIST_BUDDY="/usr/libexec/PlistBuddy"

usage() {
  cat <<'EOF'
Stage a Sparkle update archive and generate/update appcast + delta files.

Usage:
  scripts/publish-sparkle.sh \
    --updates-dir <dir> \
    --archive <path-to-archive.zip|dmg|aar|tar.*> \
    [--release-notes <path-to-notes.md|html>] \
    [--generate-appcast <path-to-generate_appcast>] \
    [--copy-mode copy|move]

Required:
  --updates-dir        Directory that is or will become your Sparkle updates root.
  --archive            Release archive to publish.

Optional:
  --release-notes      Markdown/HTML notes copied next to archive with matching basename.
  --generate-appcast   Explicit path to Sparkle generate_appcast tool.
  --copy-mode          copy (default) or move staged files into updates directory.
  -h, --help           Show help.

Notes:
  - generate_appcast signs updates using your Sparkle private key in Keychain by default.
  - If Keychain access prompts appear, approve them for release automation.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --updates-dir)
      UPDATES_DIR="${2:-}"
      shift 2
      ;;
    --archive)
      ARCHIVE_PATH="${2:-}"
      shift 2
      ;;
    --release-notes)
      RELEASE_NOTES="${2:-}"
      shift 2
      ;;
    --generate-appcast)
      GENERATE_APPCAST_BIN="${2:-}"
      shift 2
      ;;
    --copy-mode)
      COPY_MODE="${2:-}"
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

if [[ -z "$UPDATES_DIR" || -z "$ARCHIVE_PATH" ]]; then
  echo "Missing required arguments." >&2
  usage
  exit 1
fi

if [[ ! -f "$ARCHIVE_PATH" ]]; then
  echo "Archive not found: $ARCHIVE_PATH" >&2
  exit 1
fi

if [[ -n "$RELEASE_NOTES" && ! -f "$RELEASE_NOTES" ]]; then
  echo "Release notes file not found: $RELEASE_NOTES" >&2
  exit 1
fi

if [[ "$COPY_MODE" != "copy" && "$COPY_MODE" != "move" ]]; then
  echo "--copy-mode must be one of: copy, move" >&2
  exit 1
fi

resolve_generate_appcast() {
  local explicit="$1"
  if [[ -n "$explicit" ]]; then
    if [[ -x "$explicit" ]]; then
      echo "$explicit"
      return 0
    fi
    return 1
  fi

  if command -v generate_appcast >/dev/null 2>&1; then
    command -v generate_appcast
    return 0
  fi

  if [[ -x "/usr/local/bin/generate_appcast" ]]; then
    echo "/usr/local/bin/generate_appcast"
    return 0
  fi

  if [[ -x "/opt/homebrew/bin/generate_appcast" ]]; then
    echo "/opt/homebrew/bin/generate_appcast"
    return 0
  fi

  return 1
}

extract_bundle_version_from_zip() {
  local archive_path="$1"
  local plist_entry=""
  local tmp_plist=""
  local bundle_version=""

  plist_entry="$(
    unzip -Z1 "$archive_path" 2>/dev/null \
      | grep -E '^[^/]+\.app/Contents/Info\.plist$' \
      | grep -v '^__MACOSX/' \
      | head -n1 \
      || true
  )"
  if [[ -z "$plist_entry" ]]; then
    return 1
  fi

  tmp_plist="$(mktemp)"
  if ! unzip -p "$archive_path" "$plist_entry" > "$tmp_plist" 2>/dev/null; then
    rm -f "$tmp_plist"
    return 1
  fi

  bundle_version="$("$PLIST_BUDDY" -c 'Print :CFBundleVersion' "$tmp_plist" 2>/dev/null || true)"
  rm -f "$tmp_plist"
  if [[ -z "$bundle_version" ]]; then
    return 1
  fi

  printf '%s' "$bundle_version"
}

cleanup_duplicate_zip_bundle_versions() {
  if ! command -v unzip >/dev/null 2>&1; then
    return 0
  fi
  if [[ ! -x "$PLIST_BUDDY" ]]; then
    return 0
  fi

  local -a seen_versions=()
  local -a seen_files=()
  local -a removed_files=()
  local archive_path=""

  while IFS= read -r archive_path; do
    local bundle_version=""
    bundle_version="$(extract_bundle_version_from_zip "$archive_path" || true)"
    if [[ -z "$bundle_version" ]]; then
      continue
    fi

    local existing_index="-1"
    local i=0
    for i in "${!seen_versions[@]}"; do
      if [[ "${seen_versions[$i]}" == "$bundle_version" ]]; then
        existing_index="$i"
        break
      fi
    done

    if [[ "$existing_index" != "-1" ]]; then
      local previous_archive="${seen_files[$existing_index]}"
      if [[ "$previous_archive" != "$archive_path" && -f "$previous_archive" ]]; then
        rm -f "$previous_archive"
        removed_files+=("$(basename "$previous_archive") (CFBundleVersion=$bundle_version)")
      fi
      seen_files[$existing_index]="$archive_path"
    else
      seen_versions+=("$bundle_version")
      seen_files+=("$archive_path")
    fi
  done < <(find "$UPDATES_DIR" -maxdepth 1 -type f -name '*.zip' | sort -V)

  if [[ "${#removed_files[@]}" -gt 0 ]]; then
    echo "Removed duplicate archive(s) with identical CFBundleVersion:"
    printf ' - %s\n' "${removed_files[@]}"
  fi
}

GENERATE_APPCAST_BIN="$(resolve_generate_appcast "$GENERATE_APPCAST_BIN" || true)"
if [[ -z "$GENERATE_APPCAST_BIN" ]]; then
  echo "generate_appcast tool was not found. Pass --generate-appcast explicitly." >&2
  exit 1
fi

mkdir -p "$UPDATES_DIR"

ARCHIVE_NAME="$(basename "$ARCHIVE_PATH")"
TARGET_ARCHIVE="$UPDATES_DIR/$ARCHIVE_NAME"

if [[ "$COPY_MODE" == "copy" ]]; then
  cp "$ARCHIVE_PATH" "$TARGET_ARCHIVE"
else
  mv "$ARCHIVE_PATH" "$TARGET_ARCHIVE"
fi

if [[ -n "$RELEASE_NOTES" ]]; then
  NOTES_EXT="${RELEASE_NOTES##*.}"
  case "$NOTES_EXT" in
    md|MD|html|HTML|htm|HTM)
      ;;
    *)
      echo "Release notes should use .md or .html extension." >&2
      exit 1
      ;;
  esac

  BASENAME_NO_EXT="${ARCHIVE_NAME%.*}"
  TARGET_NOTES="$UPDATES_DIR/$BASENAME_NO_EXT.$NOTES_EXT"

  if [[ "$COPY_MODE" == "copy" ]]; then
    cp "$RELEASE_NOTES" "$TARGET_NOTES"
  else
    mv "$RELEASE_NOTES" "$TARGET_NOTES"
  fi
fi

cleanup_duplicate_zip_bundle_versions

"$GENERATE_APPCAST_BIN" "$UPDATES_DIR"

echo "Sparkle publish staging complete."
echo "  updates dir: $UPDATES_DIR"
echo "  archive:     $TARGET_ARCHIVE"
if [[ -n "$RELEASE_NOTES" ]]; then
  echo "  notes:       $TARGET_NOTES"
fi
if [[ -f "$UPDATES_DIR/appcast.xml" ]]; then
  echo "  appcast:     $UPDATES_DIR/appcast.xml"
else
  echo "  appcast:     not found (check SUFeedURL filename and generate_appcast output)"
fi

echo "Generated update files:"
find "$UPDATES_DIR" -maxdepth 1 -type f \( -name '*.xml' -o -name '*.delta' -o -name '*.zip' -o -name '*.dmg' -o -name '*.aar' \) | sort
