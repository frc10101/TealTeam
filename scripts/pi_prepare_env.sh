#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOT_ENV_PRIMARY="/boot/firmware/tealteam.env"
BOOT_ENV_FALLBACK="/boot/tealteam.env"
TARGET_ENV="${PROJECT_DIR}/.env"

# Create local .env from example on first boot if needed.
if [[ ! -f "${TARGET_ENV}" ]]; then
  if [[ -f "${PROJECT_DIR}/.env.example" ]]; then
    cp "${PROJECT_DIR}/.env.example" "${TARGET_ENV}"
  fi
fi

# Headless override path: drop tealteam.env onto boot partition and it will be applied.
if [[ -f "${BOOT_ENV_PRIMARY}" ]]; then
  install -m 600 "${BOOT_ENV_PRIMARY}" "${TARGET_ENV}"
elif [[ -f "${BOOT_ENV_FALLBACK}" ]]; then
  install -m 600 "${BOOT_ENV_FALLBACK}" "${TARGET_ENV}"
fi
