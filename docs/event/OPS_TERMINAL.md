# The operator's terminal: `tt-ops`

One screen, on the machine that serves, for the person minding the server at an event: is it up, is the network the event's, who is signed in, what has come in, what the log says, and the few controls the day needs. It runs in a terminal (over SSH on the Pi, or in a window on the laptop that serves), not in a browser: nothing new listens on the LAN, the server binary carries no terminal code, and it keeps working while the server is down, which is when it is needed.

```
sudo /opt/tealteam/tt-ops                      # the Pi, over SSH
target/release/tt-ops --dir ~/tealteam-demo    # the Omen laptop (deploy/laptop/omenMaxwell-setup.sh)
tt-ops --once                                  # one plain-text report, exit 1 if the server is down
```

It finds the install by its `.env`: `--dir`, else `/opt/tealteam`, else `~/tealteam-demo`, else the current folder. From the file it takes `DATABASE_URL`, `PORT`, `TLS_PORT` and `BACKUP_DIR`, as tt-web does. It tells the Pi from the laptop by whether systemd knows a `tealteam` unit; otherwise it looks for the laptop script's `server.pid` and `server.log` beside the binary.

## What it shows

| Tab | What | From |
| --- | --- | --- |
| **1 Overview** | Process state, pid, uptime, memory, restarts; `/health` (storage, build, schema, form); HTTPS listening; the network checks `deploy/pi/network/status.sh` makes (10.101.0.1 on the wired port, the uplink, mDNS, the ports); database file and size, disk free, events, forms, who is online, backups and the newest one's age; the newest sign-ins and forms; the last warnings and errors | systemd or the pidfile, the kernel, the server's `/health`, the database file, the log |
| **2 Logs** | The server's log as it is written, newest at the bottom. `l` cycles the level (info, warn, error), `s` the subject (sign-ins and accounts, scouting, sync, backups, requests, server), `f` follows or pauses, `j`/`k` scroll | `journalctl -u tealteam -f` on the Pi (with `sudo -n` if the user cannot read the journal), `tail -F server.log` on the laptop |
| **3 People** | Every account with roles, team, last sign-in, open sessions, and whether its page has checked in within the last 3 minutes; every device with its team, last heartbeat, who was signed in, and its clock offset. `d` switches between the two tables | The database |
| **4 Data** | Every event with matches, teams, forms (and how many wait for review) and assignments, with the pages' default event starred; every table's row count; the backup folder's files | The database, the backup folder |
| **5 Actions** | The list below, with what each does | |

The screen re-reads the database and the machine every 3 seconds; `r` asks at once. It opens the same SQLite file the server has open (WAL lets a reader in beside the writer) and never writes to it.

## What it does

| Action | How | Needs |
| --- | --- | --- |
| Restart, stop, start the server | Pi: `sudo -n systemctl restart tealteam` (stop, start). Laptop: SIGTERM to the pid in `server.pid`, then the laptop script's own start line (`systemd-inhibit … tt-web serve`, appending to `server.log`) | `sudo` without a password on the Pi; the default `pi` user has it |
| Back up now | `tt-web backup FOLDER`, run as the database's owner (`sudo -n -u tealteam` on the Pi) from the install folder, so it reads the same `.env`. The folder defaults to `BACKUP_DIR`; name the stick's folder to take a copy out of the building. Wait for "It is on the disk." | As above |
| Check the newest backup restores | `tt-web check-backup [FILE]`, the restore test | As above |
| Sync now / Pull from FTC Events | `POST /api/frc/sync` and `POST /api/ftc/pull` over localhost, signed in as an admin | An admin account; internet, or the FTC keys on the server |
| Import a schedule CSV | The lead scout's import form (`POST /api/schedule/import`) with a CSV from this machine; the report or every bad row comes back as the page would show it | An admin account |
| Edit an account's roles and team (People tab, Enter) | The Accounts page's form (`POST /api/admin/users/{id}`); the last admin cannot be demoted | An admin account |
| Reset a password (People tab, `p`) | `POST /api/admin/users/{id}/password`; that account is signed out everywhere | An admin account |
| Power off | `sudo -n systemctl poweroff`, after typing the word. Pi only | `sudo` without a password |

Every change to the data goes through the running server's own forms, never around them: the same checks, the same change log the devices follow, and a line in the server's log naming the admin who did it. The first such action asks for an admin's email and password; the server checks it, and the tool keeps only the session cookie, until it exits.

Restart and poweroff use `sudo -n`, which never asks for a password: it either works at once or fails, and the result screen then shows the exact line to run in a shell. The tool never asks for a sudo password itself.

### Root and the database

On the Pi the operator runs `tt-ops` with `sudo`, because `/srv/tealteam/data` is the `tealteam` user's alone. A root process beside a SQLite database another user owns can leave root-owned `-wal` and `-shm` files when it was the only connection (the server stopped), and the server then cannot open its own database. The tool gives those files back to the database's owner after each read, before it starts the server, and on exit; `tt-web backup` is run as that user, never as root.

## Keys

`1`–`5` pick a tab (`Tab` and `Shift-Tab` too), `j`/`k` or the arrows move, `Enter` acts, `Esc` cancels, `q` quits. The bottom line lists the tab's own keys. In a form, `Tab` moves between fields, `Ctrl-U` clears one, `Enter` submits. A result stays on screen until `Esc` or `Enter`.

## Limits

- Linux only (it reads `/proc`, `ip`, `systemctl`). The Chromebooks cannot run it; the lead's browser pages are theirs.
- It shows the sign-ins and forms the database records, not the server's own memory: the uplink's state, a sync in progress, or the tether watch are in the log and on the lead's page, not here.
- The log view starts from the last 600 lines and holds 3000; the journal keeps the rest (`journalctl -u tealteam`).
- `--once` is the overview only; the actions need the screen.

## Trying it without a Pi

A laptop-style install in any folder is enough:

```sh
cargo build -p tt-web -p tt-ops
d=/tmp/tt-install; mkdir -p $d/data $d/backups; cp target/debug/tt-web $d/
printf 'TEALTEAM_ENV=prod\nPORT=18080\nTLS_PORT=off\nDATABASE_URL=sqlite://%s/data/tealteam.db\nBACKUP_DIR=%s/backups\n' $d $d > $d/.env
(cd $d && nohup ./tt-web serve >> server.log 2>&1 & echo $! > server.pid)
target/debug/tt-ops --dir $d
```

Sign up the first account in a browser at `http://127.0.0.1:18080/sign-up` (the first account is the admin), and the actions that need one work. Two tests run against such an install when asked for, and are ignored otherwise:

```sh
TT_OPS_LIVE_DIR=$d cargo test -p tt-ops live_restart -- --ignored --nocapture
TT_OPS_LIVE_URL=http://127.0.0.1:18080 TT_OPS_LIVE_EMAIL=you@example.com TT_OPS_LIVE_PASSWORD=yourpass \
  TT_OPS_LIVE_ACCOUNT=1 cargo test -p tt-ops live_admin -- --ignored --nocapture
```

The first restarts, stops and starts the install. The second signs in, saves the account's roles, imports the rehearsal schedule as `REHEARSAL`, sends a bad CSV and reads the refusal, asks for a sync, and resets the password to its own value (which signs the session out, as it should).

## Where it lives

`crates/tt-ops`, a binary of its own beside `tt-load`, so ratatui and crossterm stay out of the server. `deploy/build-release.sh` builds it with the server, and `deploy/pi/service/install.sh` copies it to `/opt/tealteam/tt-ops` when it finds one beside the server binary. It is not a service.
