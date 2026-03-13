#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

COMPOSE_ARGS=(-f docker-compose.pi.yml)

# Build on boot is optional; default is faster startup for event day.
if [[ "${TEALTEAM_BUILD_ON_BOOT:-false}" == "true" ]]; then
	docker compose "${COMPOSE_ARGS[@]}" up -d --build
else
	docker compose "${COMPOSE_ARGS[@]}" up -d
fi

# Show LAN URL on terminal + optional LCD screen.
"${ROOT_DIR}/scripts/pi_show_ip.sh"
