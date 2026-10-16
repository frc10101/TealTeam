// Keeps this device's local copy pulled up to date (C7).
//
// Browser pages are read-only (open decision 3): there is nothing of this
// device's own to push, only whatever changed on the Pi to apply. The work is
// the service worker's: tt-client's wasm module opens this device's copy in
// OPFS, asks the Pi for what changed since its own cursors, and applies it --
// insert, update, or delete, keyed the same way the change log is. This only
// says when: soon after the page opens, when the browser says it is back
// online, and on each row from the live stream (S8), whose URL only the wasm
// module can build, since it is this device's own cursors and schema.
//
// A device with no copy yet is given one first, with ttSnapshot.bootstrap
// (S10): every event this viewer may see, since Team 10101 is the whole
// deployment (open decision 4). A page that wants a narrower copy, such as
// the grid scoped to one event, asks for it before this script's first pull
// finds none and fills in the gap itself.
//
// A pull that finds this device's build too old to read the Pi's schema asks
// for a fresh snapshot, so the next reload -- which link.js's update banner
// already offers -- opens a copy the new schema can read. It cannot otherwise
// "block the stale view": the device-made page (C5) already says it may be
// behind, and there is no live page here to block.
//
// Only where a service worker runs, which needs https (open decision 9); this
// script and snapshot.js each check that for themselves. Each pull's report
// is dispatched on document as "tt:sync", for anything that wants to show it.
(function () {
  "use strict";

  var FIRST_MS = 3 * 1000;
  var DEBOUNCE_MS = 500;

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

  var running = null;
  function pull() {
    if (running) return running;
    running = bootstrapIfNeeded()
      .then(function () {
        var bearer = window.ttToken ? window.ttToken.token() : null;
        return ask({ type: "tt-sync", bearer: bearer });
      })
      .then(function (report) {
        if (!report) return report;
        document.dispatchEvent(new CustomEvent("tt:sync", { detail: report }));
        if (report.stopped && report.stopped.kind === "reload" && window.ttSnapshot) {
          window.ttSnapshot.bootstrap({ replace: true }).catch(function () {});
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
        timer = window.setTimeout(pull, DEBOUNCE_MS);
      };
      stream.addEventListener("change", onRow);
      stream.addEventListener("upstream", onRow);
      stream.addEventListener("cursor", onRow);
      stream.onerror = function () {
        // A 409 (another schema) closes the stream for good, the same
        // trouble a pull already knows how to find and name; anything else
        // is the browser reconnecting to the same URL on its own.
        if (stream && stream.readyState === EventSource.CLOSED) {
          stream = null;
          pull();
        }
      };
    });
  }

  window.setTimeout(function () {
    pull().then(openStream);
  }, FIRST_MS);
  window.addEventListener("online", function () {
    pull().then(openStream);
  });
})();
