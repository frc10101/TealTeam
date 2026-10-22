#!/usr/bin/env bash
# Run the server on Maxwell's Omen laptop (Omarchy) for a demo, in place of
# the Pi. The lead scout works on this laptop at http://localhost:PORT/; the
# Chromebooks use its address on the same network.
#
#   deploy/laptop/omenMaxwell-setup.sh [start] [--dir DIR] [--port PORT] [--tls-port PORT|off] [--build | --binary FILE]
#   deploy/laptop/omenMaxwell-setup.sh stop [--close-firewall]
#   deploy/laptop/omenMaxwell-setup.sh status
#
# start: builds the release binary if there is none (or with --build), or
# takes one built elsewhere with --binary (any x86_64 Linux build runs here:
# it needs glibc 2.34, and the Omen needs no Rust toolchain), copies
# it into DIR (default ~/tealteam-demo) with a prod .env, opens PORT and the
# HTTPS port (default 8443, H1; its CA is made in DIR/data/tls) in ufw
# (asks for sudo), and starts it in the background under systemd-inhibit, so
# closing the lid or idling does not suspend the laptop while it serves. An
# existing DIR keeps its database: rerunning is a restart, not a wipe.
# Then load accounts, schedule and assignments with prep-event.sh (printed).
#
# Unlike the Pi, this laptop hands out no addresses and answers no
# tealteam.local: the Chromebooks need a network with its own router, and use
# the http://IP:PORT/ this prints.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

action=start dir="$HOME/tealteam-demo" port=8080 tls_port=8443 build=0 close=0 bin="$ROOT/target/release/tt-web"
if [[ $# -gt 0 && $1 != --* ]]; then action="$1"; shift; fi
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dir) dir="${2:?--dir needs a folder}"; shift 2 ;;
        --port) port="${2:?--port needs a number}"; shift 2 ;;
        --tls-port) tls_port="${2:?--tls-port needs a number or off}"; shift 2 ;;
        --build) build=1; shift ;;
        --binary) bin="${2:?--binary needs a file}"; shift 2 ;;
        --close-firewall) close=1; shift ;;
        *) echo "usage: $0 [start|stop|status] [--dir DIR] [--port PORT] [--tls-port PORT|off] [--build | --binary FILE] [--close-firewall]" >&2; exit 2 ;;
    esac
done
pidfile="$dir/server.pid"

running() { [[ -f $pidfile ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; }
# The address other devices reach: the one on the default route.
lan_ip() { ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}'; }

case "$action" in
    status)
        if running; then
            say "running (pid $(cat "$pidfile")) from $dir"
            curl -fsS "http://127.0.0.1:$port/health" && echo
        else
            say "not running"
        fi
        exit 0 ;;
    stop)
        if running; then
            # systemd-inhibit passes the signal on and lets go of the lid.
            kill "$(cat "$pidfile")"
            say "stopped"
        else
            say "not running"
        fi
        rm -f "$pidfile"
        if [[ $close -eq 1 ]]; then
            sudo ufw delete allow "$port/tcp"
            [[ $tls_port == off ]] || sudo ufw delete allow "$tls_port/tcp"
        fi
        exit 0 ;;
    start) ;;
    *) echo "unknown action: $action (start, stop or status)" >&2; exit 2 ;;
esac

if running; then
    say "already running (pid $(cat "$pidfile")); stop it first: $0 stop"
    exit 1
fi

say "binary"
if [[ $build -eq 1 || ! -x $bin ]]; then
    [[ $bin == "$ROOT/target/release/tt-web" ]] || { echo "no binary at $bin" >&2; exit 1; }
    "$ROOT/deploy/build-release.sh"
fi
mkdir -p "$dir/data" "$dir/backups"
cp "$bin" "$dir/tt-web"
note "copied to $dir/tt-web"

say "settings"
if [[ -f $dir/.env ]]; then
    note "keeping $dir/.env"
else
    cat > "$dir/.env" <<EOF
TEALTEAM_ENV=prod
PORT=$port
TLS_PORT=$tls_port
TLS_NAMES=$(hostname).local,$(lan_ip)
DATABASE_URL=sqlite://$dir/data/tealteam.db
BACKUP_DIR=$dir/backups
EOF
    note "wrote $dir/.env"
fi
grep -q "^PORT=$port\$" "$dir/.env" || note "WARNING: $dir/.env sets another PORT than $port"

say "firewall"
if systemctl is-active --quiet ufw; then
    sudo ufw allow "$port/tcp" comment "TealTeam" | sed 's/^/    /'
    [[ $tls_port == off ]] || sudo ufw allow "$tls_port/tcp" comment "TealTeam HTTPS" | sed 's/^/    /'
else
    note "ufw is off: nothing to open"
fi

say "server"
cd "$dir"
nohup systemd-inhibit --what=handle-lid-switch:sleep:idle --who=TealTeam \
    --why="serving TealTeam" --mode=block ./tt-web serve >> server.log 2>&1 &
echo $! > "$pidfile"
for _ in $(seq 1 50); do
    curl -fs "http://127.0.0.1:$port/health" > /dev/null && break
    sleep 0.2
done
if ! curl -fsS "http://127.0.0.1:$port/health"; then
    echo; echo "the server did not answer; the end of $dir/server.log:" >&2
    tail -n 20 server.log >&2
    exit 1
fi
echo

ip="$(lan_ip)"
say "ready"
note "this laptop (lead scout): http://localhost:$port/"
note "Chromebooks:              http://${ip:-THIS-LAPTOP-IP}:$port/"
note "                          http://$(hostname).local:$port/  (if the name resolves there)"
if [[ $tls_port != off ]]; then
    note "HTTPS, once a device trusts http://${ip:-THIS-LAPTOP-IP}:$port/ca.crt:"
    note "                          https://${ip:-THIS-LAPTOP-IP}:$tls_port/  (docs/event/HTTPS_SETUP.md)"
    note "the CA lives in $dir/data/tls: keep it, or every device trusts a new one"
    note "the certificate covers this laptop's name and $ip; for others, TLS_NAMES=... in $dir/.env, then restart"
fi
note "log: $dir/server.log   stop: $0 stop --dir $dir"
note "keep it plugged in; the lid can close, but the screen still locks"
echo
say "next: accounts, schedule and assignments"
note "deploy/event/prep-event.sh deploy/profiles/ftc-qual-4robot.env --url http://127.0.0.1:$port --sheet $dir/signin-sheet.txt"
