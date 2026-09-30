#!/usr/bin/env bash
# The event-day network on the Pi (P4, P6). See docs/PI_NETWORK.md.
#
#   sudo ./setup.sh             apply
#   sudo ./setup.sh --dry-run   say what it would change, change nothing
#
# Idempotent: run it again after any change here, or whenever in doubt. It
# only ever writes the files it owns and restarts what they changed.
#
# UNTESTED on a Pi: written for Raspberry Pi OS Bookworm (NetworkManager),
# checked with ShellCheck, never run. docs/PI_NETWORK.md lists what to verify.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOSTNAME_WANTED="tealteam"
DRY_RUN=0

case "${1:-}" in
    "") ;;
    --dry-run) DRY_RUN=1 ;;
    *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

if [[ $DRY_RUN -eq 0 && $EUID -ne 0 ]]; then
    echo "run as root: sudo $0" >&2
    exit 1
fi

say() { printf '==> %s\n' "$*"; }

# Run a command, or only print it on a dry run.
run() {
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '    would run: %s\n' "$*"
    else
        "$@"
    fi
}

# Install $1 to $2 with mode $3 when the content differs. Succeeds (returns 0)
# only when it changed something, so callers know what to restart; a failed
# install stops the script.
place() {
    local src="$HERE/$1" dest="$2" mode="$3"
    if [[ -f "$dest" ]] && cmp -s "$src" "$dest"; then
        return 1
    fi
    say "installing $dest"
    # Exit by hand: `set -e` does not apply inside a function called from `&&`.
    run install -D -m "$mode" "$src" "$dest" || { echo "could not install $dest" >&2; exit 1; }
    return 0
}

# ── Packages ────────────────────────────────────────────────────────────────

missing=()
for pkg in network-manager dnsmasq avahi-daemon avahi-utils nftables; do
    dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "ok installed" || missing+=("$pkg")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    say "installing packages: ${missing[*]}"
    run apt-get install -y "${missing[@]}"
fi

# ── Name: tealteam.local (P4) ───────────────────────────────────────────────

if [[ "$(hostname)" != "$HOSTNAME_WANTED" ]]; then
    say "setting the hostname to $HOSTNAME_WANTED"
    run hostnamectl set-hostname "$HOSTNAME_WANTED"
fi
# Raspberry Pi OS maps the hostname to 127.0.1.1; keep sudo from stalling on
# a name it cannot resolve.
if ! grep -qE "^127\.0\.1\.1[[:space:]]+$HOSTNAME_WANTED\$" /etc/hosts; then
    say "pointing 127.0.1.1 at $HOSTNAME_WANTED in /etc/hosts"
    if grep -qE '^127\.0\.1\.1[[:space:]]' /etc/hosts; then
        run sed -i -E "s/^127\.0\.1\.1[[:space:]].*/127.0.1.1\t$HOSTNAME_WANTED/" /etc/hosts
    else
        run sh -c "printf '127.0.1.1\t%s\n' '$HOSTNAME_WANTED' >> /etc/hosts"
    fi
fi

restart_avahi=0
place tealteam-http.service.avahi /etc/avahi/services/tealteam.service 0644 && restart_avahi=1

# ── Links: wired clients, tethered uplink (P6) ──────────────────────────────

reload_nm=0
place tealteam-lan.nmconnection /etc/NetworkManager/system-connections/tealteam-lan.nmconnection 0600 && reload_nm=1
place tealteam-uplink.nmconnection /etc/NetworkManager/system-connections/tealteam-uplink.nmconnection 0600 && reload_nm=1
if [[ $reload_nm -eq 1 ]]; then
    run nmcli connection reload
fi

# No Wi-Fi access point, ever: it violates E143. Delete any AP or hotspot
# profile, and rank a shop Wi-Fi client below the tethered phone.
while IFS=: read -r name type; do
    [[ "$type" == "802-11-wireless" ]] || continue
    mode="$(nmcli -g 802-11-wireless.mode connection show "$name" 2>/dev/null || true)"
    if [[ "$mode" == "ap" || "$mode" == "adhoc" ]]; then
        say "deleting Wi-Fi $mode profile \"$name\" (E143)"
        run nmcli connection delete "$name"
    elif [[ "$(nmcli -g ipv4.route-metric connection show "$name" 2>/dev/null || true)" != "600" ]]; then
        say "ranking Wi-Fi \"$name\" below the tethered phone"
        run nmcli connection modify "$name" ipv4.route-metric 600 ipv6.route-metric 600
    fi
done < <(nmcli -t -f NAME,TYPE connection show)

if ip link show eth0 >/dev/null 2>&1; then
    run nmcli connection up tealteam-lan || say "eth0 did not come up yet; is the switch plugged in?"
fi

# ── Addresses and names for the clients ─────────────────────────────────────

restart_dnsmasq=0
place dnsmasq-tealteam.conf /etc/dnsmasq.d/tealteam.conf 0644 && restart_dnsmasq=1

# ── Port 80 ─────────────────────────────────────────────────────────────────

restart_port80=0
place port80.nft /etc/tealteam/port80.nft 0644 && restart_port80=1
place tealteam-port80.service /etc/systemd/system/tealteam-port80.service 0644 && {
    restart_port80=1
    run systemctl daemon-reload
}

# ── Services ────────────────────────────────────────────────────────────────

for unit in dnsmasq avahi-daemon tealteam-port80; do
    if ! systemctl is-enabled --quiet "$unit" 2>/dev/null; then
        say "enabling $unit"
        run systemctl enable "$unit"
    fi
done
[[ $restart_dnsmasq -eq 1 ]] && run systemctl restart dnsmasq
[[ $restart_avahi -eq 1 ]] && run systemctl restart avahi-daemon
[[ $restart_port80 -eq 1 ]] && run systemctl restart tealteam-port80
for unit in dnsmasq avahi-daemon tealteam-port80; do
    systemctl is-active --quiet "$unit" 2>/dev/null || run systemctl start "$unit"
done

say "done. Check it with: $HERE/status.sh"
