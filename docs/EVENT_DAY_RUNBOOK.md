# Event day: set up the scouting server

*One page. Print it, laminate it, tape it to the Pi's case. Detail: `docs/PI_NETWORK.md` (network), `docs/PI_STORAGE.md` (SSD, backups).*

> **[ PHOTO GOES HERE: the Pi, the switch, and every cable plugged in correctly. Label each cable. ]**

**Kit:** Pi (SSD attached) · official 27 W Pi supply · unmanaged switch + its power · one flat Ethernet cable per client · USB-C-to-Ethernet adapters · gaff tape · the uplink phone + its USB cable · a USB stick. **No Wi-Fi hotspot or router, ever** (rule E143).

## Set up (about 10 minutes)

1. **Cable before power.** Pi `eth0` → switch. Each client → switch (phones/tablets through an adapter). Tape every cable run flat.
2. **Power on in order:** switch first, then the Pi. Wait for the Pi's green light to settle (about a minute).
3. **The server starts by itself** at boot. Confirm with `systemctl status tealteam`: it should say `active (running)`. If not, `sudo systemctl restart tealteam`, and `journalctl -u tealteam -n 50` says why.
4. **Check:** in the TealTeam folder, `deploy/pi/network/status.sh`. Every line should be `ok`. Any `FAIL` names the fix. `no uplink` is fine for now.
5. **Uplink (optional):** the phone's owner plugs it into the Pi and turns on **USB tethering**. Run `status.sh` again: it should say *internet via the tethered phone*. The server syncs by itself within seconds; the Lead Scout page says **Phone tether: plugged in**. Without it, scouting still works; only rankings stop updating.
6. **Confirm from a tablet:** plug it into the switch and open **http://tealteam.local**. If that fails, try **http://tealteam**, then **http://10.101.0.1**. The sign-in page means it works. Write down which address worked on this model.
7. **Lead scout:** sign in, pick the event, check the rankings say how old they are.

## Between match blocks: back up to the stick

Plug in the stick, then:

```
sudo /opt/tealteam/tt-web backup /media/$USER/STICK
```

Wait for **"It is on the disk."** Check the observation count looks right, then eject the stick before pulling it. The server also snapshots itself to the SSD every 10 minutes; the stick is the copy that leaves the building.

## If something is wrong

| Symptom | Do this |
| --- | --- |
| Tablet can't open the page | Cable and adapter seated? Switch powered? Try `http://10.101.0.1`. Run `status.sh`. |
| `status.sh`: eth0 FAIL | Switch unpowered or Pi cable loose. |
| `status.sh`: app FAIL | `sudo systemctl restart tealteam`; if it keeps failing, is the SSD plugged in? |
| Rankings stale (amber) | Phone unplugged or no signal: re-plug, re-enable USB tethering. |
| FTA asks about wireless | There is none. Offer to unplug the phone; nothing else changes. |

## Tear down

1. Last backup to the stick (above), and eject it.
2. `sudo poweroff` (it stops the server cleanly) and wait for the green light to stop.
3. Unplug the Pi, then the switch. Coil cables; count adapters back in.
