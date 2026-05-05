#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/dioxus-cli.sh"

usage() {
  cat <<'EOF'
Run Loopbox in desktop dev mode from the public repo.

Usage:
  scripts/dev-serve.sh [-- <extra dx args>]

Examples:
  scripts/dev-serve.sh
  DIOXUS_CLI_BIN="$HOME/.cargo/bin/dx" scripts/dev-serve.sh
  scripts/dev-serve.sh -- --verbose
EOF
}

DX_EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      DX_EXTRA_ARGS=("$@")
      break
      ;;
    *)
      echo "Unexpected argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

DIOXUS_CLI="$(resolve_dioxus_cli)"

cd "$PROJECT_DIR"
exec "$DIOXUS_CLI" serve --platform desktop "${DX_EXTRA_ARGS[@]}"
