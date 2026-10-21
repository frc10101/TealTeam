#!/usr/bin/env bash
# Build the service worker's wasm module (C5) into the server's static files.
#
#   deploy/build-client.sh            release build (what a Pi should run)
#   deploy/build-client.sh --debug    quicker, larger
#
# Then build tt-web as usual: it compiles crates/tt-web/static/ into the
# binary, client/ included, and the service worker imports it from there.
# A tt-web built without running this first works as before (C1): with no
# server, every page is the offline shell. deploy/build-release.sh does both
# steps for a Pi (C14); check.sh and CI run this too.
#
# wasm-bindgen's CLI must be exactly the version in Cargo.lock, pinned below.
# Without that version on the PATH, this installs it once with cargo into
# ~/.cargo/wasm-bindgen-<version>/, beside rather than over any other
# wasm-bindgen you have. WASM_BINDGEN=/path/to/wasm-bindgen picks a binary.

set -euo pipefail

# Bump with the wasm-bindgen crate in Cargo.lock; this checks they agree.
PIN=0.2.127

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/crates/tt-web/static/client"
TOOLS="${CARGO_HOME:-$HOME/.cargo}/wasm-bindgen-$PIN"

profile=release
flag=(--release)
if [[ "${1:-}" == "--debug" ]]; then
    profile=debug
    flag=()
fi

locked="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' "$ROOT/Cargo.lock")"
if [[ "$locked" != "$PIN" ]]; then
    echo "Cargo.lock has wasm-bindgen $locked, but $0 pins $PIN; change PIN to match" >&2
    exit 1
fi

version() { "$1" --version 2>/dev/null | awk '{ print $2 }'; }

if [[ -n "${WASM_BINDGEN:-}" ]]; then
    BINDGEN="$WASM_BINDGEN"
elif command -v wasm-bindgen >/dev/null && [[ "$(version wasm-bindgen)" == "$PIN" ]]; then
    BINDGEN=wasm-bindgen
else
    BINDGEN="$TOOLS/bin/wasm-bindgen"
    if [[ "$(version "$BINDGEN")" != "$PIN" ]]; then
        echo "installing wasm-bindgen-cli $PIN into $TOOLS (once)..."
        cargo install wasm-bindgen-cli --version "$PIN" --locked --root "$TOOLS"
    fi
fi
have="$(version "$BINDGEN")"
if [[ "$have" != "$PIN" ]]; then
    echo "$BINDGEN is wasm-bindgen ${have:-(not runnable)}, but Cargo.lock needs $PIN" >&2
    exit 1
fi

if command -v rustup >/dev/null && ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "installing wasm32-unknown-unknown..."
    rustup target add wasm32-unknown-unknown
fi

cd "$ROOT"
cargo build -p tt-client --target wasm32-unknown-unknown --locked "${flag[@]}"

target="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
rm -rf "$OUT"
# no-modules: a classic service worker loads it with importScripts.
"$BINDGEN" --target no-modules --no-typescript --out-dir "$OUT" \
    "$target/wasm32-unknown-unknown/$profile/tt_client.wasm"
ls -l "$OUT"
