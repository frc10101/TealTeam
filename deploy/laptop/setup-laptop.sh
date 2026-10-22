#!/usr/bin/env bash
# Get a Linux or macOS laptop (the lead's or the admin's) ready for the event
# LAN (SK1). The scouts' Chromebooks cannot run this: see docs/event/SETUP_GUIDE.md.
#
#   deploy/laptop/setup-laptop.sh [--url http://tealteam.local] [--dry-run]
#   deploy/laptop/setup-laptop.sh --undo     put Wi-Fi and sleep back
#
# Checks for a wired link, turns Wi-Fi off, checks the server answers and says
# its version, puts a shortcut to the server on the desktop, and keeps the
# laptop awake while it is on AC. Needs no internet.
#
# UNTESTED on a real event LAN or on macOS: --dry-run only, on Linux.

set -euo pipefail

url="http://tealteam.local" DRY_RUN=0 UNDO=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --url) [[ $# -ge 2 ]] || { echo "--url needs a value" >&2; exit 2; }; url="${2%/}"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        --undo) UNDO=1; shift ;;
        *) echo "usage: $0 [--url URL] [--dry-run] [--undo]" >&2; exit 2 ;;
    esac
done

say() { printf '==> %s\n' "$*"; }
run() {
    if [[ $DRY_RUN -eq 1 ]]; then printf '    would run: %s\n' "$*"; else "$@" || echo "    that did not work: $*"; fi
}
os="$(uname -s)"

if [[ $UNDO -eq 1 ]]; then
    say "putting things back"
    if [[ "$os" == Darwin ]]; then
        run sudo pmset -c sleep 10
        echo "    Wi-Fi: turn it on in the menu bar when you want it"
    else
        run gsettings reset org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type
        run gsettings reset org.gnome.desktop.session idle-delay
        command -v nmcli >/dev/null && run nmcli radio wifi on
    fi
    exit 0
fi

say "wired link"
wired=0
if [[ "$os" == Darwin ]]; then
    networksetup -listallhardwareports | grep -qi ethernet && wired=1
    ifconfig 2>/dev/null | grep -q "status: active" || wired=0
elif command -v ip >/dev/null; then
    for dev in /sys/class/net/*; do
        [[ -d "$dev/wireless" || "$(basename "$dev")" == lo ]] && continue
        [[ "$(cat "$dev/carrier" 2>/dev/null || echo 0)" == 1 ]] && wired=1
    done
fi
[[ $wired -eq 1 ]] && echo "    a wired link is up" || echo "    NO wired link found: plug the cable into the switch (USB-C Ethernet adapter if needed)"

say "Wi-Fi off (event data is wired only)"
if [[ "$os" == Darwin ]]; then
    dev="$(networksetup -listallhardwareports | awk '/Wi-Fi/ {getline; print $2}')"
    [[ -n "$dev" ]] && run networksetup -setairportpower "$dev" off
elif command -v nmcli >/dev/null; then
    run nmcli radio wifi off
else
    echo "    turn Wi-Fi off yourself, in your network settings"
fi

say "the server at $url"
if status="$(curl -fsS --max-time 5 "$url/health" 2>&1)"; then
    echo "    answers: $status"
else
    echo "    does not answer. Try --url http://10.101.0.1 ; if that fails, check the cable and the Pi"
fi

say "shortcut on the desktop"
desk="${XDG_DESKTOP_DIR:-$HOME/Desktop}"
if [[ "$os" == Darwin ]]; then
    f="$desk/TealTeam.webloc"
    body="<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>URL</key><string>$url/</string></dict></plist>"
else
    f="$desk/TealTeam.desktop"
    body="[Desktop Entry]
Type=Link
Name=TealTeam
URL=$url/"
fi
if [[ $DRY_RUN -eq 1 ]]; then echo "    would write $f"; elif [[ -d "$desk" ]]; then printf '%s\n' "$body" > "$f"; echo "    wrote $f"; else echo "    no $desk folder: bookmark $url/ yourself"; fi

say "stay awake on AC (undo with --undo)"
if [[ "$os" == Darwin ]]; then
    run sudo pmset -c sleep 0
elif command -v gsettings >/dev/null; then
    run gsettings set org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type nothing
    run gsettings set org.gnome.desktop.session idle-delay 0
else
    echo "    set \"never sleep while charging\" in your power settings"
fi

cat <<EOT

Still by hand, in the browser:
  [ ] open $url/ and sign in (the sign-in sheet has your password)
  [ ] clear old site data for tealteam from earlier events (browser settings, site data)
  [ ] bookmark $url/ and $url/submission
  [ ] if tealteam.local fails, use http://10.101.0.1/
EOT
