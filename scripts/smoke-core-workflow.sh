#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "== fmt =="
cargo fmt -- --check

echo "== tests =="
cargo test

echo "== clippy =="
cargo clippy --all-targets --all-features -- -D warnings

echo "== build =="
cargo build

echo "Loopbox core smoke completed."
