#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IP_ADDR="$(hostname -I | awk '{print $1}')"
WEB_PORT="${WEB_HOST_PORT:-80}"
HOSTNAME_URL="http://$(hostname).local/"

if [[ -z "${IP_ADDR}" ]]; then
  echo "Unable to determine Pi IP address"
  exit 1
fi

if [[ "${WEB_PORT}" == "80" ]]; then
  URL="http://${IP_ADDR}/"
else
  URL="http://${IP_ADDR}:${WEB_PORT}/"
fi

echo "TealTeam URL: ${URL}"
printf '%s\n' "${URL}" > /tmp/tealteam-url.txt
echo "TealTeam mDNS URL: ${HOSTNAME_URL}"
printf '%s\n' "${HOSTNAME_URL}" > /tmp/tealteam-mdns-url.txt

# Optional LCD output (I2C 16x2 backpack via RPLCD). If unavailable, keep boot flow healthy.
if [[ "${LCD_ENABLED:-true}" == "true" ]] && command -v python3 >/dev/null 2>&1; then
  python3 "${ROOT_DIR}/scripts/pi_show_ip_lcd.py" \
    --line1 "TealTeam Ready" \
    --line2 "${IP_ADDR}:${WEB_PORT}" || true
fi
