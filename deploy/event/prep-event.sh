#!/usr/bin/env bash
# Get a running server ready for an event, over HTTP, as the admin (SK1).
#
#   deploy/event/prep-event.sh deploy/profiles/frc-standard.env
#   deploy/event/prep-event.sh <profile> --url http://10.101.0.1
#
# The main way: on the Pi itself, over SSH (the scouts' Chromebooks cannot run
# shell scripts). Run on a Pi named tealteam, it talks to http://127.0.0.1:8080
# by itself. From a Linux/macOS laptop on the switch, give --url.
# It drives the same forms a person would, with curl:
#   1. signs in as the admin (asks for the password, no echo);
#   2. makes an account for each person in the roster (PREP_ROSTER) that has
#      none, with a random temporary password, and sets each one's role and
#      team on the Accounts page. An account that exists is left alone, its
#      password too;
#   3. gets the schedule the profile's way (PREP_SCHEDULE) and checks the
#      event has matches;
#   4. hands the robots of the unplayed matches out among the first
#      PREP_SCOUTS scouts, as the lead scout's Hand out open robots does;
#   5. writes the sign-in sheet (names, emails, temporary passwords) to a file
#      made private (mode 600), when it made accounts. It is never printed and
#      never in git; a second run adds to it rather than replacing it.
# Safe to run again; it adds only what is missing.
#
# Not done here, because the server has no page for it: the first admin
# account is made by whoever signs up first (this script offers to, when the
# server has no accounts yet); tablets' names; the rankings.
#
# For scripts: PREP_ADMIN_EMAIL, PREP_ADMIN_PASSWORD (and PREP_ADMIN_NAME, for
# a first account) in the environment skip the questions. Needs curl.

set -euo pipefail

usage() {
    echo "usage: $0 <profile-file> [--url http://tealteam.local]" >&2
    exit 2
}

profile="" url="http://tealteam.local" url_given=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --url) [[ $# -ge 2 ]] || usage; url="$2"; url_given=1; shift 2 ;;
        -*) usage ;;
        *) [[ -z "$profile" ]] || usage; profile="$1"; shift ;;
    esac
done
[[ -n "$profile" ]] || usage
[[ -f "$profile" ]] || { echo "no profile file $profile" >&2; exit 2; }
[[ "$url_given" -eq 1 || "$(hostname)" != tealteam ]] || url="http://127.0.0.1:8080"
url="${url%/}"
command -v curl >/dev/null || { echo "curl is needed" >&2; exit 1; }

say() { printf '==> %s\n' "$*"; }
die() { echo "stopped: $*" >&2; exit 1; }

# ── The profile (read, not sourced) ─────────────────────────────────────────
declare -A P=()
while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    [[ "$line" =~ ^[[:space:]]*([A-Z][A-Z0-9_]*)=(.*)$ ]] || die "profile line not KEY=VALUE: $line"
    val="${BASH_REMATCH[2]}"
    val="${val#\"}"; val="${val%\"}"; val="${val#\'}"; val="${val%\'}"
    P["${BASH_REMATCH[1]}"]="$val"
done < "$profile"

scouts_wanted="${P[PREP_SCOUTS]:-6}"
[[ "$scouts_wanted" =~ ^[1-9][0-9]*$ ]] || die "PREP_SCOUTS must be a whole number above zero"
schedule="${P[PREP_SCHEDULE]:-sync}"
[[ "$schedule" == sync || "$schedule" == existing ]] || die "PREP_SCHEDULE must be sync or existing"
default_team="${P[PREP_TEAM_NUMBER]:-${P[FIRST_TEAM_NUMBER]:-}}"
roster="${P[PREP_ROSTER]:-}"
[[ -f "$roster" ]] || die "no roster file \"$roster\" (PREP_ROSTER): copy deploy/event/roster.example.csv and edit it"

event_key="${P[PREP_EVENT_KEY]:-}"
if [[ -z "$event_key" ]]; then
    code="${P[PREP_EVENT_CODE]:-${P[FIRST_EVENT_CODE]:-}}"
    if [[ -z "$code" ]]; then
        read -rp "FIRST event code (e.g. TXHOU): " code < /dev/tty
    fi
    [[ -n "$code" ]] || die "no event: set PREP_EVENT_CODE or PREP_EVENT_KEY"
    event_key="${P[FIRST_SEASON]:-2026}$(printf '%s' "$code" | tr '[:upper:]' '[:lower:]')"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
jar="$work/jar"

# curl with the admin's cookies; the body to stdout. Callers read the status
# from the file named by $2 when they care.
get() { curl -sS --max-time 30 -b "$jar" -c "$jar" "$url$1"; }

# ── The server ──────────────────────────────────────────────────────────────
say "server at $url"
health="$(curl -sS --max-time 10 "$url/health")" || die "nothing answers at $url. Is the Pi on, and this laptop on the switch? Try --url http://10.101.0.1"
[[ "$health" == *'"storage":"ready"'* ]] || die "the server is up but its storage is not ready: $health"
echo "    $health"

# ── Sign in ─────────────────────────────────────────────────────────────────
# 0 when the cookie jar now holds a session.
login() {
    local code
    code="$(curl -sS --max-time 30 -o /dev/null -w '%{http_code}' -b "$jar" -c "$jar" \
        --data-urlencode "email=$1" --data-urlencode "password=$2" "$url/api/auth/login")"
    [[ "$code" == 303 ]]
}

say "sign in as the admin"
admin_email="${PREP_ADMIN_EMAIL:-}"
[[ -n "$admin_email" ]] || read -rp "    admin email: " admin_email < /dev/tty
admin_email="$(printf '%s' "$admin_email" | tr '[:upper:]' '[:lower:]')"
admin_pw="${PREP_ADMIN_PASSWORD:-}"
if [[ -z "$admin_pw" ]]; then
    read -rsp "    password: " admin_pw < /dev/tty
    echo
fi

if ! login "$admin_email" "$admin_pw"; then
    if get /sign-up | grep -q 'first account on this server'; then
        echo "    the server has no accounts yet. The first account becomes the admin."
        if [[ -z "${PREP_ADMIN_PASSWORD:-}" ]]; then
            read -rp "    create $admin_email now? [y/N] " yn < /dev/tty
            [[ "$yn" == [yY]* ]] || die "no admin account to use"
        fi
        admin_name="${PREP_ADMIN_NAME:-}"
        [[ -n "$admin_name" ]] || read -rp "    your name: " admin_name < /dev/tty
        code="$(curl -sS --max-time 30 -o /dev/null -w '%{http_code}' -b "$jar" -c "$jar" \
            --data-urlencode "name=${admin_name:-Admin}" --data-urlencode "email=$admin_email" \
            --data-urlencode "team_number=$default_team" \
            --data-urlencode "password=$admin_pw" --data-urlencode "confirm_password=$admin_pw" \
            "$url/api/auth/signup")"
        [[ "$code" == 303 ]] || die "the server refused that sign-up (password under 8 characters?)"
    else
        die "could not sign in as $admin_email: wrong email or password"
    fi
fi
[[ "$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar" "$url/admin/users")" == 200 ]] \
    || die "$admin_email is signed in but is not an admin"
echo "    signed in"

# ── Accounts ────────────────────────────────────────────────────────────────
# email<TAB>id for every account, read off the Accounts page.
account_ids() {
    get /admin/users | tr '\n' ' ' | sed 's/<tr>/\n<tr>/g' |
        sed -n 's/.*href="#account-\([0-9][0-9]*\)".*data-label="Email">\([^<]*\)<.*/\2\t\1/p'
}

temp_password() {
    head -c 512 /dev/urandom | LC_ALL=C tr -dc 'abcdefghjkmnpqrstuvwxyz23456789' | cut -c1-10
}

declare -A ids=()
refresh_ids() {
    ids=()
    local email id list
    list="$(account_ids)"
    while IFS=$'\t' read -r email id; do
        [[ -n "$email" ]] && ids["$email"]="$id"
    done <<< "$list"
}

say "accounts from $roster"
refresh_ids
sheet_rows=() scout_ids=() made=0 kept=0
while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    IFS=, read -r name email role team extra <<< "$line"
    [[ "$name" == name ]] && continue
    [[ -z "${extra:-}" ]] || die "too many fields (a comma in a name?): $line"
    name="$(printf '%s' "$name" | sed 's/^ *//; s/ *$//')"
    email="$(printf '%s' "$email" | tr -d ' ' | tr '[:upper:]' '[:lower:]')"
    role="$(printf '%s' "$role" | tr -d ' ' | tr '[:upper:]' '[:lower:]')"
    team="$(printf '%s' "${team:-}" | tr -d ' ')"
    [[ -n "$name" && "$email" == *@* ]] || die "needs a name and an email: $line"
    case "$role" in scout | lead | coach | admin) ;; *) die "role must be scout, lead, coach or admin: $line" ;; esac
    team="${team:-$default_team}"
    [[ -z "$team" || "$team" =~ ^[0-9]+$ ]] || die "team must be a number: $line"

    password=""
    if [[ -z "${ids[$email]:-}" ]]; then
        password="$(temp_password)"
        # A new cookie jar: this is the new person signing up, not the admin.
        code="$(curl -sS --max-time 30 -o /dev/null -w '%{http_code}' \
            --data-urlencode "name=$name" --data-urlencode "email=$email" \
            --data-urlencode "team_number=$team" \
            --data-urlencode "password=$password" --data-urlencode "confirm_password=$password" \
            "$url/api/auth/signup")"
        [[ "$code" == 303 ]] || die "the server refused the account for $email (is the email valid?)"
        refresh_ids
        [[ -n "${ids[$email]:-}" ]] || die "made $email but the Accounts page does not list it"
        made=$((made + 1))
        echo "    made   $email ($role)"
    else
        kept=$((kept + 1))
        echo "    exists $email ($role)"
    fi
    id="${ids[$email]}"

    # Roles and team, on the Accounts page. Never change the admin's own: a
    # roster line for them must not take their admin role away.
    if [[ "$email" != "$admin_email" ]]; then
        args=(--data-urlencode "team_number=$team")
        case "$role" in
            admin) args+=(--data-urlencode "is_admin=on") ;;
            lead) args+=(--data-urlencode "is_lead_scout=on") ;;
            coach) args+=(--data-urlencode "is_coach=on") ;;
        esac
        code="$(curl -sS --max-time 30 -o /dev/null -w '%{http_code}' -b "$jar" "${args[@]}" "$url/api/admin/users/$id")"
        [[ "$code" == 303 ]] || die "could not set the role of $email (HTTP $code)"
    fi
    [[ "$role" == scout ]] && scout_ids+=("$id")
    sheet_rows+=("$name|$email|$role|${password:-(existing account: password unchanged)}")
done < "$roster"
echo "    $made made, $kept already there"

# ── Schedule ────────────────────────────────────────────────────────────────
# True when the event has matches, read off the assignment grid; false if none
# or the event is unknown.
has_matches() {
    local page
    page="$(get "/lead-scout/assignments?event=$event_key" | tr '\n' ' ')"
    # An unknown event falls back to another one; the page then selects that.
    [[ "$page" == *"value=\"$event_key\" selected"* ]] || return 1
    [[ "$page" == *"robots in upcoming matches"* ]] && return 0
    [[ "$page" =~ Played\ matches\ \(([1-9][0-9]*)\) ]]
}

say "schedule for $event_key ($schedule)"
if [[ "$schedule" == sync ]]; then
    result="$(curl -sS --max-time 120 -b "$jar" -X POST "$url/api/frc/sync?event=$event_key" || true)"
    echo "    sync: ${result:-no answer}"
    if [[ "$result" != *'"ok":true'* ]]; then
        echo "    The sync did not fully work. The Pi has no internet at the venue: that is expected there."
        echo "    What matters is whether the matches were already loaded (tt-web bulk-load, at the shop)."
    fi
    # The pass that loads matches runs in the background after the sync.
    for _ in $(seq 1 30); do
        has_matches && break
        sleep 3
    done
fi
has_matches || die "$event_key has no matches on the server. Load them with 'tt-web bulk-load' at the shop or over a wired uplink, then run this again (docs/SETUP_GUIDE.md)"
echo "    matches are loaded"

# ── Assignments ─────────────────────────────────────────────────────────────
say "hand out the robots"
[[ ${#scout_ids[@]} -gt 0 ]] || die "the roster has no scouts to hand robots to"
if [[ ${#scout_ids[@]} -lt $scouts_wanted ]]; then
    echo "    note: the roster has ${#scout_ids[@]} scouts, fewer than PREP_SCOUTS=$scouts_wanted; using them all"
fi
args=()
for id in "${scout_ids[@]:0:$scouts_wanted}"; do
    args+=(--data-urlencode "assignee=u:$id")
done
location="$(curl -sS --max-time 60 -o "$work/assign.html" -w '%{http_code} %{redirect_url}' -b "$jar" \
    "${args[@]}" "$url/api/assignments/auto?event=$event_key")"
[[ "$location" == 303* ]] || die "the server refused the hand-out: open $url/lead-scout/assignments?event=$event_key and read why"
summary="$(get "/lead-scout/assignments?event=$event_key" | tr '\n' ' ' |
    sed -n 's/.*<strong>\([0-9]* of [0-9]*\)<\/strong> robots in upcoming matches have a scout.*/\1/p')"
echo "    robots with a scout: ${summary:-(every match was already played)}"

# ── Sign-in sheet ───────────────────────────────────────────────────────────
sheet="${P[PREP_SIGNIN_SHEET]:-signin-sheet-$event_key.txt}"
say "sign-in sheet $sheet"
if [[ $made -eq 0 ]]; then
    echo "    no new accounts: any sheet from an earlier run is left as it is"
else
    # Appended, so a second run never loses the first run's passwords.
    (umask 077 && touch "$sheet")
    {
        echo "Sign in at $url/sign-in    event $event_key    $(date +%F)"
        echo "Change the temporary password on the Account page. Keep this sheet off the internet and out of git."
        echo
        printf '%-24s %-34s %-6s %s\n' NAME EMAIL ROLE PASSWORD
        for row in "${sheet_rows[@]}"; do
            IFS='|' read -r n e r p <<< "$row"
            printf '%-24s %-34s %-6s %s\n' "$n" "$e" "$r" "$p"
        done
        echo
    } >> "$sheet"
    chmod 600 "$sheet"
    echo "    written (mode 600). Print it, hand each person their line, then delete it."
fi

say "done. Next: open $url/lead-scout/assignments?event=$event_key to see the grid."
