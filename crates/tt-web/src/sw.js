// TealTeam's service worker (C1). Served at /sw.js by src/shell.rs, which
// fills in the three placeholders below.
//
// What it does, and only this:
//
//   * On install, precache the offline shell: /offline and every static file,
//     into a cache named for this build. A new binary is a new worker with a
//     new cache; the old cache is deleted when it takes over.
//   * Pages (navigations): the network, always. Only when the network fails,
//     the device makes the page itself (C5): tt-client's wasm module runs the
//     server's own page code over the device's copy in OPFS. For an address
//     it cannot make, or with no copy yet, the cached /offline page -- at the
//     address that was asked for, so it can reload itself there once the
//     server answers. A page made here says so, and reloads the same way.
//   * Static files: the network first, so a page and its stylesheet always
//     come from the same build; the cache only when the network fails.
//   * Everything else passes straight through: POSTs, /health, /api, and the
//     live regions' fetches. Nothing a person sees is ever cached for them,
//     and nothing is written to the cache after install. A live region is
//     not made on the device: what it shows came from the server, and the
//     device's copy is no newer until the sync client (C7) keeps it current.
//   * A lead scout's page may ask for a courier tick (S7, static/js/courier.js):
//     the wasm module fetches from TBA with this device's signal and hands
//     the Pi what it fetched. One at a time, since it writes the device's
//     copy, and the asker gets the same answer as a tick already running.
//
// Registered by static/js/shell.js, and only in a secure context.

"use strict";

const BUILD = "__BUILD__";
const CACHE = "tealteam-shell-" + BUILD;
const SHELL = "/offline";
const PRECACHE = __PRECACHE__;
// The wasm module's script, or null in a binary built without it
// (deploy/build-client.sh). Its .wasm is beside it, and in PRECACHE.
const CLIENT = __CLIENT__;

if (CLIENT) {
  try {
    importScripts(CLIENT);
  } catch (e) {
    // Without it every page is the shell, as before C5.
    console.warn("TealTeam: no device pages:", e);
  }
}

// Compiled once per worker, from this build's cache, so it works with no
// server. A failure is not kept: the next page tries again.
let compiled = null;
function client() {
  if (!compiled) {
    compiled = caches
      .open(CACHE)
      .then((cache) => cache.match(CLIENT.replace(/\.js$/, "_bg.wasm")))
      .then((module) => {
        if (!module) throw new Error("the wasm module is not in the cache");
        return wasm_bindgen({ module_or_path: module });
      })
      .catch((e) => {
        compiled = null;
        throw e;
      });
  }
  return compiled;
}

// The page at `url`, made on this device, or null when it cannot be.
function fromDevice(url) {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  return client()
    .then(() => wasm_bindgen.render(url.pathname, url.search.slice(1)))
    .then((html) =>
      html === null
        ? null
        : new Response(html, {
            headers: { "content-type": "text/html; charset=utf-8", "x-tealteam-source": "device" },
          }),
    )
    .catch((e) => {
      console.warn("TealTeam: making " + url.pathname + " on this device:", e);
      return null;
    });
}

// The tick under way, if any.
let ticking = null;
function courierTick(bearer) {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  if (!ticking) {
    ticking = client()
      .then(() => wasm_bindgen.courier(bearer || null))
      .then((report) => (report === null ? null : JSON.parse(report)))
      .catch((e) => {
        console.warn("TealTeam: fetching upstream on this device:", e);
        return null;
      })
      .finally(() => {
        ticking = null;
      });
  }
  return ticking;
}

self.addEventListener("message", (event) => {
  const data = event.data || {};
  if (data.type !== "tt-courier") return;
  const port = event.ports && event.ports[0];
  event.waitUntil(
    courierTick(data.bearer).then((report) => {
      if (port) port.postMessage(report);
    }),
  );
});

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      // Past the HTTP cache: the shell must be this build's, not a copy the
      // browser kept from the last one.
      .then((cache) => cache.addAll(PRECACHE.map((url) => new Request(url, { cache: "reload" }))))
      // Take over at once. Pages and files still come from the network
      // first, so an open page does not mix two builds.
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(
          names
            .filter((name) => name.startsWith("tealteam-shell-") && name !== CACHE)
            .map((name) => caches.delete(name)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request).catch(() =>
        fromDevice(url).then(
          (page) =>
            page ||
            caches.open(CACHE).then((cache) => cache.match(SHELL)).then((shell) => shell || Response.error()),
        ),
      ),
    );
    return;
  }

  if (url.pathname.startsWith("/static/")) {
    event.respondWith(
      fetch(request).catch(() =>
        caches
          .open(CACHE)
          .then((cache) => cache.match(url.pathname))
          .then((file) => file || Response.error()),
      ),
    );
  }
});
