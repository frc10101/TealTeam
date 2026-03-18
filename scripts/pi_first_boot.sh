#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOT_DIR="/boot/firmware"
if [[ ! -d "${BOOT_DIR}" ]]; then
  BOOT_DIR="/boot"
fi
for arg in "$@"; do
  case "$arg" in
    -h|--help)
      cat <<'EOF'
Usage: sudo ./scripts/pi_first_boot.sh

Bootstraps a fresh Raspberry Pi for TealTeam by installing prerequisites,
Docker, Python runtime dependencies, and preparing local app config.
EOF
      exit 0
      ;;
    *)
      echo "Unknown option: $arg"
      echo "Run with --help for usage."
      exit 1
      ;;
  esac
done

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run as root: sudo ./scripts/pi_first_boot.sh"
  exit 1
fi

if ! command -v apt-get >/dev/null 2>&1; then
  echo "This script currently supports apt-based systems (Raspberry Pi OS/Debian)."
  exit 1
fi

echo "[1/8] Installing base packages..."
apt-get update
apt-get install -y ca-certificates curl git python3 python3-pip avahi-daemon

if ! command -v docker >/dev/null 2>&1; then
  echo "[2/8] Installing Docker..."
  curl -fsSL https://get.docker.com | sh
else
  echo "[2/8] Docker already installed."
fi

echo "[3/8] Enabling Docker service..."
systemctl enable --now docker
systemctl enable --now avahi-daemon || true

if docker compose version >/dev/null 2>&1; then
  echo "[4/8] Docker Compose plugin is available."
else
  echo "[4/8] Docker Compose plugin not found after Docker install."
  echo "Please verify Docker installation before continuing."
  exit 1
fi

TARGET_USER="${SUDO_USER:-}"
if [[ -n "${TARGET_USER}" && "${TARGET_USER}" != "root" ]]; then
  echo "[5/8] Adding ${TARGET_USER} to docker group..."
  usermod -aG docker "${TARGET_USER}" || true
else
  echo "[5/8] Skipping docker group update (no non-root sudo user detected)."
fi

if [[ -f "${PROJECT_DIR}/.env" ]]; then
  echo "[6/8] .env already exists (leaving as-is)."
elif [[ -f "${PROJECT_DIR}/.env.example" ]]; then
  echo "[6/8] Creating .env from .env.example..."
  cp "${PROJECT_DIR}/.env.example" "${PROJECT_DIR}/.env"
else
  echo "[6/8] Could not find .env.example in ${PROJECT_DIR}."
  exit 1
fi

echo "[7/8] Installing Python libraries used by Pi scripts..."
apt-get install -y i2c-tools
python3 -m pip install --break-system-packages RPLCD smbus2 || python3 -m pip install RPLCD smbus2
if command -v raspi-config >/dev/null 2>&1; then
  raspi-config nonint do_i2c 0 || true
fi

echo "[8/8] Writing boot-partition headless env template..."
if [[ ! -f "${BOOT_DIR}/tealteam.env" ]]; then
cat > "${BOOT_DIR}/tealteam.env" <<EOF
# Optional headless override file loaded automatically at service start.
# Copy this file and fill in real values before events if you do not want to SSH into the Pi.
FIRST_API_USERNAME=
FIRST_API_KEY=
FIRST_SEASON=2026
FIRST_SYNC_ON_BOOT=true
FIRST_EVENT_CODE=
FIRST_TEAM_NUMBER=10101
FIRST_COUNTRY=
TBA_AUTH_KEY=
WEB_HOST_PORT=80
DB_DATA_PATH=${PROJECT_DIR}/.data/postgres
EOF
else
  echo "${BOOT_DIR}/tealteam.env already exists (leaving as-is)."
fi

echo
echo "Bootstrap complete. Next steps:"
echo "1) Fill ${BOOT_DIR}/tealteam.env (headless) or edit ${PROJECT_DIR}/.env"
echo "2) Install autostart service: sudo ./scripts/install_pi_autostart.sh"
echo "3) Start service now: sudo systemctl start tealteam.service"
echo "4) Follow logs: journalctl -u tealteam.service -f"
echo "5) App URL (mDNS): http://$(hostname).local/"
if [[ -n "${TARGET_USER}" && "${TARGET_USER}" != "root" ]]; then
  echo "6) Re-login shell (or reboot) so docker group membership applies for ${TARGET_USER}"
fi
