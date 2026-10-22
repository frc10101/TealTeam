// Keeps this device's local copy in step with the Pi (C7, C12).
//
// The work is the service worker's: tt-client's wasm module opens this
// device's copy in OPFS, pushes its outbox, then asks the Pi for what changed
// since its own cursors and applies it -- insert, update, or delete, keyed
// the same way the change log is. Browser pages are read-only (open decision
// 3), so the outbox is empty until something queues into it and the push
// sends nothing. This only says when: soon after the page opens, when the
// browser says it is back online, on each row from the live stream (S8),
// whose URL only the wasm module can build, since it is this device's own
// cursors and schema, and once more before the update banner (S11) reloads
// the page, in its detail.waitFor, so what is queued goes before the build
// changes under it.
//
// A device with no copy yet is given one first, with ttSnapshot.bootstrap
// (S10): every event this viewer may see, since Team 10101 is the whole
// deployment (open decision 4). A page that wants a narrower copy, such as
// the grid scoped to one event, asks for it before this script's first sync
// finds none and fills in the gap itself.
//
// A sync that finds this device's build too old for the Pi's schema
// ({"kind":"reload"}) asks for a fresh snapshot, so the next reload -- which
// link.js's update banner already offers -- opens a copy the new schema can
// read. Not while anything is still in the outbox: a fresh copy has none, so
// that would lose it. Then the outbox is offered as a file instead, to hand
// to the lead scout, once per page, and the copy is kept as it is.
//
// OF1: the page also hands the worker its offline token at once, so the
// device knows whose scouting page to make if the server goes before the
// first sync; it tells the chip what the device's outbox still holds after
// each sync; and while anything does, it tries again when link.js sees the
// server and every 10 s.
//
// Only where a service worker runs, which needs https (open decision 9); this
// script and snapshot.js each check that for themselves. Each sync's report
// is dispatched on document as "tt:sync", for anything that wants to show it.
// window.ttSync.sync() and .saveOutbox() are there for a page's own buttons.
(function () {
  "use strict";

  var FIRST_MS = 3 * 1000;
  var DEBOUNCE_MS = 500;
  var RETRY_MS = 10 * 1000;

  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;

  function ask(message) {
    return new Promise(function (resolve) {
      var worker = navigator.serviceWorker.controller;
      if (!worker) return resolve(null);
      var channel = new MessageChannel();
      channel.port1.onmessage = function (event) {
        resolve(event.data);
      };
      worker.postMessage(message, [channel.port2]);
    });
  }

  function bootstrapIfNeeded() {
    if (!window.ttSnapshot) return Promise.resolve();
    return window.ttSnapshot.local().then(function (local) {
      if (local) return;
      return window.ttSnapshot.bootstrap({}).catch(function () {
        // Already exists (another tab just made one), offline, full, or
        // another schema: nothing more to do here. The next trigger, or a
        // reload, tries again.
      });
    });
  }

  function stamp() {
    var d = new Date();
    var two = function (n) { return (n < 10 ? "0" : "") + n; };
    return d.getFullYear() + two(d.getMonth() + 1) + two(d.getDate()) + "-" +
      two(d.getHours()) + two(d.getMinutes());
  }

  // The outbox as a download, when there is anything in it. Resolves true
  // when a file was offered.
  function saveOutbox() {
    return ask({ type: "tt-export-outbox" }).then(function (file) {
      if (!file) return false;
      var url = URL.createObjectURL(new Blob([file], { type: "application/json" }));
      var link = document.createElement("a");
      link.href = url;
      link.download = "tealteam-unsent-" + stamp() + ".json";
      document.body.appendChild(link);
      link.click();
      link.remove();
      window.setTimeout(function () { URL.revokeObjectURL(url); }, 60 * 1000);
      return true;
    });
  }

  // A build too old for the Pi. With nothing queued, a fresh copy for the
  // next reload; with something queued, the file, and the copy kept.
  var offered = false;
  function behind(report) {
    if (report.queued > 0) {
      if (offered) return Promise.resolve();
      offered = true;
      return saveOutbox();
    }
    if (!window.ttSnapshot) return Promise.resolve();
    return window.ttSnapshot.bootstrap({ replace: true }).catch(function () {});
  }

  var running = null;
  var retry = null;
  function sync() {
    if (running) return running;
    running = bootstrapIfNeeded()
      .then(function () {
        var bearer = window.ttToken ? window.ttToken.token() : null;
        return ask({ type: "tt-sync", bearer: bearer });
      })
      .then(function (report) {
        if (!report) return report;
        if (window.ttLink) window.ttLink.held(report.queued, "device");
        if (report.queued > 0) {
          window.clearTimeout(retry);
          retry = window.setTimeout(sync, RETRY_MS);
        }
        document.dispatchEvent(new CustomEvent("tt:sync", { detail: report }));
        if (report.stopped && report.stopped.kind === "reload") {
          return behind(report).then(function () { return report; });
        }
        return report;
      })
      .finally(function () {
        running = null;
      });
    return running;
  }

  var stream = null;
  function openStream() {
    if (stream || !window.EventSource) return;
    ask({ type: "tt-stream-url" }).then(function (url) {
      if (!url || stream) return;
      stream = new EventSource(url);
      var timer = null;
      var onRow = function () {
        window.clearTimeout(timer);
        timer = window.setTimeout(sync, DEBOUNCE_MS);
      };
      stream.addEventListener("change", onRow);
      stream.addEventListener("upstream", onRow);
      stream.addEventListener("cursor", onRow);
      stream.onerror = function () {
        // A 409 (another schema) closes the stream for good, the same
        // trouble a sync already knows how to find and name; anything else
        // is the browser reconnecting to the same URL on its own.
        if (stream && stream.readyState === EventSource.CLOSED) {
          stream = null;
          sync();
        }
      };
    });
  }

  // The update banner's reload waits for this, up to its own cap: one more
  // push of the outbox, and the file if the new build refuses it. A sync
  // still under way at the cap carries on in the worker, and what it did not
  // send is still in the copy after the reload.
  document.addEventListener("tt:before-update", function (event) {
    if (!event.detail || !event.detail.waitFor) return;
    if (stream) {
      stream.close();
      stream = null;
    }
    event.detail.waitFor.push(sync());
  });

  window.ttSync = { sync: sync, saveOutbox: saveOutbox };

  if (window.ttToken && window.ttToken.token()) {
    ask({ type: "tt-remember", bearer: window.ttToken.token() });
  }
  document.addEventListener("tt:reachable", function () {
    sync().then(openStream);
  });

  window.setTimeout(function () {
    sync().then(openStream);
  }, FIRST_MS);
  window.addEventListener("online", function () {
    sync().then(openStream);
  });
})();
