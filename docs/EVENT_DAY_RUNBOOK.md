# Event day: set up the scouting server

*One page. Print it, laminate it, tape it to the Pi's case. Detail: `docs/PI_NETWORK.md` (network), `docs/PI_STORAGE.md` (SSD, backups).*

> **[ PHOTO GOES HERE: the Pi, the switch, and every cable plugged in correctly. Label each cable. ]**

**Kit:** Pi (SSD attached) · official 27 W Pi supply · unmanaged switch + its power · one flat Ethernet cable per Lenovo Chromebook · USB-C-to-Ethernet adapters where needed · gaff tape · USB stick. **No Wi-Fi hotspot, router, mesh, cellular tether, or other 802.11/radio network for event data, ever.**

## Before you leave: install the build

On the Pi, with internet, in the TealTeam folder: `deploy/build-release.sh`, then the `sudo .../install.sh ...` line it prints. That build carries the offline client; `install.sh` refuses one without it.

## Set up (about 10 minutes)

1. **Cable before power.** Pi `eth0` → switch. Each Lenovo Chromebook → switch, using an adapter only when needed. Tape every cable run flat.
2. **Power on in order:** switch first, then the Pi. Wait for the Pi's green light to settle (about a minute).
3. **The server starts by itself** at boot. Confirm with `systemctl status tealteam`: it should say `active (running)`. If not, `sudo systemctl restart tealteam`, and `journalctl -u tealteam -n 50` says why.
4. **Check:** in the TealTeam folder, `deploy/pi/network/status.sh`. Every line should be `ok`. Any `FAIL` names the fix. `no uplink` is fine for now.
5. **Do not enable a phone tether at the event.** The Pi operates without an uplink; rankings and upstream data use the latest local snapshot.
6. **Confirm from a Chromebook:** plug it into the switch and open **http://tealteam.local**. If needed, use **http://tealteam** or **http://10.101.0.1**.
7. **Lead scout:** sign in, pick the event, and check the rankings age. Scouting and the lead's pages run on the wired Chromebooks. Phones are only for viewing data: one that is not wired uses its local copy and syncs later away from the event.

## Between match blocks: back up to the stick

Plug in the stick, then:

```
sudo /opt/tealteam/tt-web backup /media/$USER/STICK
```

Wait for **"It is on the disk."** Check the observation count looks right, then eject the stick before pulling it. The server also snapshots itself to the SSD every 10 minutes; the stick is the copy that leaves the building.

## If something is wrong

| Symptom | Do this |
| --- | --- |
| Chromebook can't open the page | Cable and adapter seated? Switch powered? Try `http://10.101.0.1`. Run `status.sh`. |
| `status.sh`: eth0 FAIL | Switch unpowered or Pi cable loose. |
| `status.sh`: app FAIL | `sudo systemctl restart tealteam`; if it keeps failing, is the SSD plugged in? |
| Rankings stale (amber) | The event has no uplink by design. Use the local snapshot and record when upstream data was last updated. |
| FTA asks about wireless | There is none. Offer to unplug the phone; nothing else changes. |

## Tear down

1. Last backup to the stick (above), and eject it.
2. `sudo poweroff` (it stops the server cleanly) and wait for the green light to stop.
3. Unplug the Pi, then the switch. Count the cables and transfer media.
