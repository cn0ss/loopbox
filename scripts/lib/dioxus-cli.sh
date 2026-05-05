#!/usr/bin/env bash

resolve_dioxus_cli() {
  local candidates=()

  if [[ -n "${DIOXUS_CLI_BIN:-}" ]]; then
    candidates+=("$DIOXUS_CLI_BIN")
  fi
  if [[ -x "${HOME:-}/.cargo/bin/dx" ]]; then
    candidates+=("$HOME/.cargo/bin/dx")
  fi
  if command -v dx >/dev/null 2>&1; then
    candidates+=("$(command -v dx)")
  fi

  local candidate
  local resolved
  local version_output
  for candidate in "${candidates[@]}"; do
    if [[ "$candidate" == */* ]]; then
      resolved="$candidate"
    else
      resolved="$(command -v "$candidate" 2>/dev/null || true)"
    fi

    if [[ -z "$resolved" || ! -x "$resolved" ]]; then
      continue
    fi

    version_output="$("$resolved" --version 2>/dev/null || true)"
    case "$version_output" in
      dioxus\ *)
        printf '%s\n' "$resolved"
        return 0
        ;;
    esac
  done

  echo "Dioxus CLI not found." >&2
  echo "Install it with: cargo install dioxus-cli" >&2
  echo "If another dx binary is earlier in PATH, set DIOXUS_CLI_BIN=/path/to/dx." >&2
  return 1
}
