#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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

echo "[1/7] Installing base packages..."
apt-get update
apt-get install -y ca-certificates curl git python3 python3-pip

if ! command -v docker >/dev/null 2>&1; then
  echo "[2/7] Installing Docker..."
  curl -fsSL https://get.docker.com | sh
else
  echo "[2/7] Docker already installed."
fi

echo "[3/7] Enabling Docker service..."
systemctl enable --now docker

if docker compose version >/dev/null 2>&1; then
  echo "[4/7] Docker Compose plugin is available."
else
  echo "[4/7] Docker Compose plugin not found after Docker install."
  echo "Please verify Docker installation before continuing."
  exit 1
fi

TARGET_USER="${SUDO_USER:-}"
if [[ -n "${TARGET_USER}" && "${TARGET_USER}" != "root" ]]; then
  echo "[5/7] Adding ${TARGET_USER} to docker group..."
  usermod -aG docker "${TARGET_USER}" || true
else
  echo "[5/7] Skipping docker group update (no non-root sudo user detected)."
fi

if [[ -f "${PROJECT_DIR}/.env" ]]; then
  echo "[6/7] .env already exists (leaving as-is)."
elif [[ -f "${PROJECT_DIR}/.env.example" ]]; then
  echo "[6/7] Creating .env from .env.example..."
  cp "${PROJECT_DIR}/.env.example" "${PROJECT_DIR}/.env"
else
  echo "[6/7] Could not find .env.example in ${PROJECT_DIR}."
  exit 1
fi

echo "[7/7] Installing Python libraries used by Pi scripts..."
apt-get install -y i2c-tools
python3 -m pip install --break-system-packages RPLCD smbus2 || python3 -m pip install RPLCD smbus2
if command -v raspi-config >/dev/null 2>&1; then
  raspi-config nonint do_i2c 0 || true
fi

echo
echo "Bootstrap complete. Next steps:"
echo "1) Edit ${PROJECT_DIR}/.env with your API keys and settings"
echo "2) Install autostart service: sudo ./scripts/install_pi_autostart.sh"
echo "3) Start service now: sudo systemctl start tealteam.service"
echo "4) Follow logs: journalctl -u tealteam.service -f"
if [[ -n "${TARGET_USER}" && "${TARGET_USER}" != "root" ]]; then
  echo "5) Re-login shell (or reboot) so docker group membership applies for ${TARGET_USER}"
fi
