# Setup guide: from a box of parts to an event-ready scouting server

*For a person who is not a developer. Each part is a checklist with a "done when" line. Event-day wiring is `EVENT_DAY_RUNBOOK.md`; the details behind this are `PI_STORAGE.md` and `PI_NETWORK.md`.*

> **Untested on real hardware.** `deploy/pi/bootstrap.sh` has been run with `--dry-run` only, and `deploy/event/prep-event.sh` against a server on a laptop. Nobody has run bootstrap on a Pi yet. Do Parts 1 to 3 once at the shop, fix this page where it is wrong, and only then trust it at an event.

**Who does what.** The Pi serves everything. **Chromebooks** (wired, one per scout and lead) do the scouting, the lead scout's pages, schedule import and admin, in the browser: they run no scripts. **Phones** only view data and sync offline. A Linux/Mac/Windows laptop is optional.

**Rules that never change:** no Wi-Fi, hotspot, Bluetooth or any radio for event data (wired switch only, E143); plain `http://`, never `https://`; `TEALTEAM_ENV=prod`; the Pi has **no internet at the event**.

## Part 1. Build machine

Any Linux or macOS machine with internet, or the Pi itself (slow, about 20 minutes, but it is the way the runbook has always worked).

1. Install Rust with `rustup` (the repo's `rust-toolchain.toml` picks the version), `git`, and a C compiler (`build-essential` / Xcode tools).
2. Get the code and the release tag: `git clone <repo> && cd TealTeam && git checkout <tag>`.
3. **Build on the Pi** (simplest): copy the folder to the Pi and run `deploy/build-release.sh` there as your normal user, never root. It builds the offline client (the first run installs `wasm-bindgen`, which needs internet), then `target/release/tt-web`.
4. **Or cross-compile** for the Pi from another machine. UNTESTED: `deploy/build-release.sh` builds for the machine it runs on. Install `rustup target add aarch64-unknown-linux-gnu` and an aarch64 gcc (`gcc-aarch64-linux-gnu` on Debian/Ubuntu), then:
   ```sh
   deploy/build-client.sh
   CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
     cargo build --release --locked -p tt-web --target aarch64-unknown-linux-gnu
   ```
   The binary is `target/aarch64-unknown-linux-gnu/release/tt-web`. (TLS is rustls, so no OpenSSL is needed; SQLite is built from C.)
5. Copy the binary and the repo's `deploy/` folder to the Pi (`scp`).

**Done when:** `file tt-web` says `ARM aarch64`, and `./tt-web help` on the Pi prints the commands.

## Part 2. The Pi

Use a Raspberry Pi 5 with an NVMe or USB SSD and the official 27 W supply.

1. **Flash** *Raspberry Pi OS Lite (64-bit), Bookworm* with Raspberry Pi Imager. In its settings: hostname `tealteam`, a user (not `pi`; any name you will remember), SSH on, and **the shop's Wi-Fi** so the Pi has internet for the next steps. (Wi-Fi is for the shop only; the event network is wired.)
2. **SSD:** follow `PI_STORAGE.md` steps 1 to 4, ending with `findmnt /srv/tealteam` showing the SSD. Bootstrap refuses to go on if it does not, and never partitions a disk itself.
3. **Log in over Wi-Fi, not over the Ethernet cable** (bootstrap moves the cable to the event LAN and refuses an SSH session on it): `ssh <user>@tealteam.local`.
4. Put the binary and the repo's `deploy/` folder on the Pi, then:
   ```sh
   sudo deploy/pi/bootstrap.sh deploy/profiles/frc-standard.env ./tt-web
   ```
   Try `--dry-run` first: it says everything it would change. It checks the Pi, installs the few packages needed, checks the SSD, runs `network/setup.sh` and `service/install.sh`, writes `/opt/tealteam/.env` from the profile and asks you for the keys (typed with no echo), restarts the server, waits for it, takes one backup and restores it to check it, and prints `status.sh`. Run again any time: it keeps what is already in the env file.
5. **Load the event while there is internet** (the Pi has none at the venue): `cd /opt/tealteam && sudo -u tealteam ./tt-web bulk-load`. It pulls the events, teams, schedule and rankings the keys allow and prints counts per event: check your event is in the list with the matches you expect. Do it again the night before if the schedule might change.
6. **Power cuts.** The database commits to the SSD before a scout is told "saved", so pulling the plug loses nothing that was confirmed. The server restarts by itself at boot. After a cut: wait a minute, open the page, run `deploy/pi/network/status.sh`.
7. **Upgrading** to a new tag: take a backup first (`sudo -u tealteam /opt/tealteam/tt-web backup /srv/tealteam/backups`), build the new binary (Part 1), run `sudo deploy/pi/service/install.sh ./tt-web` (or bootstrap again). It keeps the env file and data. Never upgrade during an event.

**Done when:** `deploy/pi/network/status.sh` is all `ok` (an `eth0` FAIL with no switch cabled yet is expected), and `curl http://localhost/health` says `"storage":"ready"`.

## Part 3. Production environment

The env file is `/opt/tealteam/.env`, owner `tealteam`, **mode 600**. It holds API keys: **never commit it, paste it into chat, or copy it to a laptop.** Edit with `sudo -e /opt/tealteam/.env`, then `sudo systemctl restart tealteam`. The profile `deploy/profiles/frc-standard.env` holds no secrets: an empty value means "bootstrap asks".

| Variable | Value | Meaning |
| --- | --- | --- |
| `TEALTEAM_ENV` | `prod` | Required. Anything else may let a restart reset the schema |
| `PORT` | `8080` | Port 80 is redirected to it (`port80.nft`); change both or neither |
| `DATABASE_URL` | `sqlite:///srv/tealteam/data/tealteam.db` | On the SSD, three slashes |
| `BACKUP_DIR` | `/srv/tealteam/backups` | Snapshot every 10 minutes, a day kept (144) |
| `BACKUP_COPY_TO` | unset | Default folder for `tt-web backup` (the USB stick) |
| `FIRST_API_USERNAME`, `FIRST_API_KEY` | secret | FIRST Events API, from frc-events.firstinspires.org/services/API |
| `TBA_AUTH_KEY` | secret | The Blue Alliance, from thebluealliance.com/account. Empty turns the TBA sync off |
| `FIRST_SEASON` | `2026` | |
| `FIRST_EVENT_CODE` | e.g. `TXHOU` | Keeps the sync to your event |
| `FIRST_TEAM_NUMBER` | `10101` | |
| `FIRST_COUNTRY` | unset | Only used with no event code and no team number |
| `FIRST_SYNC_ON_BOOT` | `true` | Pull the event list at start; harmless with no internet |
| `TETHER_INTERFACES` | `usb0,eth1` | A USB-tethered phone syncs when plugged in; off-site only |

- **Backups, three layers:** every 10 minutes onto the SSD (automatic); between match blocks onto a USB stick, `sudo -u tealteam /opt/tealteam/tt-web backup /media/$USER/STICK` (it restores the copy and counts what is in it); and the stick leaves the building.
- **Restore drill** (do it once at the shop): `sudo -u tealteam /opt/tealteam/tt-web check-backup` restores the newest backup into a scratch database and says what is in it. To really restore: `sudo systemctl stop tealteam`, copy the chosen `tealteam-….db` over `/srv/tealteam/data/tealteam.db` (delete its `-wal` and `-shm` files), owner `tealteam`, `sudo systemctl start tealteam`.
- **Logs:** `journalctl -u tealteam -f`; `-n 100` for the last lines. The startup lines say where the database is and whether it is on the SD card (it must not be).
- **Between events:** back up to the stick first, then `sudo systemctl stop tealteam`, move `/srv/tealteam/data/tealteam.db*` aside (keep it), `sudo systemctl start tealteam`: a fresh database. The next first sign-up is the admin. Never reset with a non-`prod` `TEALTEAM_ENV`.

**Done when:** `sudo ls -l /opt/tealteam/.env` shows `-rw-------`, and a restore drill ran once.

## Part 4. Chromebooks (the scouts and the lead)

One checklist per Chromebook. **Print `deploy/laptop/CHROMEBOOK_CARD.md` as one page per Chromebook** (the same steps, in big type). ChromeOS runs no shell scripts and has no hosts file, so nothing here is scripted.

1. Plug a **USB-C Ethernet adapter** into the Chromebook and the flat cable into the switch. Wait for the network icon to say wired.
2. **Turn Wi-Fi off:** quick settings (bottom right), click Wi-Fi, switch it off.
3. Open **http://tealteam.local/**. If it does not load, try **http://tealteam/**, then **http://10.101.0.1/** (the Pi's fixed address; it always works).
4. **Trust the server once** (HTTPS, [HTTPS_SETUP.md](HTTPS_SETUP.md)): open **http://tealteam.local/ca.crt** (it downloads), then `chrome://certificate-manager`, *Local certificates*, *Import*, pick **tealteam-ca.crt**, tick *Trust this certificate for identifying websites*. Open **https://tealteam.local/**: no warning, a padlock. Skipped, `http://` still works, without offline reloads.
   **Bookmark** the https address (Ctrl+D) and **pin it to the shelf**: three-dot menu, Cast, save and share, *Create shortcut*, tick *Open as window*.
5. **Sign in** with the name and temporary password from the sign-in sheet (Part 6); change the password on the Account page.
6. **Clear old site data from earlier events:** open `chrome://settings/content/all`, search `tealteam`, and delete the entries.
7. **Keep the screen on:** Settings, Device, Power, set *While charging* to *Keep display on*. Keep the Chromebook plugged in.
8. Open **Scout**: the page downloads its offline copy once; leave it a moment. With the padlock (https), scouting then carries on with no server: each save is held on the Chromebook and sent when the server is back (OF1, [HTTPS_SETUP.md](HTTPS_SETUP.md)). Without it, a scout whose server drops stays on the page and switches to paper.

**Done when:** the Chromebook shows your name at the top of https://tealteam.local/ with no certificate warning.

**Name lookup.** The Pi's dnsmasq (`dnsmasq-tealteam.conf`) is the DNS server the Chromebooks are given by DHCP, and it answers `tealteam.local` and `tealteam` itself, so they do not depend on mDNS. No network change was needed. The fallback is always the fixed IP.

### Optional: a Linux, Mac or Windows laptop (the lead's or admin's)

`deploy/laptop/setup-laptop.sh` (Linux/macOS) or, in an Administrator PowerShell, `deploy/laptop/setup-laptop.ps1` (Windows). Both check the wired link, turn Wi-Fi off, check the server answers, put a shortcut on the desktop, and keep the laptop awake on AC; `--dry-run` shows what they would do and `--undo` / `-Undo` puts it back. Windows without Bonjour gets a hosts line for the Pi. You still sign in and clear old site data by hand in the browser (the script prints that list). Needs no internet.

## Part 5. Phones

Phones only **view** data and sync offline. They stay off the event LAN unless wired.

1. Away from the venue, with internet, open the server's address once and sign in.
2. Let the page finish: it downloads an offline copy. Do this the night before.
3. At the venue the phone shows its copy (older data is marked). It syncs when it reaches the server again.

**Done when:** with airplane mode on, the phone still opens the pages.

## Part 6. Event prep

On the Pi, over SSH, in the repo folder (Chromebooks cannot run it; the server's pages do the same things by hand if you prefer):

1. Copy `deploy/event/roster.example.csv` to your own file (name, email, role, team; role is `scout`, `lead`, `coach` or `admin`) and point `PREP_ROSTER` in a copy of the profile at it. Set `PREP_EVENT_CODE` (or `PREP_EVENT_KEY` if TBA's key differs from `2026` + lower-case code).
2. Run it:
   ```sh
   deploy/event/prep-event.sh my-profile.env
   ```
   On a Pi named `tealteam` it uses `http://127.0.0.1:8080`; from a Linux/macOS laptop add `--url http://10.101.0.1`. It asks for the admin's email and password (a fresh server has no admin: it offers to make the first account), then makes an account for everyone in the roster with a random temporary password, sets their roles and team on the Accounts page, asks the server to sync the schedule and checks the event has matches, hands the robots out among the first `PREP_SCOUTS` (6) scouts, and writes `signin-sheet-<event>.txt` (mode 600, ignored by git). Rerun it any time; it only adds what is missing.
3. Print the sign-in sheet, cut it into lines, hand them out, delete the file.
4. Not scripted (no page for it): naming tablets, entering rankings. Then follow `EVENT_DAY_RUNBOOK.md`.

**Done when:** the lead scout's Assignments page says every upcoming robot has a scout.

## Night before

- [ ] Server up; `bulk-load` re-run with internet; counts include your event and its matches
- [ ] Env file keys set, mode 600; `TEALTEAM_ENV=prod`; restore drill done once
- [ ] A backup on the USB stick, and the stick in the bag
- [ ] `prep-event.sh` run; sign-in sheet printed
- [ ] Every Chromebook: charged, adapter, Wi-Fi off, bookmark, signed in, old site data cleared, offline copy downloaded
- [ ] Phones opened once on the server with internet
- [ ] Kit: Pi + SSD, 27 W supply, switch + power, one flat cable and adapter per Chromebook, gaff tape, USB stick, printed runbook, Chromebook cards, sign-in sheet

## At the venue

- [ ] Cables first, then power: switch, then the Pi (runbook "Set up")
- [ ] `deploy/pi/network/status.sh` all `ok`; no uplink is fine
- [ ] One Chromebook opens https://tealteam.local/ with no warning (its CA trusted), else http://10.101.0.1/
- [ ] The CA folder (`/srv/tealteam/data/tls`) is on the backup stick
- [ ] Lead scout signs in, picks the event, checks the assignments page
- [ ] Never plug in a phone tether; no Wi-Fi, hotspot or router
- [ ] Between match blocks: `tt-web backup` to the stick; eject it before pulling
