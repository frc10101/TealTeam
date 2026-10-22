# TealTeam: set up this Chromebook

1. USB-C Ethernet adapter into the Chromebook, flat cable into the switch.
2. Wi-Fi OFF: bottom right, click Wi-Fi, switch it off.
3. Open **http://tealteam.local/**
   If it will not open: **http://tealteam/**, then **http://10.101.0.1/**
4. Trust the server once: open **http://tealteam.local/ca.crt** (it downloads). Go to `chrome://certificate-manager`, Local certificates, Import, pick **tealteam-ca.crt**, tick *Trust this certificate for identifying websites*. Then open **https://tealteam.local/**: no warning, a padlock.
   (Details: docs/event/HTTPS_SETUP.md. Skip it and `http://` still works, without offline reloads.)
   Bookmark the **https** address (Ctrl+D). Pin it: three dots, Cast, save and share, Create shortcut, tick Open as window.
5. Sign in with the name and temporary password on your sign-in line. Then change the password on the Account page.
6. Clear old event data: type `chrome://settings/content/all` in the address bar, search `tealteam`, delete.
7. Keep the screen on: Settings, Device, Power, *While charging*: Keep display on. Stay plugged in.
8. Open **Scout** once and wait a moment so it saves a copy for offline.
   If the chip says **Offline**: with the padlock (https), keep scouting; each save is held on this Chromebook and sent when the server is back. Without it (http), do not reload or leave the page; tell the lead scout and use paper.

Stuck? Find the lead scout. Do not use Wi-Fi, a hotspot, or a phone.
