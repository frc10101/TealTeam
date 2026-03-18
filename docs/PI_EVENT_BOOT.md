# Raspberry Pi Event Boot Setup

This setup makes TealTeam start automatically when the Pi powers on.
It also prints the Pi URL and optionally shows it on a 16x2 I2C LCD.

`pi_first_boot.sh` and `install_pi_autostart.sh` both install Python and the required script libraries (`RPLCD`, `smbus2`).

Designed for headless event operation: no monitor, keyboard, or mouse required.

## 1. One-time Pi setup

Run on the Pi from the project root:

```bash
sudo ./scripts/pi_first_boot.sh
```

`pi_first_boot.sh` also creates a boot-partition config template at either:

- `/boot/firmware/tealteam.env` (Bookworm default)
- `/boot/tealteam.env` (older layouts)

Fill that file with your real API keys from any laptop that can read the SD card.
The service imports it automatically at boot, so you do not need interactive login on the Pi.

Then install autostart:

```bash
sudo ./scripts/install_pi_autostart.sh
```

Then start once now (without reboot):

```bash
sudo systemctl start tealteam.service
```

`install_pi_autostart.sh` also enables `tealteam-url-refresh.timer`, which refreshes URL files every 30 seconds for quick SSH checks.

After the first successful service start, boot mode is switched automatically to event mode:

- `FIRST_SYNC_ON_BOOT=false` is written to active config
- `TEALTEAM_BOOT_MODE=event` is written for visibility
- Marker file created at `.state/first_boot_mode_switched`

## 2. Ports used

- Web UI: host `TCP 80` -> container `8080` (default in Pi mode)
- PostgreSQL: not exposed to host (internal Docker network only)
- Adminer: not included in Pi runtime compose

If you need a different web port:

```bash
WEB_HOST_PORT=8080 sudo systemctl restart tealteam.service
```

## 3. Router / network recommendation

- Put the Pi on LAN via Ethernet if possible.
- Reserve a static DHCP lease in the router for the Pi MAC address.
- Teams connect to `http://tealteam.local/` when mDNS is available, or `http://<pi-ip>/`.

## 4. LCD support (optional hardware)

The script supports common I2C backpack LCDs via `RPLCD`.
If LCD dependencies are missing, startup continues normally.

Dependencies are installed automatically by the setup scripts.

Enable I2C on Raspberry Pi:

```bash
sudo raspi-config nonint do_i2c 0
sudo reboot
```

LCD defaults:

- I2C address: `0x27`
- I2C bus: `1`
- LCD is disabled by default (`LCD_ENABLED=false`) for headless operation.

## 5. Useful commands

```bash
systemctl status tealteam.service
systemctl status tealteam-url-refresh.timer
journalctl -u tealteam.service -f
journalctl -u tealteam-url-refresh.service -f
docker compose -f docker-compose.pi.yml ps
cat /tmp/tealteam-url.txt
cat /tmp/tealteam-mdns-url.txt
cat .state/first_boot_mode_switched
```
