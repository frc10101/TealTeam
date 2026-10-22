# Setup guide: a fresh Pi and the team's laptops, event-ready

For someone who is not a developer. Plain steps, each part ends with a **Done when** line. Reference: `docs/event/PI_STORAGE.md` (SSD, backups), `docs/event/PI_NETWORK.md` (network), `docs/event/EVENT_DAY_RUNBOOK.md` (the day).

> **Untested on real hardware.** The scripts were checked with `bash -n`, `--dry-run` on a development machine, and (prep-event.sh) a real run against a local server. `bootstrap.sh`, the Chromebook steps and the Pi itself have not been run on the team's gear. Do the whole guide once at the shop and fix this page where it is wrong.

**Rules that never bend:** wired Ethernet and one switch only, no Wi-Fi, hotspot or Bluetooth for event data; plain `http://` (no HTTPS) on the event LAN; `TEALTEAM_ENV=prod`; the Pi has no internet at the venue, so everything is installed at the shop.

---

## FTC Athens quick start

Athens Qualifier `USALATQ`, Sat 2026-10-24. **1 Pi, 4 scout Chromebooks, 1 lead Chromebook, phones for viewing.** Profile: `deploy/profiles/ftc-qual-4robot.env` (4 robots a match, FTC Events pull optional, scouting assistant off). Chromebooks cannot run shell scripts, so everything that is a command runs **on the Pi**; each Chromebook only follows the card at the end of this page.

**At the shop, with internet (once):**

```sh
# On the Pi (SSH in as the pi user), after the SSD is mounted at /srv/tealteam (Part 2):
git clone <the team repo> TealTeam && cd TealTeam
git checkout <final tag>                   # the retagged build you will run (the beta-1 tag predates this kit)
deploy/build-release.sh                    # 20-40 min; builds on the Pi
sudo deploy/pi/bootstrap.sh deploy/profiles/ftc-qual-4robot.env target/release/tt-web
#   asks for FTC_API_USERNAME / FTC_API_KEY: press Enter twice to skip (CSV is the plan of record)
```

**Rehearsal (Wed night or any time; the schedule is the 12-team fixture):**

```sh
deploy/event/prep-event.sh deploy/profiles/ftc-qual-4robot.env --rehearsal
#   asks the lead's password (a new database makes that first account the admin).
#   Writes signin-sheet-*.txt in this folder: temporary passwords. Never commit it.
```

**Event day, when the official schedule is posted** (save it as `athens.csv` with the header `match,red1,red2,blue1,blue2`; a `*` after a team marks a surrogate). Over SSH on the Pi:

```sh
cd TealTeam
cp deploy/event/roster.example.csv roster.csv      # edit: real names, one admin (the lead), 4 scouts, teams
deploy/event/prep-event.sh deploy/profiles/ftc-qual-4robot.env --roster roster.csv --csv athens.csv
cat signin-sheet-*.txt                              # print it, hand out, shred after the event
```

With both FTC Events keys set on the Pi, drop `--csv`: the script pulls the schedule instead. If the schedule changes, run the same command again: it keeps observations. With 2 or 3 scouts on duty the script says some robots stay open on purpose.

**Each Chromebook (scout and lead):** the card at the end of this page, one printed copy each. **Phones:** open `http://tealteam.local/` once on the wired network if you can, or just look at the read-only pages; they keep an offline copy and sync later (Part 5).

Then the runbook: `docs/event/EVENT_DAY_RUNBOOK.md`.

---

## Part 1. Build machine

The release binary must carry the offline client, and `install.sh` refuses one that does not. `deploy/build-release.sh` builds on the machine it runs on: **it does not cross-compile.** So the simplest build machine is the Pi itself, with internet at the shop.

1. Install Rust with `rustup` (the repo's `rust-toolchain.toml` picks the version and the `wasm32-unknown-unknown` target) and a C toolchain: `sudo apt install build-essential pkg-config git`.
2. `git clone` the repo, `git checkout` the tag to run, then `deploy/build-release.sh`. The first run also installs `wasm-bindgen`'s CLI (internet needed).
3. The binary is `target/release/tt-web`. To build on a laptop instead you need an aarch64 cross toolchain; this guide does not cover that.

**Done when:** `grep -c client/tt_client_bg.wasm target/release/tt-web` prints 1 or more.

## Part 2. The Pi

1. Flash **Raspberry Pi OS Lite, 64-bit (Bookworm)** with Raspberry Pi Imager. In its settings: hostname `tealteam`, a user (the guide says `pi`), a password, SSH on, **no Wi-Fi**. Wi-Fi may be used at the shop only for the build; turn it off before the event (`sudo nmcli radio wifi off`).
2. Attach the SSD (a USB 3 SSD in a blue port; the official 27 W supply). Partition, format and mount it at `/srv/tealteam` by UUID: `docs/event/PI_STORAGE.md` steps 1-4. This is the one step the scripts will not do for you, because it erases a disk.
3. Run `sudo deploy/pi/bootstrap.sh PROFILE target/release/tt-web --dry-run` first, then without `--dry-run`. It checks the OS and SSD, runs `deploy/pi/network/setup.sh`, writes `/opt/tealteam/.env` from the profile, runs `deploy/pi/service/install.sh`, waits for `/health`, takes one backup and restores it to check, and prints `status.sh`.
4. Verify: `deploy/pi/network/status.sh` shows no `FAIL` (`no uplink` is fine), and `systemctl status tealteam` says `active (running)`.
5. **Backups.** Every 10 minutes the server snapshots to `/srv/tealteam/backups` and keeps 24 hours. Between match blocks copy to a USB stick: `sudo /opt/tealteam/tt-web backup /media/$USER/STICK` (the runbook has the full line). Restore drill: `tt-web check-backup`, and `docs/event/PI_STORAGE.md` "Putting a backup back".
6. **Power cut.** Every save is flushed to the SSD before the scout sees "saved", so pulling the plug loses nothing that was confirmed. The service restarts at boot. Waiting for the green light, then `status.sh`, is the whole recovery.
7. **Upgrading to a new tag:** take a backup first (step 5), `git fetch --tags && git checkout <tag>`, `deploy/build-release.sh`, `sudo deploy/pi/service/install.sh target/release/tt-web` (or run `bootstrap.sh` again: it changes only what differs), then `status.sh`.

**Done when:** `curl -s http://localhost/health` returns `"storage":"ready"` and `status.sh` has no `FAIL`.

## Part 3. Production environment

The Pi's settings live in **`/opt/tealteam/.env`**, mode 600, owned by `tealteam`. `bootstrap.sh` writes it from the example and the profile and keeps what is already there on a rerun. Keys are never committed: the profile lists them empty, and bootstrap asks for them without echo.

| Variable | Value / meaning |
| --- | --- |
| `TEALTEAM_ENV` | `prod`. Anything else (`dev`) allows schema resets; never on the Pi |
| `PORT` | `8080`; port 80 is redirected to it (`port80.nft`). If changed, change `port80.nft` too |
| `DATABASE_URL` | `sqlite:///srv/tealteam/data/tealteam.db` (three slashes, on the SSD) |
| `BACKUP_DIR` | `/srv/tealteam/backups`, the 10-minute snapshots |
| `BACKUP_COPY_TO` | optional; where a bare `tt-web backup` copies (a mounted stick) |
| `FTC_API_USERNAME`, `FTC_API_KEY`, `FTC_EVENT_CODE` | optional FTC Events pull; both keys or neither; code `USALATQ` |
| `ASK_BACKEND` and `ASK_*` | the scouting assistant; **off for Athens**, leave unset |
| `FIRST_API_*`, `TBA_AUTH_KEY` | not used on the FTC build |
| `TETHER_INTERFACES` | the phone-tether watcher; off on the FTC build |
| `FIRST_SYNC_ON_BOOT` | upstream fetch at boot; off on the FTC build |

Logs: `journalctl -u tealteam -n 100` (or `-f`). **Between events:** take a backup, then either keep the data and import the new schedule (it replaces matches, keeps observations) or start clean: `sudo systemctl stop tealteam`, move the `tealteam.db*` files in `/srv/tealteam/data` into a dated folder, `sudo systemctl start tealteam`; the first account created afterwards is the admin.

**Done when:** `sudo ls -l /opt/tealteam/.env` shows `-rw-------` and `tealteam`, and `tt-web check-backup` says the database is whole.

## Part 4. Laptops

**The team's laptops are Lenovo Chromebooks.** They do the scouting, the lead scout's pages and admin. Follow the **Chromebook card** below, one per laptop; it needs no scripts and no internet. They cannot edit a hosts file, so the name must resolve through the Pi: its dnsmasq already answers `tealteam.local` and `tealteam` over DNS and hands the Pi out as the DNS server, so a Chromebook does not depend on mDNS. If a name does not open, use the fixed address **http://10.101.0.1/**. (Not yet checked on a Chromebook: whether Chrome sends `.local` only to mDNS; that is why the IP is on the card.)

A Linux, Mac or Windows laptop, if you have one, can run `deploy/laptop/setup-laptop.sh` (Linux/macOS) or `setup-laptop.ps1` (Windows, as administrator). They check the wired link, turn Wi-Fi off, find the server (adding a hosts entry on Windows if needed), print its build, put a shortcut on the desktop and stop sleep on AC. `--undo` reverses it. They cannot sign in or clear site data for you. Use `--dry-run` first.

**Done when:** each laptop shows the sign-in page at `http://tealteam.local/` or `http://10.101.0.1/`.

**No Pi (a demo):** Maxwell's Omen laptop (Omarchy) can be the server instead: `deploy/laptop/omenMaxwell-setup.sh` copies the release binary to `~/tealteam-demo` with a prod `.env`, opens port 8080 in ufw (asks for sudo), and starts it so the lid and idle do not suspend the laptop. `stop` and `status` do what they say; rerunning keeps the database. The lead scout uses `http://localhost:8080/` on that laptop, the Chromebooks the `http://IP:8080/` it prints. It hands out no addresses and answers no `tealteam.local`, so the network needs its own router.

## Part 5. Phones

Phones only **view** data and sync offline; scouting and the lead's work happen on the laptops. Phones stay off the event Wi-Fi/radio network (there is none). Before the event, on any network that reaches the Pi, open the address once and let the page finish loading: it downloads an offline copy. At the event a phone shows its last copy and updates when it can reach the server again. Expect read-only pages (team, rankings, graph, drive coach, pick list) and no scouting form.

**Done when:** with the phone's network off, the home page still opens.

## Part 6. Event prep

On the Pi, `deploy/event/prep-event.sh PROFILE` (the quick start above). It signs in as the admin, creates accounts from the roster (a name, an email or username, a role of scout, lead, coach or admin, a team number), sets roles on the Accounts page, loads the schedule (CSV, or the FTC pull if the keys are set), checks every match has 4 robots, auto-assigns, and writes the sign-in sheet. A username without `@` becomes `username@tealteam.local`. A rehearsal schedule is refused without `--rehearsal`.

**Stays manual:** each person changes their password at `/account`; names missing from the roster show "not on roster"; re-run auto-assign when the number of scouts changes; check the schedule against the printed sheet; the first Chromebook sign-in.

**Done when:** the assignments page says "N of N robots in upcoming matches have a scout" (with 4 scouts).

---

*Printable copies of these checklists and the Chromebook card (two to a page): `docs/event/print/event-checklists.pdf` and `docs/event/print/chromebook-card.pdf`. For using the laptops once they are set up: `scout-card.pdf` (one per scout laptop) and `lead-card.pdf` (the lead's), both below, with .docx versions to edit. Rebuild with `python3 docs/event/print/build.py`.*

## Night before

- [ ] Pi built from the final tag; `status.sh` clean; `check-backup` ok; backup on the stick.
- [ ] Switch, Pi supply, one cable per Chromebook, USB-C Ethernet adapters, gaff tape, USB stick, printed runbook, printed cards and paper kit.
- [ ] Chromebooks charged and wiped (old site data cleared), cards printed, sign-in sheet printed.
- [ ] Phones charged; offline copy loaded.
- [ ] Re-read the Team Update for scoring changes.

## At the venue

- [ ] Cable before power; switch first, then the Pi; wait for the green light.
- [ ] `deploy/pi/network/status.sh` has no `FAIL`.
- [ ] One Chromebook opens `http://tealteam.local/`; then all of them, per the card.
- [ ] Schedule posted: run `prep-event.sh` with the real CSV (or the pull); compare with the printed sheet.
- [ ] One scout, one match, check the result, then the full crew.
- [ ] Backup to the stick after every qualification round.

---

## Chromebook card (print one per laptop)

**TealTeam on a Chromebook.** Do these in order.

1. Plug the USB-C Ethernet adapter into the Chromebook and the cable into the switch.
2. Turn **Wi-Fi off**: click the clock, click the Wi-Fi tile so it is grey. No hotspot, no Bluetooth tether.
3. Open Chrome and go to **http://tealteam.local/**. If it does not open, try **http://tealteam/**, then **http://10.101.0.1/**.
4. **Trust the server once** (HTTPS, `docs/event/HTTPS_SETUP.md`): open **http://tealteam.local/ca.crt** (it downloads). Go to `chrome://certificate-manager`, **Local certificates**, **Import**, pick **tealteam-ca.crt**, and tick **Trust this certificate for identifying websites**. Then open **https://tealteam.local/**: no warning, a padlock.
5. Press Ctrl+D to bookmark the **https** address. Click the three dots, **Cast, save and share**, **Create shortcut**, tick **Open as window** if you like; right-click its icon and **Pin to shelf**.
6. **Clear old data from earlier events:** go to `chrome://settings/content/all`, search `tealteam`, and delete the data for it (also any `10.101.0.1`).
7. **Sign in** with your name and the temporary password from the sign-in sheet, then change it at **Account**.
8. **Keep the screen on:** Settings, **Device**, **Power**, set **While charging** to **Keep display on**. Leave the charger plugged in.
9. Open your scouting page once so the offline copy loads, then wait for your match.

If the page will not open: check the cable and adapter, that the switch has power, and that Wi-Fi is off. Tell the lead scout.

## Scout card: scouting a match (print one per scout laptop)

1. **Sign in** at **https://tealteam.local/** (a certificate warning: use **http://tealteam.local/** and tell the lead) and press **Start scouting**. Keep this tab open all day.
2. The page says **You are scouting** and the team number, with **Your next assignment** at the top. Find that robot on the field before the match starts: red or blue, and its number.
3. **Fill it in while the match runs.** Tap **+** each time it scores. What you type is kept as you go, so a reload loses nothing.
4. When the match ends, finish the notes and press **Save observation**. Then follow **Your next assignment** at the top.
5. **Check the chip in the header:** **Synced** means the server has it. **Offline · 1 held** means it is waiting on this laptop: **do not reload, click a link or close the tab** (the page will not come back until the server does), never clear the browser, tell the lead scout, and switch to paper.

If something is off: wrong robot or no assignment, use **Not your robot? Choose another** or **Not in this match? Team number** and tell the lead scout. **Declined**: read why, press **Record it again**. Missed the robot: tell the lead scout, do not guess. Server out of reach for a while: **Server out of reach?** → **Show the code**, the lead scout scans it. App down: paper sheets, one per robot per match.

## Lead card: lead scout during matches (print one for the lead laptop)

**Before the first match.** **Lead Scout → Assignments** should say **N of N robots in upcoming matches have a scout**; if not, **Hand out open robots**. To change who watches a robot, tap the match, pick the scout, save.

**Every few matches.** **Waiting for review**: open the oldest, **Approve and see the next**, or **Why decline it?** + **Decline** (the scout records it again; **Put back to waiting** undoes a mis-tap). **Refused when sent**: record each against the right match, or dismiss. **Rotation**: swap out a scout who has been on too long. Back up to the stick after every qualification round.

**When things change.** Schedule changed: **Match schedule**, import the new CSV (observations are kept), then on **Assignments** open and save each match it lists as changed, then **Hand out open robots**. Fewer scouts: **Hand out open robots** again. A scout's laptop offline: **Scan a tablet**, then they scan the receipt. App down: paper sheets, typed in afterwards with `PAPER` in *Rule incidents*.
