# HTTPS on the event LAN: getting the app and its certificate

*How to run TealTeam over HTTPS on the Pi (or a laptop standing in for it), so the Chromebooks get a **secure context** and the offline features turn on. Built 2026-10-22 (H1). Checked on a desktop with headless Chromium trusting the CA. **Not yet run on the Pi or a real Chromebook**: work through [Verify](#5-verify) at the shop.*

## Why

Chrome turns on service workers and private file storage (OPFS) only for a secure context: HTTPS, or `http://localhost`. Over plain `http://tealteam.local` they are off (`static/js/sync.js` checks `window.isSecureContext`). The LAN rehearsal (`docs/ftc/FTC_BETA_PLAN.md` §5 B1, 2026-10-22) showed the cost: saved forms are held and sync later, but a reload or a link while the server is unreachable shows Chrome's error page.

Over HTTPS, a reload while the server is out of reach shows the page from the laptop's copy, with what it holds, and phones keep a full offline copy. Over HTTPS the offline client also makes **Home, Teams, Notes, Graph and the drive coach's schedule** from the device's own copy of the data (`crates/tt-client/src/pages.rs`). It does **not** make a scouting form: a form not already open shows the offline page ("Can't reach the TealTeam server", with the agenda and the QR handoff). In the LAN test a scout's next assignment did open offline, most likely from Chromium's own page cache of a page that profile had already visited (over plain HTTP that showed Chrome's error page). Check it on a freshly set-up Chromebook before relying on it. Paper stays the fallback.

**What this is not:** a public HTTPS site. Open decision 5 stands: nothing is served to the internet. There is no domain, no Let's Encrypt and no internet at the venue. The server is its own certificate authority (CA), trusted only by our own devices.

## How it works

```
Chromebook ──https://tealteam.local──▶ nft :443 → :8443 ──▶ tt-web (HTTPS, its own CA)
           ──http://tealteam.local───▶ nft :80  → :8080 ──▶ tt-web (HTTP, the fallback)
```

- **The certificate.** On its first start, tt-web makes a CA (valid 10 years) and a server certificate signed by it, in `TLS_DIR`, which defaults to a `tls` folder beside the database (`/srv/tealteam/data/tls` on the Pi). The certificate covers `tealteam.local`, `tealteam`, `10.101.0.1`, `localhost`, `127.0.0.1`, and anything in `TLS_NAMES`. The server certificate lasts 397 days and is made again at start-up when under 30 days are left, or when the names change; the CA stays the same.
- **Both ports.** HTTPS on `TLS_PORT` (default 8443) and HTTP on `PORT` (8080) serve the same app. The Pi forwards 443 and 80 to them (`deploy/pi/network/port80.nft`). If HTTPS cannot start, the log says `HTTPS is off: …` and HTTP carries on.
- **Trusting it.** Every device fetches the CA certificate from **http://tealteam.local/ca.crt** and imports it once (Part 4). A device without it gets a full-page warning on https, and http still works.
- **Settings** (`/opt/tealteam/.env`, or the laptop's `.env`): `TLS_PORT` (`off` for none), `TLS_DIR`, `TLS_NAMES` (extra names or addresses, separated by commas), and `TLS_CERT` + `TLS_KEY` to use a certificate made elsewhere instead.
- **The crypto** is rustls with `ring`, which the build already carried; nothing new to install on the Pi.

## 1. Get the application

On the Pi (or the server laptop), with internet, at the shop:

```sh
git clone -b ftc-biobuzz https://github.com/frc10101/TealTeam.git ~/TealTeam   # public repo; no SSH key needed
cd ~/TealTeam
git checkout <final tag>            # the build you will run at the event
deploy/build-release.sh             # 20–40 min on a Pi
```

Then bring it up as usual:

- **Pi:** `sudo deploy/pi/bootstrap.sh deploy/profiles/ftc-qual-4robot.env target/release/tt-web` (`docs/event/SETUP_GUIDE.md`). The profile sets `TLS_PORT=8443`, and `setup.sh` loads the 443 forward. On a Pi set up before H1, re-run `sudo deploy/pi/network/setup.sh` to pick up the new forward, and add `TLS_PORT=8443` to `/opt/tealteam/.env`.
- **A laptop instead of the Pi:** `deploy/laptop/omenMaxwell-setup.sh start` opens 8080 and 8443 in ufw and puts the laptop's name and address on the certificate. A laptop also needs DHCP/DNS and mDNS on its wired port, as in the rehearsal: NetworkManager shared mode on `10.101.0.1/24`, `/etc/NetworkManager/dnsmasq-shared.d/tealteam.conf`, and `avahi-publish -a -R tealteam.local 10.101.0.1`. ChromeOS asks only mDNS for `.local` names.

## 2. Start it and find the CA

```sh
sudo systemctl restart tealteam
journalctl -u tealteam -n 30 | grep -i -E "certificate|https"
#   made a certificate authority in /srv/tealteam/data/tls ...   (first start only)
#   made the server certificate names=tealteam.local, tealteam, 10.101.0.1, ...
#   listening on https://0.0.0.0:8443
deploy/pi/network/status.sh          # "the app answers https://localhost (port 443) ..."
```

The folder now holds `ca.crt` (what devices trust), `ca.key` (mode 600: what makes certificates), `server.crt`, `server.key` and `server.names`.

## 3. Keep the CA

**Copy the whole `tls` folder to the backup stick once, and keep it.** `ca.key` is what every trusting device relies on. If the Pi is rebuilt without it, the server makes a new CA and every device has to be done again. To restore, put the folder back before the first start. Never put it in git or chat. Anyone with `ca.key` could impersonate a website to our devices, so treat it like a password.

```sh
sudo cp -a /srv/tealteam/data/tls /media/$USER/STICK/tealteam-tls
```

## 4. Trust the CA on each device (once per device)

Do this at the shop. On the wired network, open **http://tealteam.local/ca.crt** (or **http://10.101.0.1/ca.crt**); it downloads `tealteam-ca.crt`.

**Chromebook (one at a time):**

1. Open `http://tealteam.local/ca.crt`. It saves `tealteam-ca.crt` to **Downloads**.
2. Open `chrome://certificate-manager` (older ChromeOS: Settings → Privacy and security → Security → Manage certificates).
3. Under **Local certificates** (older: the **Authorities** tab), **Import**, and pick `tealteam-ca.crt`.
4. Tick **Trust this certificate for identifying websites**, then OK.
5. Open **https://tealteam.local/**: no warning, a padlock. Bookmark that address in place of the http one.

**Managed Chromebooks (all at once):** in the Google Admin console, Devices → Networks → Certificates, add `tealteam-ca.crt` with **Chromebook** and **Use this certificate as an HTTPS certificate authority** ticked. The school's admin does this.

**Android phone:** download it from the address above, then Settings → Security (or Security & privacy) → More security settings → Encryption & credentials → Install a certificate → **CA certificate** → `tealteam-ca.crt`.

**iPhone:** open the address in Safari and allow the profile download, then Settings → **Profile Downloaded** → Install. Then Settings → General → About → **Certificate Trust Settings** → turn on **TealTeam event CA**.

## 5. Verify

At the shop, with the full wired setup:

- [ ] `status.sh` on the Pi says the app answers `https://localhost`.
- [ ] On each Chromebook with the CA: **https://tealteam.local/**, **https://tealteam/** and **https://10.101.0.1/** open with no warning.
- [ ] Signed in as a scout on https: DevTools → Application → Service workers shows `/sw.js` activated.
- [ ] Offline (DevTools → Network → Offline, or unplug): save a form, then reload. The page comes back with **Offline · 1 held**. Back online, the chip says **Synced** and the lead's queue has the form once.
- [ ] Plain **http://tealteam.local/** still works on every device.
- [ ] A phone with the CA opens https and keeps its offline copy.
- [ ] After a reboot of the Pi, `journalctl -u tealteam` shows the certificate was kept, not made again.

Checked so far (2026-10-22, a desktop as the server on the LAN at 10.101.0.1, a second laptop driving Chromium 154 with the CA in its own certificate store):

- without the CA: `ERR_CERT_AUTHORITY_INVALID` on https, and http still works; with it: no warning, `isSecureContext` true, `/sw.js` activated, `/api/sync/stream` over HTTP/2;
- offline as a scout: save → **Offline · 1 held**; reload, and close and reopen the tab, both come back from the worker still holding it; the next assignment opened offline (likely the browser's page cache; the worker does not make forms, see Why); back online, **Synced** within 2 s and the lead's queue has it once;
- sign-in, a full save, approve, Compare, Graph and Help over https with no console errors and no 4xx/5xx;
- race tests over https: one record sent 12 times at once stored once, two scouts at once, 40 posts under constant sync pulls (20 of 20 stored once, 0 errors), 20 sign-ins at once;
- `curl --cacert ca.crt` verified `10.101.0.1`, `localhost` and `tealteam.local` over TLS 1.3 with HTTP/2; a restart kept the CA and certificate.

## 6. On the day, and after

- **The cards** give `https://tealteam.local/` first and `http://tealteam.local/` as the fallback. A device that warns about the certificate uses http, and the lead adds the CA to it later.
- **The clock:** certificates depend on the Pi's clock. The runbook already checks `date` at setup. The certificate is dated a day back, so a clock a few hours out is fine, but a Pi that boots to 1970 is not.
- **If HTTPS breaks during the event:** use `http://`. Nothing about the data depends on which one a device used.
- **Turning it off:** `TLS_PORT=off` in the `.env`, then restart. To remove the trust, delete the TealTeam CA on each device from the screen it was imported on.
