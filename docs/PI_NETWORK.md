# The Pi's network at an event

How scouts reach the server, and how the server reaches the internet (P4, P6). The files are in `deploy/pi/network/`, and `setup.sh` installs all of them. Storage (the SSD, WAL) is a separate document.

> **None of this has run on a Pi yet.** The scripts pass ShellCheck. The nftables ruleset loads, twice, in a scratch network namespace. The Avahi file is well-formed XML, and `setup.sh --dry-run` runs. Nothing else is tested. Work through [What to verify](#what-to-verify) at the shop before the first event.

## The shape

```
   phones, tablets, laptops ──(Ethernet, USB-C adapters)──▶ unmanaged switch ──▶ Pi eth0  10.101.0.1
                                                                                  Pi usb0  ◀── one phone, USB-tethered
```

- **Clients are wired.** Every client plugs into an unmanaged switch cabled to the Pi's `eth0`. Laptops use their own port; phones and tablets use a USB-C-to-Ethernet adapter. Ethernet is unambiguously allowed at events.
- **There is no Wi-Fi access point, and nothing here can make one.** Rule E143 bars teams from setting up their own wireless in the venue. `setup.sh` deletes any access-point or ad-hoc Wi-Fi profile it finds, and `status.sh` fails if one comes back.
- **The internet comes from one phone, USB-tethered.** One person owns it. A phone acting as a phone is not "setting up wireless communication". The server works without it: the tether only carries TBA and FIRST syncs.

**The rules are not settled.** P2 (confirming 2026's E143 wording, and asking the FTA) is still open. This follows the plan's recommendation, which is to design for the compliant path. If the FTA objects to the tethered phone, unplug it. Nothing else changes.

## What each file does

| File | Installed to | What it does |
| --- | --- | --- |
| `tealteam-lan.nmconnection` | `/etc/NetworkManager/system-connections/` | `eth0` at `10.101.0.1/24`, static, and **never a default route** |
| `tealteam-uplink.nmconnection` | same | Any tethered phone: `usb0` (Android) or `eth1` (iPhone). DHCP from the phone, route metric **50**, the lowest on the Pi, so it wins whenever it is plugged in |
| `dnsmasq-tealteam.conf` | `/etc/dnsmasq.d/tealteam.conf` | DHCP on `eth0` only: `10.101.0.50`–`.250`, the Pi as DNS, and **no gateway**. Answers `tealteam.local` and `tealteam` over plain DNS; forwards nothing |
| `tealteam-http.service.avahi` | `/etc/avahi/services/tealteam.service` | Advertises the app as an `_http._tcp` service on port 80 |
| `port80.nft` + `tealteam-port80.service` | `/etc/tealteam/`, `/etc/systemd/system/` | Redirects port 80 on `eth0` (and on the Pi itself) to the app's 8080, so the URL has no port and the app needs no privilege |

`setup.sh` also sets the hostname to `tealteam`, which is where `tealteam.local` comes from, and installs `dnsmasq`, `avahi-daemon`, `avahi-utils`, and `nftables` if they are missing. It ranks any shop Wi-Fi client profile at metric 600, below the phone.

### Why clients get no gateway

A tablet with its own cellular data keeps using it for the internet, and reaches only the server over the cable. The Pi does not route or share its tether. That keeps one person's data plan from carrying thirty tablets' app updates, and it makes the wired network plainly a LAN, not an internet service.

### Why plain DNS as well as mDNS

`tealteam.local` is an mDNS name, and Avahi answers it. macOS, iOS, Windows 10+, and most Linux resolve it that way. Android's support depends on version and browser. So the DHCP lease also names the Pi as DNS server, and dnsmasq answers `tealteam.local` and `tealteam` directly. A client that ignores both still has **http://10.101.0.1**. Print that on the runbook (P8).

## Using it

```sh
# once, and again after any change in deploy/pi/network/
sudo deploy/pi/network/setup.sh --dry-run   # what it would change
sudo deploy/pi/network/setup.sh

# any time, especially on arrival at an event
deploy/pi/network/status.sh
```

`status.sh` changes nothing. It checks the hostname, `eth0`'s address, which way the default route goes, that no AP profile exists, that dnsmasq, Avahi and the port-80 redirect are running, that `tealteam.local` resolves, and that the app answers `http://localhost/health`. It exits non-zero on any failure. A missing uplink is a note, not a failure.

**The app's port is assumed to be 8080**, the default. If `PORT` is set to something else, change `port80.nft` to match and rerun `setup.sh`.

**Being conservative about Wi-Fi:** the Pi's Wi-Fi as a *client* of the shop's network is not an access point, and the route metric keeps it behind the phone. For events, `sudo nmcli radio wifi off` turns the radio off entirely, and `on` turns it back on at the shop.

## What to verify

At the shop, with the real switch, cables, adapters, and phones:

1. `setup.sh` runs clean on a fresh Raspberry Pi OS Bookworm image, and **a second run changes nothing**.
2. After a reboot, `status.sh` passes with the switch cabled and no phone.
3. A laptop on the switch gets a `10.101.0.x` lease with no gateway, and opens `http://tealteam.local`.
4. **Resolution on each platform** (RI-N3): an iPhone, an Android phone (with Chrome), a Mac, and a Windows laptop, each wired through the switch. On each, note whether `tealteam.local` resolved by mDNS or by the DNS fallback, and which needed `10.101.0.1`.
5. An Android phone on the switch **keeps its cellular internet** while it reaches the server over Ethernet. Android may not route to a wired network it decides "has no internet". If so, that is the first thing to fix.
6. An Android phone tethered by USB: `usb0` comes up, `ip route` shows the default via `usb0` at metric 50, and a manual sync on the Lead Scout page succeeds. Then the same with an iPhone (`eth1`).
7. Pull the tether mid-sync: the server says "No internet" and keeps serving.
8. `nmcli connection show` lists no Wi-Fi AP. Create one by hand and confirm `status.sh` fails and `setup.sh` deletes it.
9. Time it: set up and tear down twice, by a student who did not write this (P9).
