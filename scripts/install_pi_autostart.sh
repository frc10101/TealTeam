#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_NAME="tealteam"
SERVICE_PATH="/etc/systemd/system/${SERVICE_NAME}.service"
URL_REFRESH_SERVICE_PATH="/etc/systemd/system/tealteam-url-refresh.service"
URL_REFRESH_TIMER_PATH="/etc/systemd/system/tealteam-url-refresh.timer"
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
apt-get install -y python3 python3-pip i2c-tools avahi-daemon
python3 -m pip install --break-system-packages RPLCD smbus2 || python3 -m pip install RPLCD smbus2
systemctl enable --now avahi-daemon || true

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
Environment=LCD_ENABLED=false
ExecStartPre=${PROJECT_DIR}/scripts/pi_prepare_env.sh
ExecStart=${PROJECT_DIR}/scripts/pi_boot.sh
ExecStartPost=${PROJECT_DIR}/scripts/pi_finalize_first_boot.sh
ExecStop=${DOCKER_BIN} compose -f ${PROJECT_DIR}/docker-compose.pi.yml down
TimeoutStartSec=0

[Install]
WantedBy=multi-user.target
EOF

cat > "${URL_REFRESH_SERVICE_PATH}" <<EOF
[Unit]
Description=Refresh TealTeam URL files
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
WorkingDirectory=${PROJECT_DIR}
Environment=WEB_HOST_PORT=80
Environment=LCD_ENABLED=false
ExecStart=${PROJECT_DIR}/scripts/pi_show_ip.sh
EOF

cat > "${URL_REFRESH_TIMER_PATH}" <<'EOF'
[Unit]
Description=Refresh TealTeam URL files every 30 seconds

[Timer]
OnBootSec=20s
OnUnitActiveSec=30s
AccuracySec=5s
Unit=tealteam-url-refresh.service

[Install]
WantedBy=timers.target
EOF

chmod 644 "${SERVICE_PATH}"
chmod 644 "${URL_REFRESH_SERVICE_PATH}" "${URL_REFRESH_TIMER_PATH}"
chmod +x "${PROJECT_DIR}/scripts/pi_boot.sh" "${PROJECT_DIR}/scripts/pi_prepare_env.sh" "${PROJECT_DIR}/scripts/pi_finalize_first_boot.sh" "${PROJECT_DIR}/scripts/pi_show_ip.sh" "${PROJECT_DIR}/scripts/pi_show_ip_lcd.py"

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}.service"
systemctl enable --now tealteam-url-refresh.timer

# Best-effort firewall setup when UFW is present.
if command -v ufw >/dev/null 2>&1; then
  PORT="${WEB_HOST_PORT:-80}"
  ufw allow "${PORT}/tcp" || true
fi

echo "Installed ${SERVICE_NAME}.service"
echo "Installed tealteam-url-refresh.timer (30s URL refresh)"
echo "Start now: sudo systemctl start ${SERVICE_NAME}.service"
echo "Status:    systemctl status ${SERVICE_NAME}.service"
echo "Logs:      journalctl -u ${SERVICE_NAME}.service -f"
