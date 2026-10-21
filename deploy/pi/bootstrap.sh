#!/usr/bin/env bash
# A fresh Raspberry Pi OS into a running TealTeam server (SK1).
#
#   sudo deploy/pi/bootstrap.sh deploy/profiles/frc-standard.env path/to/tt-web
#   deploy/pi/bootstrap.sh <profile> <tt-web> --dry-run       say, change nothing
#   sudo deploy/pi/bootstrap.sh <profile> <tt-web> --non-interactive
#                                                  never ask: blank secrets stay
#                                                  blank, fill them in the env file
#
# In order: check the Pi, install the few packages the scripts need, check the
# SSD is mounted (it never partitions a disk), set up the event network
# (network/setup.sh), install the service (service/install.sh), write the env
# file from the example and the profile (asking for the keys, mode 600), wait
# for the server, take a backup and restore-check it, and print the status.
#
# Safe to run again: a value already in the env file is kept, not asked for.
# Build tt-web with deploy/build-release.sh (see docs/SETUP_GUIDE.md).
#
# Needs internet only for apt, so run it at the shop. The network step moves
# eth0 to the event LAN (10.101.0.1, no internet): do not run it over SSH on
# eth0.
#
# UNTESTED on a Pi: checked with bash -n and --dry-run on a laptop, never run
# as root on Raspberry Pi OS.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR=/opt/tealteam
DATA_ROOT=/srv/tealteam
SERVICE_USER=tealteam
ENV_FILE="$APP_DIR/.env"

usage() {
    echo "usage: $0 <profile-file> <path/to/tt-web> [--dry-run] [--non-interactive]" >&2
    exit 2
}

profile="" binary="" DRY_RUN=0 ASK=1
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        --non-interactive) ASK=0 ;;
        -*) usage ;;
        *)
            if [[ -z "$profile" ]]; then profile="$arg"
            elif [[ -z "$binary" ]]; then binary="$arg"
            else usage; fi
            ;;
    esac
done
[[ -n "$profile" && -n "$binary" ]] || usage
[[ -f "$profile" ]] || { echo "no profile file $profile" >&2; exit 2; }
[[ -f "$binary" ]] || { echo "no binary $binary: build one with deploy/build-release.sh" >&2; exit 2; }
[[ -t 0 || -r /dev/tty ]] || ASK=0

dry=()
[[ $DRY_RUN -eq 1 ]] && dry=(--dry-run)

say() { printf '==> %s\n' "$*"; }
run() {
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '    would run: %s\n' "$*"
    else
        "$@"
    fi
}
# A problem that stops a real run. On a dry run it is only said, so the rest
# of the steps can still be shown on a machine that is not a Pi.
refuse() {
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '    would refuse: %s\n' "$*"
    else
        echo "refusing: $*" >&2
        exit 1
    fi
}

# ── The profile ─────────────────────────────────────────────────────────────
# Read, not sourced: a profile is data. Keys and values land in these arrays
# in file order.
keys=() vals=()
while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    [[ "$line" =~ ^[[:space:]]*([A-Z][A-Z0-9_]*)=(.*)$ ]] || { echo "profile line not KEY=VALUE: $line" >&2; exit 2; }
    val="${BASH_REMATCH[2]}"
    val="${val#\"}"; val="${val%\"}"; val="${val#\'}"; val="${val%\'}"
    keys+=("${BASH_REMATCH[1]}")
    vals+=("$val")
done < "$profile"

# The value now in the env file for $1, empty if none.
current() {
    [[ -r "$ENV_FILE" ]] || return 0
    sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1
}
is_secret() { [[ "$1" == *KEY* || "$1" == *USERNAME* || "$1" == *AUTH* || "$1" == *PASSWORD* ]]; }

# ── The Pi ──────────────────────────────────────────────────────────────────
say "checking the Pi"
[[ "$(uname -m)" == aarch64 ]] || refuse "this is $(uname -m), not a 64-bit Raspberry Pi OS (aarch64)"
# shellcheck disable=SC1091
os_id="$(. /etc/os-release 2>/dev/null && echo "${ID:-}")"
case "$os_id" in
    debian | raspbian) ;;
    *) refuse "this is ${os_id:-an unknown OS}, not Raspberry Pi OS (Debian)" ;;
esac
command -v apt-get >/dev/null || refuse "no apt-get: this script is for Raspberry Pi OS"
[[ $DRY_RUN -eq 1 || $EUID -eq 0 ]] || refuse "run as root: sudo $0 $*"
# An ELF file's machine number is the 16-bit value at offset 18: 183 is aarch64.
if [[ "$(od -An -t u1 -j18 -N1 "$binary" 2>/dev/null | tr -d ' ')" != 183 ]]; then
    refuse "$binary is not an aarch64 build: build it on the Pi, or cross-compile (docs/SETUP_GUIDE.md)"
fi
# eth0 is about to become the event LAN.
if [[ -n "${SSH_CONNECTION:-}" ]]; then
    peer="${SSH_CONNECTION%% *}"
    if [[ "$(ip route get "$peer" 2>/dev/null | head -n 1)" == *" dev eth0 "* ]]; then
        refuse "you are logged in over eth0, which the network step moves to 10.101.0.1 and cuts off. Use Wi-Fi or the console"
    fi
fi

# ── Packages ────────────────────────────────────────────────────────────────
# The network and service scripts install the rest. curl is for the health
# check here and in status.sh; util-linux has findmnt and runuser.
missing=()
for pkg in curl util-linux; do
    dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "ok installed" || missing+=("$pkg")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    say "installing packages: ${missing[*]} (needs internet)"
    run apt-get install -y "${missing[@]}"
fi

# ── The SSD ─────────────────────────────────────────────────────────────────
say "checking the SSD at $DATA_ROOT"
if ! findmnt --mountpoint "$DATA_ROOT" >/dev/null 2>&1; then
    refuse "$DATA_ROOT is not a mounted drive. Set up the SSD first: docs/PI_STORAGE.md steps 1-4 (this script never partitions a disk)"
else
    source_dev="$(findmnt -n -o SOURCE --mountpoint "$DATA_ROOT")"
    case "$source_dev" in
        /dev/mmcblk*) refuse "$DATA_ROOT is on the SD card ($source_dev), not the SSD: docs/PI_STORAGE.md" ;;
        *) echo "    $DATA_ROOT is on $source_dev" ;;
    esac
fi

# ── Network, then the service ───────────────────────────────────────────────
say "event network (network/setup.sh)"
"$HERE/network/setup.sh" "${dry[@]}"

say "service (service/install.sh)"
"$HERE/service/install.sh" "$binary" "${dry[@]}"

# ── The env file ────────────────────────────────────────────────────────────
# The example's lines, then the profile's keys. Per key: what the file has
# now if it is not blank, else the profile's value, else (for a blank profile
# value) what you type.
say "env file $ENV_FILE"
base="$HERE/service/tealteam.env.example"
[[ -r "$ENV_FILE" ]] && base="$ENV_FILE"
new="$(mktemp)"
trap 'rm -f "$new"' EXIT
cp "$base" "$new"
for i in "${!keys[@]}"; do
    key="${keys[$i]}" val="${vals[$i]}"
    [[ "$key" == PREP_* ]] && continue
    have="$(current "$key")"
    if [[ -n "$have" ]]; then
        val="$have"
    elif [[ -z "$val" && $ASK -eq 1 && $DRY_RUN -eq 0 ]]; then
        if is_secret "$key"; then
            read -rsp "    $key (typed with no echo; Enter to leave blank): " val < /dev/tty
            echo
        else
            read -rp "    $key (Enter to leave blank): " val < /dev/tty
        fi
    elif [[ -z "$val" ]]; then
        echo "    $key is blank: set it in $ENV_FILE"
    fi
    # Replace the line in place, or add it. awk, not sed: a key may hold / & \.
    if grep -q "^$key=" "$new"; then
        K="$key" V="$val" awk 'BEGIN { FS = "=" } $1 == ENVIRON["K"] { print ENVIRON["K"] "=" ENVIRON["V"]; next } { print }' "$new" > "$new.2"
        mv "$new.2" "$new"
    else
        printf '%s=%s\n' "$key" "$val" >> "$new"
    fi
done
if [[ -r "$ENV_FILE" ]] && cmp -s "$new" "$ENV_FILE"; then
    echo "    unchanged"
    env_changed=0
else
    run install -D -m 0600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$new" "$ENV_FILE"
    env_changed=1
fi

if [[ $DRY_RUN -eq 1 ]]; then
    say "dry run: stopping before the restart, the health wait, and the backup"
    exit 0
fi

[[ $env_changed -eq 1 ]] && { say "restarting tealteam with the new settings"; systemctl restart tealteam; }

# ── Is it up? ───────────────────────────────────────────────────────────────
port="$(current PORT)"
port="${port:-8080}"
say "waiting for the server on port $port"
up=0
for _ in $(seq 1 60); do
    if curl -fsS --max-time 2 "http://localhost:$port/health" 2>/dev/null | grep -q '"storage":"ready"'; then
        up=1
        break
    fi
    sleep 1
done
if [[ $up -eq 0 ]]; then
    echo "the server did not report storage ready in 60 s. Look: journalctl -u tealteam -n 50" >&2
    exit 1
fi
curl -fsS "http://localhost:$port/health"
echo

# ── One backup, restored to prove it ────────────────────────────────────────
say "one backup and a restore check"
backup_dir="$(current BACKUP_DIR)"
backup_dir="${backup_dir:-$DATA_ROOT/backups}"
cd "$APP_DIR"
out="$(runuser -u "$SERVICE_USER" -- "$APP_DIR/tt-web" backup "$backup_dir")" || {
    echo "$out"
    echo "the backup failed" >&2
    exit 1
}
echo "$out"
file="$(printf '%s\n' "$out" | sed -n 's/^Backed up to \(.*\) ([0-9.]* MB)$/\1/p' | head -n 1)"
if [[ -n "$file" ]]; then
    runuser -u "$SERVICE_USER" -- "$APP_DIR/tt-web" check-backup "$file"
else
    echo "    could not read the backup's name; run: tt-web check-backup"
fi

# ── Status ──────────────────────────────────────────────────────────────────
say "status"
"$HERE/network/status.sh" || echo "    (a FAIL above names its fix; no switch cabled yet is expected at the shop)"
echo
say "done. Scouts open http://tealteam.local/ (or http://10.101.0.1/) from a laptop on the switch."
echo "    Next: deploy/event/prep-event.sh $profile, from a laptop."
