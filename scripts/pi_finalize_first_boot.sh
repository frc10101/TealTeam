#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${PROJECT_DIR}/.state"
MARKER_FILE="${STATE_DIR}/first_boot_mode_switched"
TARGET_ENV="${PROJECT_DIR}/.env"
BOOT_ENV_PRIMARY="/boot/firmware/tealteam.env"
BOOT_ENV_FALLBACK="/boot/tealteam.env"

set_env_key() {
  local file_path="$1"
  local key="$2"
  local value="$3"

  [[ -f "${file_path}" ]] || return 0

  if grep -q "^${key}=" "${file_path}"; then
    sed -i.bak "s|^${key}=.*$|${key}=${value}|" "${file_path}"
    rm -f "${file_path}.bak"
  else
    printf '\n%s=%s\n' "${key}" "${value}" >> "${file_path}"
  fi
}

if [[ -f "${MARKER_FILE}" ]]; then
  exit 0
fi

mkdir -p "${STATE_DIR}"

# After first boot, switch to event mode so startup does not depend on internet.
set_env_key "${TARGET_ENV}" "FIRST_SYNC_ON_BOOT" "false"
set_env_key "${TARGET_ENV}" "TEALTEAM_BOOT_MODE" "event"

if [[ -f "${BOOT_ENV_PRIMARY}" ]]; then
  set_env_key "${BOOT_ENV_PRIMARY}" "FIRST_SYNC_ON_BOOT" "false"
  set_env_key "${BOOT_ENV_PRIMARY}" "TEALTEAM_BOOT_MODE" "event"
elif [[ -f "${BOOT_ENV_FALLBACK}" ]]; then
  set_env_key "${BOOT_ENV_FALLBACK}" "FIRST_SYNC_ON_BOOT" "false"
  set_env_key "${BOOT_ENV_FALLBACK}" "TEALTEAM_BOOT_MODE" "event"
fi

echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) first boot mode switched to event" > "${MARKER_FILE}"
