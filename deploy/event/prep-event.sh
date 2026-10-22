#!/usr/bin/env bash
# Get a running server ready for an event, over HTTP, as its admin (F12):
# accounts from a roster, the schedule, and auto-assigned scouts.
#
#   deploy/event/prep-event.sh PROFILE [--url http://127.0.0.1:8080]   (default on the Pi; else http://tealteam.local)
#       [--roster FILE] [--csv FILE] [--schedule csv|ftc] [--rehearsal]
#       [--sheet FILE] [--dry-run]
#
# Main path: on the Pi itself, over SSH, against http://127.0.0.1:8080 (the
# Chromebooks cannot run shell scripts). It also works from any Linux/Mac
# laptop with --url http://tealteam.local. It does what
# the lead scout would do by clicking, with the same forms:
#   1. sign in as the admin (asks for the password; on a brand-new database the
#      first sign-up is the admin, so it creates that account instead);
#   2. create each roster account (sign-up form), set its role and team on
#      the Accounts page (/admin/users);
#   3. load the schedule: the CSV through /lead-scout/schedule, or "Pull from
#      FTC Events" when the server has both keys; check every match has
#      PREP_ROBOTS_PER_MATCH robots;
#   4. auto-assign the PREP_SCOUTS scouts, then read the coverage back;
#   5. write a sign-in sheet (names and temporary passwords) to a local file.
#
# Safe to rerun: an account that exists keeps its password; an import replaces
# matches and keeps observations; assignments already made are left alone.
# Whatever has no web page is printed as a manual step, never done to the
# database. --dry-run checks the profile, roster and CSV and sends nothing.
#
# Needs bash, curl, awk, sed. Run with the server's plain HTTP address; there
# is no HTTPS on the event LAN. The password can come from PREP_ADMIN_PASSWORD
# for scripted runs; otherwise it is asked for without echo.
# Tested against a local tt-web (scratch database, free port); the FTC pull
# and a real Pi are UNTESTED.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
die() { printf 'prep-event: %s\n' "$*" >&2; exit 1; }
usage() { sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 2; }

profile="" url="" roster_arg="" csv_arg="" source_arg=""
rehearsal=0 dry_run=0 sheet=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --url) url="${2:?--url needs an address}"; shift 2 ;;
        --roster) roster_arg="${2:?--roster needs a file}"; shift 2 ;;
        --csv) csv_arg="${2:?--csv needs a file}"; shift 2 ;;
        --schedule) source_arg="${2:?--schedule needs csv or ftc}"; shift 2 ;;
        --sheet) sheet="${2:?--sheet needs a file}"; shift 2 ;;
        --rehearsal) rehearsal=1; shift ;;
        --dry-run) dry_run=1; shift ;;
        -h | --help) usage ;;
        -*) echo "unknown option $1" >&2; usage ;;
        *) [[ -z "$profile" ]] || usage; profile="$1"; shift ;;
    esac
done
[[ -n "$profile" && -f "$profile" ]] || usage
# On the Pi itself (hostname tealteam) the server is local; elsewhere, its name.
if [[ -z "$url" ]]; then
    if [[ "$(hostname)" == tealteam ]]; then url="http://127.0.0.1:8080"; else url="http://tealteam.local"; fi
fi
url="${url%/}"
case "$url" in http://*) ;; https://*) die "use plain http: the event LAN has no HTTPS" ;; *) die "--url must start with http://" ;; esac
command -v curl >/dev/null || die "curl is not installed"

# A profile value: the last KEY=value line, or the default.
pget() {
    local v
    v="$(sed -n "s/^$1=//p" "$profile" | tail -n 1 | tr -d '\r')"
    printf '%s' "${v:-${2:-}}"
}
absolute() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$ROOT" "$1" ;; esac; }

robots="$(pget PREP_ROBOTS_PER_MATCH 4)"
scouts_want="$(pget PREP_SCOUTS 4)"
scouts_min="$(pget PREP_SCOUTS_MIN 2)"
scouts_max="$(pget PREP_SCOUTS_MAX 4)"
code="$(pget PREP_EVENT_CODE)"
ename="$(pget PREP_EVENT_NAME)"
tz="$(pget PREP_TIMEZONE)"
edate="$(pget PREP_EVENT_DATE)"
season="$(pget PREP_SEASON 2026)"
our_team="$(pget PREP_TEAM_NUMBER)"
admin_email="$(pget PREP_ADMIN_EMAIL)"
roster="$(absolute "${roster_arg:-$(pget PREP_ROSTER)}")"
csv="$(absolute "${csv_arg:-$(pget PREP_SCHEDULE_CSV)}")"
source="${source_arg:-$(pget PREP_SCHEDULE_SOURCE auto)}"

if [[ $rehearsal -eq 1 ]]; then
    code="$(pget PREP_REHEARSAL_CODE REHEARSAL)"
    ename="Rehearsal"
fi
[[ -n "$code" && -n "$ename" && -n "$tz" ]] || die "the profile needs PREP_EVENT_CODE, PREP_EVENT_NAME and PREP_TIMEZONE"
case "$source" in csv | ftc | auto) ;; *) die "--schedule must be csv, ftc or auto" ;; esac
[[ $rehearsal -eq 1 && "$source" == ftc ]] && die "--rehearsal and the FTC pull do not mix: a rehearsal has its own event code"
if [[ $scouts_want -lt $scouts_min || $scouts_want -gt $scouts_max ]]; then
    die "PREP_SCOUTS=$scouts_want is outside $scouts_min-$scouts_max"
fi
event_key="${season}$(printf '%s' "$code" | tr '[:upper:]' '[:lower:]')"

# ── Local checks: the roster and the CSV, before anything is sent ───────────

[[ -f "$roster" ]] || die "no roster at $roster"
roster_rows="$(mktemp)"
trap 'rm -rf "$roster_rows" "${work:-}"' EXIT
tr -d '\r' < "$roster" | awk -F, -v sq="'" '
    NR == 1 { if (tolower($1) != "name" || tolower($4) != "team") { print "the roster header must be name,email,role,team" > "/dev/stderr"; bad = 1 } ; next }
    /^[[:space:]]*$/ { next }
    {
        for (i = 1; i <= 4; i++) { gsub(/^[ \t]+|[ \t]+$/, "", $i) }
        name = $1; email = tolower($2); role = tolower($3); team = $4
        if (NF != 4 || name == "" || email == "") { printf "roster line %d: want name,email,role,team\n", NR > "/dev/stderr"; bad = 1; next }
        if (role !~ /^(scout|lead|coach|admin)$/) { printf "roster line %d: role \"%s\" is not scout, lead, coach or admin\n", NR, role > "/dev/stderr"; bad = 1; next }
        if (team != "" && team !~ /^[0-9]+$/) { printf "roster line %d: team \"%s\" is not a number\n", NR, team > "/dev/stderr"; bad = 1; next }
        if (email !~ /@/) email = email "@tealteam.local"
        if (email !~ /^[^@ ]+@[^@ ]+\.[^@ .]+$/) { printf "roster line %d: \"%s\" is not an email address\n", NR, email > "/dev/stderr"; bad = 1; next }
        if (seen[email]++) { printf "roster line %d: %s is listed twice\n", NR, email > "/dev/stderr"; bad = 1; next }
        print name "\t" email "\t" role "\t" team
    }
    END { exit bad }' > "$roster_rows" || die "fix the roster ($roster) and run again"
n_scouts="$(awk -F'\t' '$3 == "scout"' "$roster_rows" | wc -l | tr -d ' ')"
n_rows="$(wc -l < "$roster_rows" | tr -d ' ')"
[[ $n_rows -gt 0 ]] || die "the roster has no accounts"
if [[ $n_scouts -lt $scouts_min ]]; then
    die "the roster has $n_scouts scouts; this profile needs at least $scouts_min (PREP_SCOUTS=$scouts_want)"
fi
if [[ -z "$admin_email" ]]; then
    admin_email="$(awk -F'\t' '$3 == "admin" { print $2; exit }' "$roster_rows")"
fi
[[ -n "$admin_email" ]] || die "no admin: put a role=admin row in the roster, or set PREP_ADMIN_EMAIL"
admin_email="$(printf '%s' "$admin_email" | tr '[:upper:]' '[:lower:]')"
case "$admin_email" in *@*) ;; *) admin_email="$admin_email@tealteam.local" ;; esac
grep -q "$(printf '\t%s\t' "$admin_email")" "$roster_rows" || die "$admin_email is not in the roster"
[[ -z "$our_team" ]] || [[ "$our_team" =~ ^[0-9]+$ ]] || die "PREP_TEAM_NUMBER must be a number"
pool_n=$(( n_scouts < scouts_want ? n_scouts : scouts_want ))

# Is the schedule coming from the CSV? ftc/auto decide against the server later.
guard_csv() {
    case "$(basename "$csv")" in
        *REHEARSAL* | *rehearsal*)
            [[ $rehearsal -eq 1 ]] || die "$csv is the rehearsal schedule. Run with --rehearsal, or pass the real schedule with --csv FILE" ;;
    esac
}
check_csv() {
    [[ -f "$csv" ]] || die "no schedule at $csv (pass the real one with --csv FILE)"
    guard_csv
    tr -d '\r' < "$csv" | awk -F, -v need="$robots" '
        function bad(msg) { printf "schedule line %d: %s\n", NR, msg > "/dev/stderr"; failed = 1 }
        NR == 1 { if (tolower($0) != "match,red1,red2,blue1,blue2") bad("the header must be match,red1,red2,blue1,blue2"); next }
        /^[[:space:]]*$/ { next }
        {
            rows++
            if (NF != need + 1) { bad("want " need + 1 " columns, found " NF); next }
            delete t; n = 0
            for (i = 2; i <= NF; i++) {
                v = $i; gsub(/[ \t*]/, "", v)
                if (v !~ /^[0-9]+$/) { bad("\"" $i "\" is not a team number"); continue }
                if (v in t) bad("team " v " appears twice"); t[v] = 1; n++
            }
            if (n != need) bad("only " n " robots, want " need)
            if ($1 in m) bad("match " $1 " is listed twice"); m[$1] = 1
        }
        END { if (!rows) { print "the schedule has no matches" > "/dev/stderr"; failed = 1 } ; exit failed }' || die "fix the schedule ($csv) and run again"
}

[[ "$source" == ftc ]] || guard_csv
if [[ $dry_run -eq 1 ]]; then
    say "dry run: nothing is sent"
    note "server:    $url   event: $code ($event_key)   admin: $admin_email"
    note "roster:    $n_rows accounts, $n_scouts scouts, $pool_n handed robots"
    if [[ "$source" != ftc ]]; then
        check_csv
        note "schedule:  $csv (every match has $robots robots)"
    else
        note "schedule:  FTC Events pull"
    fi
    note "would: sign in, create accounts, set roles, load the schedule, auto-assign, write the sign-in sheet"
    exit 0
fi

# ── Talking to the server ───────────────────────────────────────────────────

umask 077
work="$(mktemp -d)"
jar="$work/jar"
http() { curl -sS --max-time 60 "$@"; }
alert() { tr '\n' ' ' | sed -n 's/.*class="alert alert-error"[^>]*>\(.*\)/\1/p' | sed 's/<\/p>.*//; s/<[^>]*>//g; s/  */ /g'; }

say "checking the server at $url"
health="$(http "$url/health" 2>&1)" || die "cannot reach $url ($health). Cable and switch? Is tealteam running?"
case "$health" in *'"storage":"ready"'*) note "up: $health" ;; *) die "the server is up but its storage is not ready: $health" ;; esac

sheet="${sheet:-$PWD/signin-sheet-$event_key-$(date +%Y%m%d-%H%M%S).txt}"
# Never emptied: a rerun adds to a sheet, so earlier passwords are not lost.
[[ -e "$sheet" ]] || : > "$sheet"
chmod 600 "$sheet"
{
    printf '\nTealTeam sign-in sheet: %s (%s), %s\nServer: %s\nTemporary passwords: change yours at /account. Keep this out of git; shred it after the event.\n\n' "$ename" "$code" "$(date '+%Y-%m-%d %H:%M')" "$url"
    printf '%-24s %-34s %-7s %s\n' NAME "SIGN-IN (email)" ROLE PASSWORD
} >> "$sheet"

new_password() {
    local p
    p="$(LC_ALL=C tr -dc 'abcdefghjkmnpqrstuvwxyz23456789' < /dev/urandom | head -c 10 || true)"
    printf '%s' "$p"
}

# signup NAME EMAIL TEAM PASSWORD -> 0 created, 1 exists, 2 refused (message on stdout)
signup() {
    local headers="$work/h" body="$work/b" status
    http -o "$body" -D "$headers" -w '%{http_code}' \
        --data-urlencode "name=$1" --data-urlencode "email=$2" --data-urlencode "team_number=$3" \
        --data-urlencode "password=$4" --data-urlencode "confirm_password=$4" \
        "$url/api/auth/signup" > "$work/code"
    status="$(cat "$work/code")"
    if [[ "$status" == 303 ]]; then return 0; fi
    local msg
    msg="$(alert < "$body")"
    case "$msg" in *"already exists"*) return 1 ;; esac
    printf '%s' "${msg:-HTTP $status}"
    return 2
}

# A refused sign-in is a 303 too, back to /sign-in?failed=1: only a 303
# anywhere else (the home page) is signed in.
sign_in() {
    local answer
    answer="$(http -o /dev/null -c "$jar" -w '%{http_code} %{redirect_url}' \
        --data-urlencode "email=$1" --data-urlencode "password=$2" "$url/api/auth/login")"
    [[ "$answer" == "303 "* && "$answer" != *"/sign-in"* ]]
}

say "signing in as the admin ($admin_email)"
admin_row="$(awk -F'\t' -v e="$admin_email" '$2 == e' "$roster_rows")"
admin_name="$(printf '%s' "$admin_row" | cut -f1)"
admin_team="$(printf '%s' "$admin_row" | cut -f4)"
[[ -n "$admin_team" ]] || admin_team="$our_team"
created_admin=0
if [[ -n "${PREP_ADMIN_PASSWORD:-}" ]]; then
    admin_pw="$PREP_ADMIN_PASSWORD"
elif [[ -t 0 ]]; then
    printf 'Password for %s (if this server is brand new, the one to give this account): ' "$admin_email" >&2
    read -rs admin_pw
    printf '\n' >&2
else
    die "no terminal to ask for the admin password on; set PREP_ADMIN_PASSWORD"
fi
[[ ${#admin_pw} -ge 8 ]] || die "a password is at least 8 characters"
if http "$url/sign-up" | grep -q 'first account on this server'; then
    say "new database: creating $admin_email as the first account (an admin)"
    signup "$admin_name" "$admin_email" "$admin_team" "$admin_pw" >/dev/null || die "could not create the admin account"
    created_admin=1
    printf '%-24s %-34s %-7s %s\n' "$admin_name" "$admin_email" admin "(the password you just typed)" >> "$sheet"
fi
sign_in "$admin_email" "$admin_pw" || die "could not sign in as $admin_email: wrong password, or not an account here"
unset admin_pw
http -o "$work/admin.html" -b "$jar" "$url/admin/users"
grep -q 'id="account-' "$work/admin.html" || die "$admin_email is signed in but is not an admin, so cannot use the Accounts page"
note "signed in"

# ── Accounts ────────────────────────────────────────────────────────────────

say "creating accounts from $(basename "$roster")"
while IFS=$'\t' read -r name email role team; do
    [[ "$email" == "$admin_email" ]] && continue
    pw="$(new_password)"
    rc=0
    msg="$(signup "$name" "$email" "$team" "$pw")" || rc=$?
    case $rc in
        0) note "created  $email"; printf '%-24s %-34s %-7s %s\n' "$name" "$email" "$role" "$pw" >> "$sheet" ;;
        1) note "exists   $email (password unchanged)"; printf '%-24s %-34s %-7s %s\n' "$name" "$email" "$role" "(already had an account: password unchanged)" >> "$sheet" ;;
        *) die "could not create $email: $msg" ;;
    esac
done < "$roster_rows"

# email -> id, from the Accounts page.
account_ids() {
    http -b "$jar" "$url/admin/users" | tr '\n' ' ' | sed 's/<tr>/\n<tr>/g' \
        | sed -nE 's/.*href="#account-([0-9]+)".*data-label="Email">([^<]*)<.*/\1 \2/p'
}
ids="$work/ids"
account_ids > "$ids"

say "setting roles and teams on the Accounts page"
scout_ids=()
while IFS=$'\t' read -r name email role team; do
    id="$(awk -v e="$email" '$2 == e { print $1; exit }' "$ids")"
    [[ -n "$id" ]] || die "$email has no account after sign-up: see the server log (journalctl -u tealteam)"
    args=(--data-urlencode "team_number=$team")
    case "$role" in
        admin) args+=(--data-urlencode is_admin=on --data-urlencode is_lead_scout=on) ;;
        lead) args+=(--data-urlencode is_lead_scout=on) ;;
        coach) args+=(--data-urlencode is_coach=on) ;;
    esac
    location="$(http -o /dev/null -b "$jar" -D - "${args[@]}" "$url/api/admin/users/$id" | tr -d '\r' | sed -n 's/^[Ll]ocation: //p')"
    case "$location" in
        *done=saved*) note "$role  $email (team ${team:-none})" ;;
        *done=last-admin*) note "kept    $email as the only admin" ;;
        *) die "the Accounts page refused $email: $location" ;;
    esac
    if [[ "$role" == scout && ${#scout_ids[@]} -lt $pool_n ]]; then scout_ids+=("$id"); fi
done < "$roster_rows"

# ── Schedule ────────────────────────────────────────────────────────────────

say "loading the schedule for $code"
use="$source"
if [[ "$use" == auto ]]; then
    if http -b "$jar" "$url/lead-scout/schedule" | grep -q 'Pull from FTC Events'; then use=ftc; else use=csv; fi
    note "the server $( [[ $use == ftc ]] && echo 'has FTC Events keys: pulling' || echo 'has no FTC Events keys: using the CSV' )"
fi
if [[ "$use" == ftc ]]; then
    http -o "$work/sched.html" -b "$jar" -X POST "$url/api/ftc/pull"
    grep -q 'Pull from FTC Events' "$work/sched.html" || die "the server has no FTC Events keys: set FTC_API_USERNAME and FTC_API_KEY in /opt/tealteam/.env, or use --schedule csv"
    err="$(alert < "$work/sched.html")"
    [[ -z "$err" ]] || die "the FTC pull failed: $err (the plan of record is the CSV: --schedule csv --csv FILE)"
    event_key="${season}$(printf '%s' "$(pget FTC_EVENT_CODE "$code")" | tr '[:upper:]' '[:lower:]')"
else
    check_csv
    http -o "$work/sched.html" -b "$jar" \
        --data-urlencode "code=$code" --data-urlencode "name=$ename" --data-urlencode "timezone=$tz" \
        --data-urlencode "date=$edate" --data-urlencode "schedule@$csv" \
        "$url/api/schedule/import"
    err="$(tr '\n' ' ' < "$work/sched.html" | grep -o 'class="alert alert-error"[^>]*>.\{0,1500\}' | head -n 1 | sed 's/<\/div>.*//; s/<li>/\n  - /g; s/<[^>]*>//g' || true)"
    if [[ -n "$err" ]]; then printf 'prep-event: the server refused the schedule:\n%s\n' "$err" >&2; exit 1; fi
    grep -q 'alert-success' "$work/sched.html" || die "the schedule import answered with neither a report nor an error; open $url/lead-scout/schedule"
    tr '\n' ' ' < "$work/sched.html" | grep -o 'alert alert-success".*' | head -n 1 | sed 's#</div>.*##; s#^[^>]*>##; s#</ul>#\n#g; s#<li>#\n- #g; s#<[^>]*>##g; s/&#39;/'"'"'/g; s/   */ /g' | sed 's/^/    /'
fi

grid="$work/grid.html"
http -b "$jar" "$url/lead-scout/assignments?event=$event_key" > "$grid"
rows="$(grep -c '<tr role="row" id=' "$grid" || true)"
slots="$(grep -c 'class="slot-team"' "$grid" || true)"
tbd="$(grep -c 'class="slot-team">TBD' "$grid" || true)"
[[ $rows -gt 0 ]] || die "no matches stored for $event_key: open $url/lead-scout/schedule"
if [[ $tbd -gt 0 || $(( slots % robots )) -ne 0 || $(( slots / rows )) -ne $robots ]]; then
    die "not every match has $robots robots ($tbd empty slots among $rows matches): fix the schedule and import again"
fi
note "$rows matches, $robots robots each, no empty slots"

# ── Auto-assign ─────────────────────────────────────────────────────────────

say "auto-assigning ${#scout_ids[@]} scouts"
[[ ${#scout_ids[@]} -ge 1 ]] || die "no scouts to assign"
args=()
for id in "${scout_ids[@]}"; do args+=(--data-urlencode "assignee=u:$id"); done
http -o /dev/null -b "$jar" -L "${args[@]}" "$url/api/assignments/auto?event=$event_key"
http -b "$jar" "$url/lead-scout/assignments?event=$event_key" > "$grid"
cover="$(tr '\n' ' ' < "$grid" | sed -nE 's/.*<strong>([0-9]+) of ([0-9]+)<\/strong> robots in upcoming matches have a scout.*/\1 \2/p')"
[[ -n "$cover" ]] || die "could not read the coverage line from $url/lead-scout/assignments?event=$event_key"
covered="${cover% *}"
total="${cover#* }"
note "$covered of $total robots in upcoming matches have a scout"
if [[ ${#scout_ids[@]} -ge $robots ]]; then
    [[ "$covered" == "$total" ]] || die "$covered of $total robots are covered with ${#scout_ids[@]} scouts: open $url/lead-scout/assignments"
else
    note "with ${#scout_ids[@]} scouts some robots stay open on purpose; the Teams table on the assignments page shows who is below 2 observations (F9)"
fi

# ── Manual steps ────────────────────────────────────────────────────────────

say "done. Sign-in sheet: $sheet"
cat <<EOF
    Hand out the sheet, then delete it after the event. It is git-ignored; do not commit it.
    Still by hand (no web page does these):
      - each person signs in on their own laptop, and changes the password at $url/account
      - names that are not on the team roster show "not on roster": add them on the schedule page
      - re-run auto-assign (this script, or the assignments page) when the number of scouts changes
      - check the schedule against the printed sheet, and the surrogates in the report above
EOF
[[ $created_admin -eq 0 ]] || note "the admin account was created by this run"
