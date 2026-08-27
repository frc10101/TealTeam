#!/usr/bin/env bash
# Run everything CI runs, locally. Same order, same failures.
#
#   ./check.sh          full run
#   ./check.sh --fast   skip the wasm build (still runs fmt, clippy, tests)
set -euo pipefail

fast=false
[[ "${1:-}" == "--fast" ]] && fast=true

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$1"; }

step "Formatting"
cargo fmt --all --check

step "Clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "Tests"
cargo test --workspace

# Build the actual binary too. `cargo test` builds a *test* binary; without this
# a stale target/debug/tt-web can pass every check and still be the old code.
step "Binary"
cargo build --workspace

if [[ "$fast" == false ]]; then
  step "wasm32 boundary (tt-core, tt-templates)"
  if ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "installing wasm32-unknown-unknown..."
    rustup target add wasm32-unknown-unknown
  fi
  cargo build -p tt-core -p tt-templates --target wasm32-unknown-unknown
fi

printf '\n\033[1;32mAll checks passed.\033[0m\n'
