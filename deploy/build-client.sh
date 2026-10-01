#!/usr/bin/env bash
# Build the service worker's wasm module (C5) into the server's static files.
#
#   deploy/build-client.sh            release build (what a Pi should run)
#   deploy/build-client.sh --debug    quicker, larger
#
# Then build tt-web as usual: it compiles crates/tt-web/static/ into the
# binary, client/ included, and the service worker imports it from there.
# A tt-web built without running this first works as before (C1): with no
# server, every page is the offline shell.
#
# Needs the wasm32-unknown-unknown target and wasm-bindgen's CLI at exactly
# the version in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <that version> --locked
# WASM_BINDGEN=/path/to/wasm-bindgen picks a binary not on the PATH.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/crates/tt-web/static/client"
BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"

profile=release
flag=(--release)
if [[ "${1:-}" == "--debug" ]]; then
    profile=debug
    flag=()
fi

want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' "$ROOT/Cargo.lock")"
if ! have="$("$BINDGEN" --version 2>/dev/null | awk '{ print $2 }')"; then
    echo "no wasm-bindgen; install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
    exit 1
fi
if [[ "$have" != "$want" ]]; then
    echo "wasm-bindgen is $have, but Cargo.lock has $want; install that one:" >&2
    echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
    exit 1
fi

cd "$ROOT"
cargo build -p tt-client --target wasm32-unknown-unknown "${flag[@]}"

target="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
rm -rf "$OUT"
# no-modules: a classic service worker loads it with importScripts.
"$BINDGEN" --target no-modules --no-typescript --out-dir "$OUT" \
    "$target/wasm32-unknown-unknown/$profile/tt_client.wasm"
ls -l "$OUT"
