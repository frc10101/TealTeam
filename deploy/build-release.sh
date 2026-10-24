#!/usr/bin/env bash
# Build the tt-web a Pi runs: the service worker's wasm module first, then
# the release binary that embeds it, so devices can make pages offline (C14).
# And tt-ops beside it, the operator's terminal (docs/event/OPS_TERMINAL.md).
#
#   deploy/build-release.sh
#   sudo deploy/pi/service/install.sh target/release/tt-web
#
# Run it as yourself, not root: the first run installs wasm-bindgen's CLI
# with cargo (deploy/build-client.sh says where).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

"$ROOT/deploy/build-client.sh"
cargo build --release --locked -p tt-web -p tt-ops

target="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
echo
echo "built $target/release/tt-web, with the offline client, and $target/release/tt-ops. Install them with:"
echo "  sudo $ROOT/deploy/pi/service/install.sh $target/release/tt-web"
