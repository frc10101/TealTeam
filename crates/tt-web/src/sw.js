// TealTeam's service worker (C1). Served at /sw.js by src/shell.rs, which
// fills in the four placeholders below.
//
// What it does, and only this:
//
//   * On install, precache the offline shell: /offline and every static file,
//     into a cache named for this build. A new binary is a new worker with a
//     new cache; the old cache is deleted when it takes over.
//     A device too full for all of it (Q6) keeps just /offline and the files
//     it loads, marked as such; static/js/shell.js reads the mark and warns.
//     With not even room for that, the install fails, the page warns, and
//     the next page tries again.
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
//   * Any signed-in page may ask for a sync (C7, C12, static/js/sync.js): the
//     wasm module pushes the device's outbox, if anything is in it, and then
//     applies whatever changed on the Pi to the device's copy. It may also
//     ask where to open the live stream (S8) for this device's own cursors,
//     which only the wasm module can read, and for the outbox as a file to
//     hand to the lead scout when it cannot be pushed.
//
// Registered by static/js/shell.js, and only in a secure context.

"use strict";

const BUILD = "__BUILD__";
const CACHE = "tealteam-shell-" + BUILD;
const SHELL = "/offline";
const PRECACHE = __PRECACHE__;
// The shell page and what it loads, for when PRECACHE does not fit (Q6).
const ESSENTIAL = __ESSENTIAL__;
// In a cache that holds only ESSENTIAL: why the rest did not fit, and what
// the rest is, for shell.js to add once there is room.
const PARTIAL = "/offline?partial";
const rest = () => PRECACHE.filter((url) => !ESSENTIAL.includes(url));
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

// The courier tick and the sync both write the device's copy, and each
// opens the file for itself and writes it back whole: two at once and the
// last one wins. So they take turns, and each runs one at a time -- a page
// that asks again while one is under way gets the same answer.
let turn = Promise.resolve();
function inTurn(work) {
  const next = turn.then(work, work);
  turn = next.catch(() => {});
  return next;
}

// The tick under way, if any.
let ticking = null;
function courierTick(bearer) {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  if (!ticking) {
    ticking = inTurn(() => client().then(() => wasm_bindgen.courier(bearer || null)))
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

// The sync under way, if any.
let syncing = null;
function sync(bearer) {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  if (!syncing) {
    syncing = inTurn(() => client().then(() => wasm_bindgen.sync(bearer || null)))
      .then((report) => (report === null ? null : JSON.parse(report)))
      .catch((e) => {
        console.warn("TealTeam: syncing this device's copy:", e);
        return null;
      })
      .finally(() => {
        syncing = null;
      });
  }
  return syncing;
}

// The outbox as a file's text, or null when there is nothing in it. It reads
// only, but in turn, so it is what the last sync left, not a sync's start.
function exportOutbox() {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  return inTurn(() => client().then(() => wasm_bindgen.exportOutbox())).catch((e) => {
    console.warn("TealTeam: writing this device's outbox out:", e);
    return null;
  });
}

function streamUrl() {
  if (typeof wasm_bindgen === "undefined") return Promise.resolve(null);
  return client()
    .then(() => wasm_bindgen.streamUrl())
    .catch((e) => {
      console.warn("TealTeam: reading this device's sync cursors:", e);
      return null;
    });
}

self.addEventListener("message", (event) => {
  const data = event.data || {};
  const port = event.ports && event.ports[0];
  if (data.type === "tt-courier") {
    event.waitUntil(
      courierTick(data.bearer).then((report) => {
        if (port) port.postMessage(report);
      }),
    );
  } else if (data.type === "tt-sync") {
    event.waitUntil(
      sync(data.bearer).then((report) => {
        if (port) port.postMessage(report);
      }),
    );
  } else if (data.type === "tt-export-outbox") {
    event.waitUntil(
      exportOutbox().then((file) => {
        if (port) port.postMessage(file);
      }),
    );
  } else if (data.type === "tt-stream-url") {
    event.waitUntil(
      streamUrl().then((url) => {
        if (port) port.postMessage(url);
      }),
    );
  }
});

function fill(cache, urls) {
  return cache.addAll(urls.map((url) => new Request(url, { cache: "reload" })));
}

// To every page of the site, controlled or not: a failed install leaves no
// cache for a page to read.
function tell(message) {
  return self.clients
    .matchAll({ includeUncontrolled: true, type: "window" })
    .then((pages) => pages.forEach((page) => page.postMessage(message)));
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      // Past the HTTP cache: the shell must be this build's, not a copy the
      // browser kept from the last one.
      .then((cache) => fill(cache, PRECACHE))
      // Out of room: addAll keeps nothing of a failed call, so try the part
      // that keeps a reload with no server off the browser's error page.
      .catch((e) =>
        caches
          .delete(CACHE)
          .then(() => caches.open(CACHE))
          .then((cache) =>
            fill(cache, ESSENTIAL).then(() => cache.put(PARTIAL, new Response(JSON.stringify({ reason: e.name, rest: rest() })))),
          ),
      )
      .catch((e) =>
        tell({ type: "tt-shell", state: "none", reason: e.name })
          .then(() => caches.delete(CACHE))
          .then(() => {
            throw e;
          }),
      )
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
