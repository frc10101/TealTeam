# HTTPS on the event LAN: getting the app and a certificate

*How to put TealTeam behind HTTPS on the Pi (or a laptop standing in for it), so the Chromebooks get a **secure context** and the offline features turn on. Written 2026-10-22. **Not yet run end to end**: work through [Verify](#5-verify) at the shop before relying on it.*

## Why, and what it changes

Chrome turns on service workers and private file storage (OPFS) only for a secure context: HTTPS, or `http://localhost`. Over plain `http://tealteam.local` they are off (`static/js/sync.js` checks `window.isSecureContext`). The LAN rehearsal (`docs/ftc/FTC_BETA_PLAN.md` §5 B1, 2026-10-22) showed what that costs: saved forms are held and sync later, but a reload or a link while the server is unreachable shows Chrome's error page.

With HTTPS:

- a reload while the server is out of reach shows the page from the laptop's copy, with what it holds;
- phones can keep a full offline copy (C1–C7, S10).

Still true with HTTPS (RH1 in `docs/plan/ACTION_ITEMS.md`): a scout cannot open a match they have not opened before while offline. Paper stays the fallback.

**What this is not:** a public HTTPS site. Open decision 5 still stands: nothing is served to the internet. The certificate comes from a certificate authority (CA) that lives on the Pi and is trusted only by our own devices. No domain, no Let's Encrypt, no internet at the venue.

**The cost:** every Chromebook and phone must have our CA's certificate installed once (Part 4). A device without it gets a full-page warning. Plain `http://` keeps working beside HTTPS as the fallback.

## How it fits together

```
Chromebook ──https://tealteam.local──▶ Caddy :443 (tls internal) ──▶ tt-web 127.0.0.1:8080
           ──http://tealteam.local───▶ nft redirect :80 → :8080 (unchanged, the fallback)
```

[Caddy](https://caddyserver.com) is a web server that runs its own small CA (`tls internal`). It makes a root certificate once (valid 10 years), and from it issues and renews the server's certificate by itself, offline. tt-web is not changed: it keeps serving plain HTTP on 8080, and Caddy forwards to it. The session cookie is not marked `Secure` (`crates/tt-web/src/auth.rs`), so signing in works over both.

## 1. Get the application

On the Pi (or the server laptop), with internet, at the shop:

```sh
git clone -b ftc-biobuzz https://github.com/frc10101/TealTeam.git ~/TealTeam   # the repo is public; no SSH key needed
cd ~/TealTeam
git checkout <final tag>            # the build you will run at the event
deploy/build-release.sh             # 20–40 min on a Pi; prints the install line
```

Then bring it up as usual:

- **Pi:** `sudo deploy/pi/bootstrap.sh deploy/profiles/ftc-qual-4robot.env target/release/tt-web` (`docs/event/SETUP_GUIDE.md`). It sets the hostname `tealteam`, `eth0` at `10.101.0.1`, DHCP/DNS, Avahi and the port-80 redirect.
- **A laptop instead of the Pi:** `deploy/laptop/omenMaxwell-setup.sh start --port 8080`. A laptop also needs DHCP/DNS and mDNS on its wired port, as in the rehearsal: NetworkManager shared mode on `10.101.0.1/24`, `/etc/NetworkManager/dnsmasq-shared.d/tealteam.conf`, and `avahi-publish -a -R tealteam.local 10.101.0.1`. ChromeOS asks only mDNS for `.local` names.

Check `curl -s http://127.0.0.1:8080/health` says `"storage":"ready"` before going on.

## 2. Install Caddy

Still with internet:

```sh
sudo apt install caddy          # Raspberry Pi OS / Debian
sudo pacman -S caddy            # Arch / Omarchy laptop
```

Both install a `caddy` systemd service that runs as the `caddy` user.

## 3. Configure it and make the certificate

Replace `/etc/caddy/Caddyfile` with:

```caddyfile
{
	# Port 80 stays with tt-web (the nft redirect), the plain-HTTP fallback.
	auto_https disable_redirects
	# Do not try to add the CA to this machine's own trust store.
	skip_install_trust
}

https://tealteam.local, https://tealteam, https://10.101.0.1 {
	tls internal
	reverse_proxy 127.0.0.1:8080
}
```

Every name a device might type is listed, because the certificate only covers the names in it. The IP is included because the cards print `http://10.101.0.1/` as the fallback.

Start it, and let it make its CA and certificate:

```sh
sudo systemctl enable --now caddy
sudo systemctl reload caddy                  # after any Caddyfile change
journalctl -u caddy -n 30                     # look for "certificate obtained successfully"
```

Open port 443 if the machine has a firewall (the Pi image has none; a laptop with ufw does):

```sh
sudo ufw allow in on eth0 to any port 443 proto tcp      # use the wired interface's name
```

**The CA's root certificate**, which every device must trust:

```
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
```

Copy it to a USB stick as `tealteam-ca.crt`:

```sh
sudo cp /var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt /media/$USER/STICK/tealteam-ca.crt
```

**Keep the CA for good.** Its private key (`root.key` beside it) is what every device now trusts. Back up the whole `pki/` folder to the event stick with the database backup, and never put it in git or chat. If the Pi is rebuilt without it, Caddy makes a new CA and every device has to be done again. To restore, put the folder back before starting Caddy. Anyone with `root.key` could impersonate any website to our devices, so treat it like a password.

## 4. Trust the CA on each device (once per device)

The file is `tealteam-ca.crt` from the stick. Do this at the shop, not at the venue.

**Chromebook (one at a time):**

1. Copy `tealteam-ca.crt` into **Downloads**.
2. Open `chrome://certificate-manager` (older ChromeOS: Settings → Privacy and security → Security → Manage certificates).
3. Under **Local certificates** (older: the **Authorities** tab), **Import**, pick the file.
4. Tick **Trust this certificate for identifying websites**, then OK.
5. Open **https://tealteam.local/**: there should be no warning, and the padlock should show.

**Managed Chromebooks (all at once):** in the Google Admin console, Devices → Networks → Certificates, add `tealteam-ca.crt` for the organisational unit with **Chromebook** and **Use this certificate as an HTTPS certificate authority** ticked. The school's admin has to do this.

**Android phone:** Settings → Security (or Security & privacy) → More security settings → Encryption & credentials → Install a certificate → **CA certificate** → pick the file. Android warns that a CA can see traffic; this one only covers our server.

**iPhone:** AirDrop or email the file to the phone and open it, then Settings → **Profile Downloaded** → Install. Then turn it on: Settings → General → About → **Certificate Trust Settings** → switch on the TealTeam CA.

## 5. Verify

At the shop, with the full wired setup:

- [ ] `curl --cacert tealteam-ca.crt -sI https://tealteam.local/health` on the Pi answers `200`.
- [ ] On each Chromebook: **https://tealteam.local/**, **https://tealteam/** and **https://10.101.0.1/** open with no warning.
- [ ] Signed in as a scout on HTTPS, DevTools → Application → Service workers shows one registered.
- [ ] Offline test (DevTools → Network → Offline, or unplug): save a form, reload. The page comes back with **Offline · 1 held**. Back online, the chip says **Synced** and the lead's queue has the form exactly once.
- [ ] Plain **http://tealteam.local/** still works.
- [ ] A phone that has the CA opens https and keeps its offline copy. One without it shows a warning, and http still works.
- [ ] `journalctl -u caddy` has no errors after a reboot, and the certificate renews by itself (Caddy renews the server's certificate long before it expires; nothing to do while it runs).

## 6. On the day, and after

- **The cards:** if every device has the CA, change the addresses on the scout, lead and Chromebook cards to `https://` (`docs/event/print/build.py`, then `python3 docs/event/print/build.py`). If even one device lacks it, keep `http://` on the cards and use https only on the devices that have it.
- **The clock:** certificates depend on the Pi's clock. The runbook already checks `date` at setup; a Pi that boots without internet and a wrong date can make the certificate look not yet valid.
- **If HTTPS breaks during the event:** use `http://`. Nothing about the data depends on which one a device used.
- **Removing it:** `sudo systemctl disable --now caddy`. On each device, delete the TealTeam CA from the same screen it was imported on.
