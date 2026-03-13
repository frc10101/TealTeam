#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_NAME="tealteam"
SERVICE_PATH="/etc/systemd/system/${SERVICE_NAME}.service"
DOCKER_BIN="$(command -v docker || true)"

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run as root: sudo ./scripts/install_pi_autostart.sh"
  exit 1
fi

if [[ -z "${DOCKER_BIN}" ]]; then
  echo "Docker is required. Install Docker first."
  exit 1
fi

if ! command -v apt-get >/dev/null 2>&1; then
  echo "This script currently supports apt-based systems (Raspberry Pi OS/Debian)."
  exit 1
fi

if [[ ! -f "${PROJECT_DIR}/docker-compose.pi.yml" ]]; then
  echo "Missing ${PROJECT_DIR}/docker-compose.pi.yml"
  exit 1
fi

echo "Installing Python runtime dependencies for Pi scripts..."
apt-get update
apt-get install -y python3 python3-pip i2c-tools
python3 -m pip install --break-system-packages RPLCD smbus2 || python3 -m pip install RPLCD smbus2

cat > "${SERVICE_PATH}" <<EOF
[Unit]
Description=TealTeam boot service (Docker Compose)
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=${PROJECT_DIR}
Environment=WEB_HOST_PORT=80
Environment=LCD_ENABLED=true
ExecStart=${PROJECT_DIR}/scripts/pi_boot.sh
ExecStop=${DOCKER_BIN} compose -f ${PROJECT_DIR}/docker-compose.pi.yml down
TimeoutStartSec=0

[Install]
WantedBy=multi-user.target
EOF

chmod 644 "${SERVICE_PATH}"
chmod +x "${PROJECT_DIR}/scripts/pi_boot.sh" "${PROJECT_DIR}/scripts/pi_show_ip.sh" "${PROJECT_DIR}/scripts/pi_show_ip_lcd.py"

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}.service"

# Best-effort firewall setup when UFW is present.
if command -v ufw >/dev/null 2>&1; then
  PORT="${WEB_HOST_PORT:-80}"
  ufw allow "${PORT}/tcp" || true
fi

echo "Installed ${SERVICE_NAME}.service"
echo "Start now: sudo systemctl start ${SERVICE_NAME}.service"
echo "Status:    systemctl status ${SERVICE_NAME}.service"
echo "Logs:      journalctl -u ${SERVICE_NAME}.service -f"
