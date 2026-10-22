#!/usr/bin/env bash
# A fresh Raspberry Pi OS -> a running TealTeam server (F12).
#
#   sudo deploy/pi/bootstrap.sh PROFILE path/to/tt-web             do it
#   sudo deploy/pi/bootstrap.sh PROFILE path/to/tt-web --dry-run   say what it would do
#         add --non-interactive to skip the secret prompts
#
# PROFILE is a file from deploy/profiles/. tt-web is the release binary from
# deploy/build-release.sh (it must carry the offline client).
#
# In order, and each step is safe to run again:
#   1. checks: root, 64-bit Raspberry Pi OS / Debian, the binary;
#   2. curl, the only package the steps below need that setup.sh does not
#      install itself;
#   3. the SSD must be mounted at /srv/tealteam, or it stops and points at
#      docs/event/PI_STORAGE.md. It never partitions a disk;
#   4. deploy/pi/network/setup.sh (hostname, eth0 10.101.0.1, DHCP/DNS, port 80);
#   5. /opt/tealteam/.env: the example plus the profile's values, mode 600,
#      owned by the service user. Secrets the profile leaves empty are asked
#      for without echo, and an empty answer leaves them unset. A rerun keeps
#      what is already in the file and only changes the keys the profile names;
#   6. deploy/pi/service/install.sh (the systemd service);
#   7. waits for /health to say storage ready;
#   8. one backup to BACKUP_DIR and a restore check of it;
#   9. deploy/pi/network/status.sh, and the address to open.
#
# Not done here: flashing the OS, attaching and formatting the SSD
# (docs/event/PI_STORAGE.md), cabling. docs/event/SETUP_GUIDE.md has those.
#
# UNTESTED on a Pi: --dry-run was run on a development machine, and the env
# file merge was run against a scratch directory. Nothing else has run for
# real. It calls scripts that are themselves untested on a Pi.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR=/opt/tealteam
DATA_ROOT=/srv/tealteam
SERVICE_USER=tealteam

profile="" binary="" dry_run=0 interactive=1
for arg in "$@"; do
    case "$arg" in
        --dry-run) dry_run=1 ;;
        --non-interactive) interactive=0 ;;
        -h | --help) sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option $arg" >&2; exit 2 ;;
        *) if [[ -z "$profile" ]]; then profile="$arg"; elif [[ -z "$binary" ]]; then binary="$arg"; else exit 2; fi ;;
    esac
done
if [[ -z "$profile" || -z "$binary" ]]; then
    echo "usage: $0 PROFILE path/to/tt-web [--dry-run] [--non-interactive]" >&2
    exit 2
fi
[[ -f "$profile" ]] || { echo "no profile at $profile" >&2; exit 2; }
[[ -f "$binary" ]] || { echo "no binary at $binary: build it with deploy/build-release.sh" >&2; exit 2; }

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
run() {
    if [[ $dry_run -eq 1 ]]; then printf '    would run: %s\n' "$*"; else "$@"; fi
}
# Something that stops a real run is only noted in a dry run, so it shows
# everything that is wrong at once.
problem() {
    if [[ $dry_run -eq 1 ]]; then printf '    WOULD STOP: %s\n' "$*"; else printf 'bootstrap: %s\n' "$*" >&2; exit 1; fi
}
pget() {
    local v
    v="$(sed -n "s/^$1=//p" "$profile" | tail -n 1 | tr -d '\r')"
    printf '%s' "${v:-${2:-}}"
}

# ── 1. Checks ───────────────────────────────────────────────────────────────

say "checking this machine"
[[ $dry_run -eq 1 || $EUID -eq 0 ]] || problem "run as root: sudo $0 $*"
[[ "$(uname -s)" == Linux ]] || problem "this is for Raspberry Pi OS (Linux), not $(uname -s)"
arch="$(uname -m)"
[[ "$arch" == aarch64 ]] || problem "this is $arch; the Pi needs 64-bit Raspberry Pi OS (aarch64)"
if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    os_like="$(. /etc/os-release; printf '%s %s' "${ID:-}" "${ID_LIKE:-}")"
    case "$os_like" in *debian* | *raspbian*) ;; *) problem "this is not Raspberry Pi OS or Debian ($os_like)" ;; esac
fi
if command -v file >/dev/null && ! file -b "$binary" | grep -qi 'aarch64\|ARM aarch64'; then
    problem "$binary is not an aarch64 build: $(file -b "$binary" | cut -c1-70). Build it on the Pi (deploy/build-release.sh)"
fi
grep -qaF 'client/tt_client_bg.wasm' "$binary" || problem "$binary has no offline client: build it with deploy/build-release.sh"
grep -q '^PORT=' "$profile" || note "the profile sets no PORT: using 8080"
port="$(pget PORT 8080)"
backup_dir="$(pget BACKUP_DIR "$DATA_ROOT/backups")"
note "profile $(basename "$profile"), binary $binary, port $port"

# ── 2. Packages ─────────────────────────────────────────────────────────────

if ! command -v curl >/dev/null 2>&1; then
    say "installing curl"
    run apt-get install -y curl
fi

# ── 3. The SSD ──────────────────────────────────────────────────────────────

say "checking the SSD at $DATA_ROOT"
if findmnt --mountpoint "$DATA_ROOT" >/dev/null 2>&1; then
    src="$(findmnt -n -o SOURCE --mountpoint "$DATA_ROOT")"
    case "$src" in
        /dev/mmcblk*) problem "$DATA_ROOT is on the SD card ($src): an SD card wears out under the database. See docs/event/PI_STORAGE.md" ;;
        *) note "mounted from $src" ;;
    esac
else
    problem "$DATA_ROOT is not a mounted SSD. Attach, format and mount it by UUID as docs/event/PI_STORAGE.md steps 1-4 say, then run this again. (It does not partition disks.)"
fi

# ── 4. The network ──────────────────────────────────────────────────────────

say "network: deploy/pi/network/setup.sh"
if [[ $dry_run -eq 1 ]]; then "$HERE/network/setup.sh" --dry-run; else "$HERE/network/setup.sh"; fi

# ── 5. The env file ─────────────────────────────────────────────────────────

say "settings: $APP_DIR/.env"
if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    note "the $SERVICE_USER user does not exist yet (install.sh makes it); making it now so the file can belong to it"
    run useradd --system --no-create-home --home-dir "$APP_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
fi
run install -d -m 0755 "$APP_DIR"

env_file="${BOOTSTRAP_ENV_FILE:-$APP_DIR/.env}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"
if [[ -f "$env_file" ]]; then
    cp "$env_file" "$work/env"
    note "keeping what $env_file already has; changing only the profile's keys"
else
    cp "$HERE/service/tealteam.env.example" "$work/env"
fi

# set_key KEY VALUE: replace the KEY= line, else the first #KEY= line, else append.
set_key() {
    K="$1" V="$2" awk '
        BEGIN { k = ENVIRON["K"]; v = ENVIRON["V"] }
        { lines[NR] = $0 }
        END {
            hit = 0
            for (i = 1; i <= NR; i++) if (index(lines[i], k "=") == 1) { lines[i] = k "=" v; hit = 1 }
            if (!hit) for (i = 1; i <= NR; i++) if (index(lines[i], "#" k "=") == 1) { lines[i] = k "=" v; hit = 1; break }
            for (i = 1; i <= NR; i++) print lines[i]
            if (!hit) print k "=" v
        }' "$work/env" > "$work/env.new"
    mv "$work/env.new" "$work/env"
}

secret_keys=",$(pget SECRET_KEYS),"
changed=()
while IFS='=' read -r key value; do
    [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || continue
    case "$key" in PREP_* | SECRET_KEYS) continue ;; esac
    value="${value%$'\r'}"
    if [[ "$secret_keys" == *",$key,"* ]]; then
        if [[ -z "$value" ]]; then
            if [[ $dry_run -eq 1 ]]; then
                note "$key: would ask for it (empty answer = leave unset)"
                continue
            elif [[ $interactive -eq 0 || ! -t 0 ]]; then
                note "$key: skipped (--non-interactive); set it in $env_file later"
                continue
            fi
            printf '%s (hidden; Enter to skip): ' "$key" >&2
            read -rs value
            printf '\n' >&2
            [[ -n "$value" ]] || { note "$key: skipped"; continue; }
        fi
        set_key "$key" "$value"
        changed+=("$key=(secret)")
    elif [[ -n "$value" ]]; then
        set_key "$key" "$value"
        changed+=("$key=$value")
    else
        note "$key: empty in the profile, left as the example has it"
    fi
done < "$profile"
for c in "${changed[@]:-}"; do [[ -z "$c" ]] || note "set $c"; done

if [[ $dry_run -eq 1 ]]; then
    note "would write $env_file, mode 600, owner $SERVICE_USER"
else
    if [[ ! -f "$env_file" ]] || ! cmp -s "$work/env" "$env_file"; then
        if id "$SERVICE_USER" >/dev/null 2>&1; then
            install -m 0600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$work/env" "$env_file"
        else
            install -m 0600 "$work/env" "$env_file"
        fi
        note "wrote $env_file (mode 600)"
        touch "$work/restart"
    else
        note "$env_file is already right"
    fi
fi

# ── 6. The service ──────────────────────────────────────────────────────────

say "service: deploy/pi/service/install.sh"
if [[ $dry_run -eq 1 ]]; then
    "$HERE/service/install.sh" "$binary" --dry-run
else
    "$HERE/service/install.sh" "$binary"
    # install.sh restarts only for a new binary or service file; a changed .env needs one too.
    [[ ! -e "$work/restart" ]] || systemctl restart tealteam
fi

# ── 7. Wait for it ──────────────────────────────────────────────────────────

say "waiting for http://localhost:$port/health"
if [[ $dry_run -eq 1 ]]; then
    note "would wait up to 60 s for \"storage\":\"ready\""
else
    ready=0
    for _ in $(seq 1 30); do
        if curl -fsS --max-time 3 "http://localhost:$port/health" 2>/dev/null | grep -q '"storage":"ready"'; then ready=1; break; fi
        sleep 2
    done
    if [[ $ready -eq 0 ]]; then
        echo "the server did not become ready: journalctl -u tealteam -n 50" >&2
        exit 1
    fi
    note "storage ready"
fi

# ── 8. One backup, and a restore check ──────────────────────────────────────

say "backup and restore check"
if [[ $dry_run -eq 1 ]]; then
    note "would run: tt-web backup $backup_dir, then tt-web check-backup (as $SERVICE_USER)"
else
    as_service() { runuser -u "$SERVICE_USER" -- bash -c "cd $APP_DIR && $*"; }
    as_service "./tt-web backup '$backup_dir'"
    as_service "./tt-web check-backup"
fi

# ── 9. Status ───────────────────────────────────────────────────────────────

say "status"
if [[ $dry_run -eq 1 ]]; then
    note "would run deploy/pi/network/status.sh"
else
    "$HERE/network/status.sh" || note "status.sh reported a FAIL above: each names its fix"
fi
cat <<EOF

The server is at http://tealteam.local/  (or http://tealteam, or http://10.101.0.1/)
and, once a device trusts http://tealteam.local/ca.crt, https://tealteam.local/
(docs/event/HTTPS_SETUP.md). Copy the CA folder (/srv/tealteam/data/tls, or
TLS_DIR) to the backup stick once: without it, every device must trust a new CA.
Next: deploy/event/prep-event.sh on this Pi (over SSH), then each Chromebook's
checklist (docs/event/SETUP_GUIDE.md).
EOF
