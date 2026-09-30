#!/usr/bin/env bash
# Is the event-day network right? Read-only; safe to run any time, no sudo
# needed (P4, P6). One line per check, and a non-zero exit if any failed.
#
# UNTESTED on a Pi: checked with ShellCheck, never run. See docs/PI_NETWORK.md.

set -uo pipefail

failed=0
ok() { printf '  ok    %s\n' "$*"; }
bad() { printf '  FAIL  %s\n' "$*"; failed=1; }
note() { printf '  --    %s\n' "$*"; }
# check <ok message> <fail message> <command...>
check() {
    local good="$1" fail="$2"
    shift 2
    if "$@"; then ok "$good"; else bad "$fail"; fi
}

echo "TealTeam network"

name="$(hostname)"
check "hostname is tealteam" "hostname is ${name}, not tealteam: run setup.sh" [ "${name}" = tealteam ]

if ip -4 addr show eth0 2>/dev/null | grep -q "inet 10\.101\.0\.1/"; then
    ok "eth0 is 10.101.0.1"
else
    bad "eth0 does not have 10.101.0.1: is the switch cabled and powered?"
fi

uplink="$(ip route show default 2>/dev/null | head -n 1)"
case "$uplink" in
    *" dev usb0 "* | *" dev eth1 "*) ok "internet via the tethered phone: $uplink" ;;
    "") note "no uplink: plug in the phone and turn on USB tethering (the app works without it)" ;;
    *) note "internet via something other than the phone: $uplink" ;;
esac

aps="$(nmcli -t -f NAME,TYPE connection show 2>/dev/null | while IFS=: read -r name type; do
    [[ "$type" == "802-11-wireless" ]] || continue
    mode="$(nmcli -g 802-11-wireless.mode connection show "$name" 2>/dev/null)"
    [[ "$mode" == "ap" || "$mode" == "adhoc" ]] && printf '%s ' "$name"
done)"
check "no Wi-Fi access point profile (E143)" "Wi-Fi AP profile(s) present: ${aps}-- run setup.sh" [ -z "${aps}" ]

for unit in dnsmasq avahi-daemon tealteam-port80; do
    check "${unit} is running" "${unit} is not running" systemctl is-active --quiet "${unit}"
done

if command -v avahi-resolve >/dev/null; then
    resolved="$(avahi-resolve -4 -n tealteam.local 2>/dev/null)"
    check "mDNS: ${resolved}" "tealteam.local does not resolve over mDNS" [ -n "${resolved}" ]
fi

if curl -fsS -o /dev/null --max-time 3 http://localhost/health; then
    ok "the app answers on port 80"
else
    bad "nothing answers http://localhost/health: is tt-web running on 8080?"
fi

leases=/var/lib/misc/dnsmasq.leases
if [[ -r "$leases" ]]; then
    note "$(wc -l < "$leases") client lease(s) handed out"
fi

exit "$failed"
