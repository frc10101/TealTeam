#!/usr/bin/env bash
# Get a Linux or macOS laptop ready for an event (F12). OPTIONAL: the team's
# Lenovo Chromebooks cannot run this; they use the ChromeOS checklist in
# docs/SETUP_GUIDE.md. This is for a Linux/Mac laptop if you have one.
#
#   deploy/laptop/setup-laptop.sh [--pi-ip 10.101.0.1] [--dry-run] [--undo]
#
# It: confirms a wired link, turns Wi-Fi off, checks tealteam.local resolves
# (else says to use the IP), fetches /health and prints the server's build,
# puts a shortcut to the server on the desktop, and stops the laptop sleeping
# on AC. --undo turns Wi-Fi on, removes the shortcut, restores sleep.
# It changes this laptop's network and power settings, so --dry-run first.
# Needs no internet. Browser steps it cannot do are printed at the end.
#
# UNTESTED on a real laptop: only --dry-run was run, on Linux.

set -euo pipefail

ip="10.101.0.1" dry=0 undo=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --pi-ip) ip="${2:?--pi-ip needs an address}"; shift 2 ;;
        --dry-run) dry=1; shift ;;
        --undo) undo=1; shift ;;
        *) echo "usage: $0 [--pi-ip IP] [--dry-run] [--undo]" >&2; exit 2 ;;
    esac
done
say() { printf '==> %s\n' "$*"; }
run() { if [[ $dry -eq 1 ]]; then printf '    would run: %s\n' "$*"; else "$@"; fi; }
os="$(uname -s)"
desktop="${XDG_DESKTOP_DIR:-$HOME/Desktop}"
link="$desktop/TealTeam"

if [[ $undo -eq 1 ]]; then
    say "undo"
    case "$os" in
        Linux)
            command -v nmcli >/dev/null && run nmcli radio wifi on
            command -v gsettings >/dev/null && run gsettings reset org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type ;;
        Darwin)
            run networksetup -setairportpower "$(networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}')" on
            echo "    sleep: sudo pmset -c sleep 10   (or your usual minutes)" ;;
    esac
    run rm -f "$link.desktop" "$link.webloc"
    exit 0
fi

say "wired link"
case "$os" in
    Linux) wired="$(ip -o link show up 2>/dev/null | awk -F': ' '$2 ~ /^(en|eth)/ {print $2}' | head -n 1)" ;;
    Darwin) wired="$(ifconfig 2>/dev/null | awk '/^en[0-9]+:/{i=$1} /status: active/{print i}' | tr -d : | head -n 1)" ;;
    *) echo "this is for Linux and macOS; Windows has setup-laptop.ps1" >&2; exit 1 ;;
esac
if [[ -z "$wired" ]]; then
    echo "    no wired link is up: plug the Ethernet cable (or adapter) into the switch" >&2
    [[ $dry -eq 1 ]] || exit 1
else
    echo "    $wired is up"
fi

say "Wi-Fi off (no radio for event data)"
case "$os" in
    Linux) command -v nmcli >/dev/null && run nmcli radio wifi off || echo "    turn Wi-Fi off in your network settings" ;;
    Darwin) run networksetup -setairportpower "$(networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}')" off ;;
esac

say "finding the server"
url="http://tealteam.local"
if { getent hosts tealteam.local || dscacheutil -q host -a name tealteam.local; } 2>/dev/null | grep -q .; then
    echo "    tealteam.local resolves"
else
    url="http://$ip"
    echo "    tealteam.local does not resolve here: using $url"
fi
if [[ $dry -eq 1 ]]; then
    echo "    would fetch $url/health"
else
    health="$(curl -fsS --max-time 5 "$url/health")" || { echo "    cannot reach $url: cable, switch, Pi power" >&2; exit 1; }
    echo "    server answers: $health"
fi

say "desktop shortcut"
if [[ $dry -eq 1 ]]; then
    echo "    would write $link for $url/ and $url/submission"
else
    mkdir -p "$desktop"
    if [[ "$os" == Darwin ]]; then
        printf '<?xml version="1.0"?><plist version="1.0"><dict><key>URL</key><string>%s/</string></dict></plist>\n' "$url" > "$link.webloc"
    else
        printf '[Desktop Entry]\nType=Link\nName=TealTeam\nURL=%s/\n' "$url" > "$link.desktop"
    fi
fi

say "no sleep on AC"
case "$os" in
    Linux) command -v gsettings >/dev/null && run gsettings set org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type nothing || echo "    set sleep to Never while plugged in in your power settings" ;;
    Darwin) echo "    run: sudo pmset -c sleep 0   (undo: --undo)" ;;
esac

cat <<EOF

Browser steps this cannot do:
  - open $url/ and sign in with the name and temporary password from the sign-in sheet
  - clear old site data for it from earlier events (browser settings, site data)
  - open the scouting page once so the offline copy downloads
EOF
