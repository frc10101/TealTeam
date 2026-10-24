#!/usr/bin/env bash
# Run the server on Maxwell's Omen laptop (Omarchy) in place of the Pi. The
# lead scout works on this laptop at http://localhost:PORT/; the Chromebooks,
# cabled to a switch on the laptop's Ethernet port, at
# https://tealteam.local:TLS_PORT/.
#
#   deploy/laptop/omenMaxwell-setup.sh [start] [--dir DIR] [--port PORT] [--tls-port PORT|off] [--lan IFACE|off] [--build | --binary FILE]
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
#
# The wired port (--lan, default the Ethernet device) is the Pi's network:
# NetworkManager shared mode on 10.101.0.1/24 (the tealteam-lan profile, which
# only this script brings up, so the laptop hands out no addresses on another
# network), dnsmasq-shared-tealteam.conf for the names and no gateway, and
# tealteam.local published over mDNS, which is all ChromeOS asks for .local.
# stop takes the port down again. --lan off serves on whatever network the
# laptop is on, at the http://IP:PORT/ this prints.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

action=start dir="$HOME/tealteam-demo" port=8080 tls_port=8443 build=0 close=0 bin="$ROOT/target/release/tt-web"
lan="$(nmcli -t -f DEVICE,TYPE device 2>/dev/null | awk -F: '$2 == "ethernet" { print $1; exit }')"
lan_addr=10.101.0.1
mdns_unit=tealteam-mdns
if [[ $# -gt 0 && $1 != --* ]]; then action="$1"; shift; fi
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dir) dir="${2:?--dir needs a folder}"; shift 2 ;;
        --port) port="${2:?--port needs a number}"; shift 2 ;;
        --tls-port) tls_port="${2:?--tls-port needs a number or off}"; shift 2 ;;
        --lan) lan="${2:?--lan needs an interface or off}"; shift 2 ;;
        --build) build=1; shift ;;
        --binary) bin="${2:?--binary needs a file}"; shift 2 ;;
        --close-firewall) close=1; shift ;;
        *) echo "usage: $0 [start|stop|status] [--dir DIR] [--port PORT] [--tls-port PORT|off] [--lan IFACE|off] [--build | --binary FILE] [--close-firewall]" >&2; exit 2 ;;
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
        if nmcli -t -f NAME connection show --active | grep -qx tealteam-lan; then
            say "tealteam-lan up on $(nmcli -g GENERAL.DEVICES connection show tealteam-lan): $(nmcli -g ipv4.method,ipv4.addresses connection show tealteam-lan | tr '\n' ' ')"
        else
            say "tealteam-lan down"
        fi
        systemctl --user is-active --quiet "$mdns_unit" && say "publishing tealteam.local" || say "not publishing tealteam.local"
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
        systemctl --user stop "$mdns_unit" 2>/dev/null || true
        if nmcli -t -f NAME connection show --active | grep -qx tealteam-lan; then
            sudo nmcli connection down tealteam-lan > /dev/null && say "tealteam-lan down"
        fi
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
# The certificate always covers tealteam.local, tealteam and 10.101.0.1 (the
# Pi's names); add this laptop's own.
names="$(hostname).local"
ip="$(lan_ip)"
[[ -z $ip || $ip == "$lan_addr" ]] || names="$names,$ip"
if [[ -f $dir/.env ]]; then
    note "keeping $dir/.env"
else
    cat > "$dir/.env" <<EOF
TEALTEAM_ENV=prod
PORT=$port
TLS_PORT=$tls_port
TLS_NAMES=$names
DATABASE_URL=sqlite://$dir/data/tealteam.db
BACKUP_DIR=$dir/backups
EOF
    note "wrote $dir/.env"
fi
grep -q "^PORT=$port\$" "$dir/.env" || note "WARNING: $dir/.env sets another PORT than $port"

if [[ $lan != off ]]; then
    say "network: $lan as the Pi's wired side ($lan_addr)"
    [[ -n $lan ]] || { echo "no Ethernet device; name one with --lan, or --lan off" >&2; exit 1; }
    # Shared mode runs NetworkManager's own dnsmasq; it needs the binary only.
    command -v dnsmasq > /dev/null || sudo pacman -S --needed --noconfirm dnsmasq
    conf=/etc/NetworkManager/dnsmasq-shared.d/tealteam.conf
    if ! cmp -s "$HERE/dnsmasq-shared-tealteam.conf" "$conf"; then
        sudo install -D -m 0644 "$HERE/dnsmasq-shared-tealteam.conf" "$conf"
        note "installed $conf"
    fi
    settings=(connection.interface-name "$lan" connection.autoconnect no
        ipv4.method shared ipv4.addresses "$lan_addr/24" ipv4.never-default yes ipv6.method disabled)
    if nmcli -t -f NAME connection show | grep -qx tealteam-lan; then
        sudo nmcli connection modify tealteam-lan "${settings[@]}"
    else
        sudo nmcli connection add type ethernet con-name tealteam-lan "${settings[@]}" > /dev/null
    fi
    sudo nmcli connection up tealteam-lan > /dev/null \
        && note "tealteam-lan up: clients get 10.101.0.x, no gateway" \
        || note "WARNING: tealteam-lan did not come up; is $lan cabled to the switch? Rerun once it is"
    systemctl --user stop "$mdns_unit" 2>/dev/null || true
    systemd-run --user --quiet --collect --unit="$mdns_unit" avahi-publish -a -R tealteam.local "$lan_addr"
    note "publishing tealteam.local -> $lan_addr over mDNS"
fi

say "firewall"
if systemctl is-active --quiet ufw; then
    sudo ufw allow "$port/tcp" comment "TealTeam" | sed 's/^/    /'
    [[ $tls_port == off ]] || sudo ufw allow "$tls_port/tcp" comment "TealTeam HTTPS" | sed 's/^/    /'
else
    note "ufw is off: nothing to open"
fi
if [[ $lan != off ]] && systemctl is-active --quiet ufw; then
    # DHCP and DNS for the clients, on the wired port only.
    sudo ufw allow in on "$lan" to any port 67 proto udp comment "TealTeam DHCP" | sed 's/^/    /'
    sudo ufw allow in on "$lan" to any port 53 comment "TealTeam DNS" | sed 's/^/    /'
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

say "ready"
note "this laptop (lead scout): http://localhost:$port/"
if [[ $lan != off && $tls_port != off ]]; then
    note "Chromebooks, on the switch: https://tealteam.local:$tls_port/"
    note "  trust the CA first:       http://tealteam.local:$port/ca.crt  (docs/event/HTTPS_SETUP.md)"
    note "  without mDNS:             https://$lan_addr:$tls_port/"
elif [[ $lan != off ]]; then
    note "Chromebooks, on the switch: http://tealteam.local:$port/"
elif [[ $tls_port == off ]]; then
    note "Chromebooks:              http://${ip:-THIS-LAPTOP-IP}:$port/"
else
    note "HTTPS, once a device trusts http://${ip:-THIS-LAPTOP-IP}:$port/ca.crt:"
    note "                          https://${ip:-THIS-LAPTOP-IP}:$tls_port/  (docs/event/HTTPS_SETUP.md)"
fi
if [[ $tls_port != off ]]; then
    note "the CA lives in $dir/data/tls: keep it, or every device trusts a new one"
    note "the certificate covers tealteam.local, $lan_addr and $(grep '^TLS_NAMES=' "$dir/.env" | cut -d= -f2); for others, TLS_NAMES=... in $dir/.env, then restart"
fi
note "log: $dir/server.log   stop: $0 stop --dir $dir"
note "keep it plugged in; the lid can close, but the screen still locks"
echo
say "next: accounts, schedule and assignments"
note "deploy/event/prep-event.sh deploy/profiles/ftc-qual-4robot.env --url http://127.0.0.1:$port --sheet $dir/signin-sheet.txt"
