# The database on the Pi's SSD, and its backups (P3, Q4)

The server keeps everything in one SQLite file. On the Pi it belongs on an NVMe or USB SSD, **not the SD card**. SD cards wear out under a database's small, constant writes, and the day one fails is an event day (REBUILD_SPEC.md §10).

> **Untested.** Everything below that touches hardware (the SSD, `parted`, `fstab`, power) was written from the standard Raspberry Pi OS procedure and has **not** been run on the team's Pi. Do it once at the shop and fix this page where it is wrong. What has been tested is listed at the end.

## What the server already does

- **WAL mode, one writer, every commit on the disk.** The database runs in WAL mode, so readers never wait for the writer, with `synchronous=FULL`: each commit is flushed to the SSD before the scout is told "saved", so a power cut cannot take back a save. Under `NORMAL` it could (found in Q3, changed in Q3b). The flush is an fsync per commit, well under a millisecond on an NVMe drive; see [LOAD_TEST.md](LOAD_TEST.md#synchronousfull-q3b). The server holds **one** connection, so its writes queue up one at a time; a second write waits up to 5 seconds rather than failing. Tests in `tt-repo-sqlite` check both on a real file.
- **Says where the database is.** At startup it logs the full path and the device under it:

  ```
  INFO database is at /srv/tealteam/data/tealteam.db on nvme0n1p1
  ```

  If that device is the SD card, it is a warning instead:

  ```
  WARN database is at /home/pi/tealteam.db on the SD card (mmcblk0p2) -- an SD card wears out ...
  ```

  `tt-web bulk-load` prints the same line.

## Put it on the SSD

1. **Attach the SSD.** An NVMe drive on the Pi 5's PCIe HAT, or a USB 3 SSD in a **blue** port. A USB SSD needs the official 27 W (5 A) supply; on a weaker one the Pi limits USB current and the drive may drop out under load.

2. **Find it.** It is `nvme0n1` or `sda`, never `mmcblk0` (that is the SD card):

   ```sh
   lsblk -o NAME,SIZE,MODEL,MOUNTPOINTS
   ```

3. **Partition and format it.** This erases the drive. Replace `nvme0n1` with the name from step 2; a USB drive's partition is `sda1`, not `sda` + `p1`.

   ```sh
   sudo parted /dev/nvme0n1 --script mklabel gpt mkpart tealteam ext4 0% 100%
   sudo mkfs.ext4 -L tealteam /dev/nvme0n1p1
   ```

4. **Mount it at boot, by UUID.** Device names can change between boots, but a UUID never does.

   ```sh
   sudo mkdir -p /srv/tealteam
   sudo blkid -s UUID -o value /dev/nvme0n1p1     # prints the UUID
   ```

   Add this line to `/etc/fstab`, with that UUID:

   ```
   UUID=<the uuid>  /srv/tealteam  ext4  defaults,noatime,nofail,x-systemd.device-timeout=10s  0  2
   ```

   Then `sudo mount -a` and `findmnt /srv/tealteam` should show the SSD. `nofail` lets the Pi boot without the SSD, so you can still get in over SSH and see what is wrong.

5. **Make the data directory *inside* the mount.** `deploy/pi/service/install.sh` does steps 5 and 6 for you (folders owned by the `tealteam` user, `.env` at `/opt/tealteam/.env`); the commands below are what it does, for a server run by hand.

   ```sh
   sudo mkdir /srv/tealteam/data /srv/tealteam/backups
   sudo chown "$USER": /srv/tealteam/data /srv/tealteam/backups
   ```

   This matters. If the SSD is missing at boot, `/srv/tealteam` is an empty folder on the SD card, with no `data/` in it. SQLite does not create folders, so the server cannot open its database. Every page then shows **Storage unavailable — data is not being saved**, instead of the server quietly starting an empty database on the SD card.

6. **Point the server at it**, in `/opt/tealteam/.env` (or `.env` beside a hand-run binary) (three slashes: `sqlite://` and then an absolute path):

   ```
   DATABASE_URL=sqlite:///srv/tealteam/data/tealteam.db
   BACKUP_DIR=/srv/tealteam/backups
   ```

7. **Moving a database that already exists:** stop the server first, then copy with SQLite's own backup command. It copies everything, including what is still in the `-wal` file:

   ```sh
   sqlite3 ~/tealteam.db ".backup /srv/tealteam/data/tealteam.db"
   ```

   Copying the `.db` file by itself can lose the most recent writes.

8. **Check the startup line** says `on nvme0n1p1` (or `sda1`), not `the SD card`.

The systemd unit in `deploy/pi/service/` (P10) already has `RequiresMountsFor=/srv/tealteam`, so it waits for the mount.

## Backups (Q4)

The Pi holds the only authoritative copy of an event's scouting, so there are three layers of backup. They are listed below in how often each happens.

**Every 10 minutes, onto the SSD.** While it runs, the server snapshots the database into `BACKUP_DIR` (`/srv/tealteam/backups` above; unset, a `backups` folder beside the database). The files are named `tealteam-20260314T094000Z.db`, and the server keeps a day of them: 144 files. At startup it logs where they go:

```
INFO backing up every 10 minutes to /srv/tealteam/backups on nvme0n1p1, keeping a day
```

If that folder is on the SD card, this line is a warning too. A folder named in `BACKUP_DIR` is never created by the server. If the SSD is missing, each snapshot fails with a warning in the log rather than landing on the SD card. The next one after the SSD comes back works.

Three things about how the snapshots are taken:

- **Online, not a copy.** Each one is SQLite's `VACUUM INTO` from a connection of its own that only reads. That gives a consistent, complete database, including what is still in the `-wal` file, and the server's writes carry on during it. **Never back up with `cp` of the live `tealteam.db`**: that can miss recent writes, or catch the file mid-write.
- **Never half a file.** A snapshot is written as `….db.partial` and renamed when complete, so power lost mid-snapshot leaves no file that looks like a good backup.
- **A wrong clock never empties the folder.** Pruning goes by the time in the name, and the newest six are always kept. A Pi that boots without its RTC (P1) and thinks it is 2031 still keeps its last hour of backups.

**Between match blocks, onto a USB stick. One command, safe while the server runs:**

```sh
./tt-web backup /media/$USER/STICK
```

It takes a fresh snapshot onto the stick and syncs it to the disk. Then it **restores that copy into a fresh database to check it**, and prints what came back:

```
Backed up to /media/pi/STICK/tealteam-20260314T113000Z.db (0.2 MB)
Restored and checked: the database is whole. It holds
        7  observations
        5  pick list entries
       …
It is on the disk. Eject the stick before pulling it out.
```

Compare the observation count with what the lead scout's review page expects. Raspberry Pi OS Desktop mounts a stick under `/media/$USER/<label>`; on Lite, mount it yourself first.

**Off site: not decided.** Whose laptop gets a copy, and who checks it ran, is **open decision 6** in `ACTION_ITEMS.md`. Until it is settled, the destination is just a setting. `BACKUP_COPY_TO=/path` makes a bare `./tt-web backup` copy there. That path could be a mounted stick, or a folder that `rsync` or a synced drive carries off the Pi. Without it, a bare `./tt-web backup` says to name a folder.

### The restore test

Do this once at the shop, before an event needs it:

```sh
./tt-web check-backup                          # the newest 10-minute snapshot
./tt-web check-backup /media/pi/STICK/tealteam-20260314T113000Z.db
```

It copies the backup into a fresh database in a temporary folder and opens it the way the server does. Then it checks it is whole (`PRAGMA integrity_check`), applies this build's migrations, and prints the counts. The backup file is only read. A backup taken by an older build says how many migrations it needed.

### Putting a backup back

1. Stop the server.
2. Move the old database **and its `-wal` and `-shm` files** out of the way together. A leftover `-wal` belongs to the old database and would be replayed into the restored one:

   ```sh
   mkdir /srv/tealteam/replaced-$(date +%H%M)
   mv /srv/tealteam/data/tealteam.db* /srv/tealteam/replaced-$(date +%H%M)/
   ```

3. Copy the backup in under the database's name. A snapshot is a whole database with no `-wal` of its own, so a plain copy is right here, unlike for the live file:

   ```sh
   cp /srv/tealteam/backups/tealteam-20260314T113000Z.db /srv/tealteam/data/tealteam.db
   ```

4. Start the server, and check the counts on the lead scout page.

## What was tested, and what was not

Tested, on a development machine, not a Pi:

- WAL mode, `synchronous=FULL`, the single connection, and a second write waiting for the first (unit tests).
- The log line naming the path and device on a real disk: an encrypted btrfs root reported as `dm-0`, and tmpfs reported as "an unknown device".
- The SD-card wording, and how device names are classified (unit tests with a made-up `mmcblk0p2`).
- Backups: a unit test restores a snapshot into a fresh database and reads the users back through the server's own code. One of those users had been written only to the `-wal` file. The snapshot was taken while the server's writer held a transaction open, and it did not wait for it. Pruning keeps 24 hours and never fewer than six.
- `./tt-web backup` into a folder, run by hand while the server was serving a copy of a test database, then `./tt-web check-backup` on the result. Both printed the same counts. The server's first timed snapshot landed 10 minutes after start.

Not tested:

- Any of steps 1–8 above.
- A real USB stick: mounting, `/media` paths, and whether the sync before "It is on the disk" is enough on a FAT-formatted stick. Eject it anyway.
- The steps under "Putting a backup back", beyond the unit test that does the same copy.
- The warning firing on a real Pi booted from its SD card.

Known blind spots, where no warning fires:

- An SD card in a USB reader shows up as `sda`, like an SSD.
- A LUKS volume on the SD card shows up as `dm-0`.
