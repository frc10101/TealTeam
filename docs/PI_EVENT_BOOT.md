# Raspberry Pi Event Boot Setup

This setup makes TealTeam start automatically when the Pi powers on.
It also prints the Pi URL and optionally shows it on a 16x2 I2C LCD.

`pi_first_boot.sh` and `install_pi_autostart.sh` both install Python and the required script libraries (`RPLCD`, `smbus2`).

## 1. One-time Pi setup

Run on the Pi from the project root:

```bash
sudo ./scripts/pi_first_boot.sh
```

Then install autostart:

```bash
sudo ./scripts/install_pi_autostart.sh
```

Then start once now (without reboot):

```bash
sudo systemctl start tealteam.service
```

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
- Teams connect to `http://<pi-ip>/` (or include the custom port).

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
- Set `LCD_ENABLED=false` in the service environment to disable LCD writes.

## 5. Useful commands

```bash
systemctl status tealteam.service
journalctl -u tealteam.service -f
docker compose -f docker-compose.pi.yml ps
cat /tmp/tealteam-url.txt
```
