#!/usr/bin/env bash
set -euo pipefail

SPARKLE_FRAMEWORK_INPUT=""
GENERATE_APPCAST_INPUT=""
AUTO_INSTALL="true"
CLEAR_QUARANTINE="true"

usage() {
  cat <<'EOF'
Resolve Sparkle framework + generate_appcast tool paths.
Optionally installs Sparkle via Homebrew cask if missing.

Usage:
  scripts/bootstrap-sparkle.sh [options]

Options:
  --framework-path <path>      Optional preferred Sparkle.framework path.
  --generate-appcast <path>    Optional preferred generate_appcast path.
  --auto-install <true|false>  Auto-install Sparkle via brew cask when missing (default: true).
  --clear-quarantine <true|false>
                               Remove com.apple.quarantine from resolved Sparkle paths (default: true).
  -h, --help                   Show help.

Output:
  framework=<resolved-framework-path>
  generate_appcast=<resolved-generate_appcast-path>
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --framework-path)
      SPARKLE_FRAMEWORK_INPUT="${2:-}"
      shift 2
      ;;
    --generate-appcast)
      GENERATE_APPCAST_INPUT="${2:-}"
      shift 2
      ;;
    --auto-install)
      AUTO_INSTALL="${2:-}"
      shift 2
      ;;
    --clear-quarantine)
      CLEAR_QUARANTINE="${2:-}"
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

AUTO_INSTALL="$(normalize_bool "$AUTO_INSTALL")"
CLEAR_QUARANTINE="$(normalize_bool "$CLEAR_QUARANTINE")"

resolve_framework_from_input() {
  local input="$1"
  [[ -z "$input" ]] && return 1

  if [[ -d "$input" && "$(basename "$input")" == "Sparkle.framework" ]]; then
    echo "$input"
    return 0
  fi
  if [[ -d "$input/Sparkle.framework" ]]; then
    echo "$input/Sparkle.framework"
    return 0
  fi
  if [[ -d "$input/Contents/Frameworks/Sparkle.framework" ]]; then
    echo "$input/Contents/Frameworks/Sparkle.framework"
    return 0
  fi
  return 1
}

resolve_generate_appcast_from_input() {
  local input="$1"
  [[ -z "$input" ]] && return 1

  if [[ -x "$input" ]]; then
    echo "$input"
    return 0
  fi
  if [[ -x "$input/generate_appcast" ]]; then
    echo "$input/generate_appcast"
    return 0
  fi
  if [[ -x "$input/Contents/bin/generate_appcast" ]]; then
    echo "$input/Contents/bin/generate_appcast"
    return 0
  fi
  return 1
}

latest_cask_sparkle_app() {
  local cask_root="$1"
  if [[ ! -d "$cask_root" ]]; then
    return 1
  fi

  local best=""
  while IFS= read -r app; do
    best="$app"
  done < <(find "$cask_root" -maxdepth 3 -type d -path "*/Sparkle.app" | sort -V)

  if [[ -n "$best" ]]; then
    echo "$best"
    return 0
  fi
  return 1
}

detect_framework_path() {
  local from_input=""
  from_input="$(resolve_framework_from_input "$SPARKLE_FRAMEWORK_INPUT" || true)"
  if [[ -n "$from_input" ]]; then
    echo "$from_input"
    return 0
  fi

  local candidates=(
    "/Applications/Sparkle.app"
    "$HOME/Applications/Sparkle.app"
  )

  local opt_cask=""
  local usr_cask=""
  opt_cask="$(latest_cask_sparkle_app "/opt/homebrew/Caskroom/sparkle" || true)"
  usr_cask="$(latest_cask_sparkle_app "/usr/local/Caskroom/sparkle" || true)"
  if [[ -n "$opt_cask" ]]; then
    candidates+=("$opt_cask")
  fi
  if [[ -n "$usr_cask" ]]; then
    candidates+=("$usr_cask")
  fi

  local opt_framework=""
  local usr_framework=""
  opt_framework="$(find /opt/homebrew/Caskroom/sparkle -maxdepth 4 -type d -name Sparkle.framework 2>/dev/null | sort -V | tail -n1)"
  usr_framework="$(find /usr/local/Caskroom/sparkle -maxdepth 4 -type d -name Sparkle.framework 2>/dev/null | sort -V | tail -n1)"
  if [[ -n "$opt_framework" ]]; then
    candidates+=("$opt_framework")
  fi
  if [[ -n "$usr_framework" ]]; then
    candidates+=("$usr_framework")
  fi

  for base in "${candidates[@]}"; do
    if [[ -d "$base" && "$(basename "$base")" == "Sparkle.framework" ]]; then
      echo "$base"
      return 0
    fi
    if [[ -d "$base/Contents/Frameworks/Sparkle.framework" ]]; then
      echo "$base/Contents/Frameworks/Sparkle.framework"
      return 0
    fi
  done
  return 1
}

detect_generate_appcast_path() {
  local from_input=""
  from_input="$(resolve_generate_appcast_from_input "$GENERATE_APPCAST_INPUT" || true)"
  if [[ -n "$from_input" ]]; then
    echo "$from_input"
    return 0
  fi

  if command -v generate_appcast >/dev/null 2>&1; then
    command -v generate_appcast
    return 0
  fi

  local candidates=(
    "/Applications/Sparkle.app/Contents/bin/generate_appcast"
    "$HOME/Applications/Sparkle.app/Contents/bin/generate_appcast"
  )

  local opt_cask=""
  local usr_cask=""
  opt_cask="$(latest_cask_sparkle_app "/opt/homebrew/Caskroom/sparkle" || true)"
  usr_cask="$(latest_cask_sparkle_app "/usr/local/Caskroom/sparkle" || true)"
  if [[ -n "$opt_cask" ]]; then
    candidates+=("$opt_cask/Contents/bin/generate_appcast")
  fi
  if [[ -n "$usr_cask" ]]; then
    candidates+=("$usr_cask/Contents/bin/generate_appcast")
  fi

  local opt_bin=""
  local usr_bin=""
  opt_bin="$(find /opt/homebrew/Caskroom/sparkle -maxdepth 4 -type f -name generate_appcast 2>/dev/null | sort -V | tail -n1)"
  usr_bin="$(find /usr/local/Caskroom/sparkle -maxdepth 4 -type f -name generate_appcast 2>/dev/null | sort -V | tail -n1)"
  if [[ -n "$opt_bin" ]]; then
    candidates+=("$opt_bin")
  fi
  if [[ -n "$usr_bin" ]]; then
    candidates+=("$usr_bin")
  fi

  for path in "${candidates[@]}"; do
    if [[ -x "$path" ]]; then
      echo "$path"
      return 0
    fi
  done
  return 1
}

maybe_install_sparkle() {
  if [[ "$AUTO_INSTALL" != "true" ]]; then
    return 1
  fi

  if ! command -v brew >/dev/null 2>&1; then
    return 1
  fi

  echo "Sparkle not found locally. Installing via Homebrew cask..." >&2
  brew install --cask sparkle
}

maybe_clear_quarantine() {
  local target="${1:-}"
  if [[ "$CLEAR_QUARANTINE" != "true" || -z "$target" ]]; then
    return 0
  fi
  if [[ ! -e "$target" ]]; then
    return 0
  fi
  if ! command -v xattr >/dev/null 2>&1; then
    return 0
  fi

  xattr -dr com.apple.quarantine "$target" >/dev/null 2>&1 || true
}

FRAMEWORK_PATH="$(detect_framework_path || true)"
GENERATE_APPCAST_PATH="$(detect_generate_appcast_path || true)"

if [[ -z "$FRAMEWORK_PATH" || -z "$GENERATE_APPCAST_PATH" ]]; then
  maybe_install_sparkle || true
  FRAMEWORK_PATH="$(detect_framework_path || true)"
  GENERATE_APPCAST_PATH="$(detect_generate_appcast_path || true)"
fi

if [[ -z "$FRAMEWORK_PATH" ]]; then
  echo "Failed to resolve Sparkle.framework path. Install Sparkle and/or pass --framework-path." >&2
  exit 1
fi
if [[ -z "$GENERATE_APPCAST_PATH" ]]; then
  echo "Failed to resolve generate_appcast path. Install Sparkle and/or pass --generate-appcast." >&2
  exit 1
fi

maybe_clear_quarantine "$FRAMEWORK_PATH"
maybe_clear_quarantine "$GENERATE_APPCAST_PATH"
maybe_clear_quarantine "$(dirname "$GENERATE_APPCAST_PATH")"

echo "framework=$FRAMEWORK_PATH"
echo "generate_appcast=$GENERATE_APPCAST_PATH"
