#!/usr/bin/env bash
# Install the server as a service that starts at boot (P10).
#
#   sudo ./install.sh path/to/tt-web             install or upgrade
#   sudo ./install.sh path/to/tt-web --dry-run   say what it would change
#
# Idempotent: rerun it with each new build. Do the SSD (docs/PI_STORAGE.md)
# first; the service will not start until /srv/tealteam is mounted.
#
# UNTESTED on a Pi: checked with ShellCheck and systemd-analyze, never run.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR=/opt/tealteam
DATA_ROOT=/srv/tealteam
SERVICE_USER=tealteam
DRY_RUN=0

binary="${1:-}"
case "${2:-}" in
    "") ;;
    --dry-run) DRY_RUN=1 ;;
    *) binary="" ;;
esac
if [[ -z "$binary" || ! -f "$binary" ]]; then
    echo "usage: $0 path/to/tt-web [--dry-run]" >&2
    exit 2
fi
if [[ $DRY_RUN -eq 0 && $EUID -ne 0 ]]; then
    echo "run as root: sudo $0 $*" >&2
    exit 1
fi

say() { printf '==> %s\n' "$*"; }
run() {
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '    would run: %s\n' "$*"
    else
        "$@"
    fi
}

# Install $1 to $2 with mode $3 when the content differs; true only when it
# changed something. A failed install stops the script (`set -e` does not
# apply inside a function called from `&&`).
place() {
    local src="$1" dest="$2" mode="$3"
    if [[ -f "$dest" ]] && cmp -s "$src" "$dest"; then
        return 1
    fi
    say "installing $dest"
    run install -D -m "$mode" "$src" "$dest" || { echo "could not install $dest" >&2; exit 1; }
    return 0
}

if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    say "creating the $SERVICE_USER system user"
    run useradd --system --no-create-home --home-dir "$APP_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
fi

restart=0
place "$binary" "$APP_DIR/tt-web" 0755 && restart=1

# Settings: written once, never overwritten, since they hold the API keys.
if [[ ! -f "$APP_DIR/.env" ]]; then
    say "writing $APP_DIR/.env from the example: add the API keys there"
    run install -m 0600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$HERE/tealteam.env.example" "$APP_DIR/.env"
    restart=1
fi

# The data folders, inside the SSD's mount. Only when it is mounted: made on
# the SD card, they would let the server start an empty database there.
if findmnt --mountpoint "$DATA_ROOT" >/dev/null 2>&1; then
    for dir in "$DATA_ROOT/data" "$DATA_ROOT/backups"; do
        if [[ ! -d "$dir" || "$(stat -c %U "$dir")" != "$SERVICE_USER" ]]; then
            say "making $dir, owned by $SERVICE_USER"
            run install -d -o "$SERVICE_USER" -g "$SERVICE_USER" -m 0750 "$dir"
        fi
    done
else
    say "$DATA_ROOT is not mounted: set up the SSD (docs/PI_STORAGE.md), then rerun this"
fi

if place "$HERE/tealteam.service" /etc/systemd/system/tealteam.service 0644; then
    restart=1
    run systemctl daemon-reload
fi
if ! systemctl is-enabled --quiet tealteam 2>/dev/null; then
    say "enabling tealteam at boot"
    run systemctl enable tealteam
fi
if [[ $restart -eq 1 ]] || ! systemctl is-active --quiet tealteam 2>/dev/null; then
    say "(re)starting tealteam"
    run systemctl restart tealteam
fi

say "done. Logs: journalctl -u tealteam -f    Status: systemctl status tealteam"
