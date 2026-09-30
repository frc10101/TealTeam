# The database on the Pi's SSD (P3)

The server keeps everything in one SQLite file. On the Pi it belongs on an NVMe or USB SSD, **not the SD card**. SD cards wear out under a database's small, constant writes, and the day one fails is an event day (REBUILD_SPEC.md §10).

> **Untested.** Everything below that touches hardware (the SSD, `parted`, `fstab`, power) was written from the standard Raspberry Pi OS procedure and has **not** been run on the team's Pi. Do it once at the shop and fix this page where it is wrong. What has been tested is listed at the end.

## What the server already does

- **WAL mode, one writer.** The database runs in WAL mode with `synchronous=NORMAL`: readers never wait for the writer, and a commit costs no extra fsync. The server holds **one** connection, so its writes queue up one at a time; a second write waits up to 5 seconds rather than failing. Tests in `tt-repo-sqlite` check both on a real file.
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

5. **Make the data directory *inside* the mount.**

   ```sh
   sudo mkdir /srv/tealteam/data
   sudo chown "$USER": /srv/tealteam/data
   ```

   This matters. If the SSD is missing at boot, `/srv/tealteam` is an empty folder on the SD card, with no `data/` in it. SQLite does not create folders, so the server cannot open its database. Every page then shows **Storage unavailable — data is not being saved**, instead of the server quietly starting an empty database on the SD card.

6. **Point the server at it**, in `.env` beside the binary (three slashes: `sqlite://` and then an absolute path):

   ```
   DATABASE_URL=sqlite:///srv/tealteam/data/tealteam.db
   ```

7. **Moving a database that already exists:** stop the server first, then copy with SQLite's own backup command. It copies everything, including what is still in the `-wal` file:

   ```sh
   sqlite3 ~/tealteam.db ".backup /srv/tealteam/data/tealteam.db"
   ```

   Copying the `.db` file by itself can lose the most recent writes.

8. **Check the startup line** says `on nvme0n1p1` (or `sda1`), not `the SD card`.

If the server runs as a systemd service, add `RequiresMountsFor=/srv/tealteam` to its `[Unit]` section, so it waits for the mount.

## What was tested, and what was not

Tested, on a development machine, not a Pi:

- WAL mode, `synchronous=NORMAL`, the single connection, and a second write waiting for the first (unit tests).
- The log line naming the path and device on a real disk: an encrypted btrfs root reported as `dm-0`, and tmpfs reported as "an unknown device".
- The SD-card wording, and how device names are classified (unit tests with a made-up `mmcblk0p2`).

Not tested:

- Any of steps 1–8 above.
- The warning firing on a real Pi booted from its SD card.

Known blind spots, where no warning fires:

- An SD card in a USB reader shows up as `sda`, like an SSD.
- A LUKS volume on the SD card shows up as `dm-0`.
