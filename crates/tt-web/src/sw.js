// TealTeam's service worker (C1). Served at /sw.js by src/shell.rs, which
// fills in the two placeholders below.
//
// What it does, and only this:
//
//   * On install, precache the offline shell: /offline and every static file,
//     into a cache named for this build. A new binary is a new worker with a
//     new cache; the old cache is deleted when it takes over.
//   * Pages (navigations): the network, always. Only when the network fails,
//     the cached /offline page -- at the address that was asked for, so it
//     can reload itself there once the server answers.
//   * Static files: the network first, so a page and its stylesheet always
//     come from the same build; the cache only when the network fails.
//   * Everything else passes straight through: POSTs, /health, /api, and the
//     live regions' fetches. Nothing a person sees is ever cached for them,
//     and nothing is written to the cache after install.
//
// Registered by static/js/shell.js, and only in a secure context.

"use strict";

const BUILD = "__BUILD__";
const CACHE = "tealteam-shell-" + BUILD;
const SHELL = "/offline";
const PRECACHE = __PRECACHE__;

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
        caches.open(CACHE).then((cache) => cache.match(SHELL)).then((shell) => shell || Response.error()),
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
