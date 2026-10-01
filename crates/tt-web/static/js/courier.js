// A lead scout's tablet fetches from TBA with its own signal, and hands the
// Pi what it fetched when it is back in reach (S7).
//
// The work is the service worker's: tt-client's wasm module asks the Pi for
// the key, fetches conditionally, keeps new responses in the device's copy,
// and pushes them as a bundle (S5). This only says when: soon after a page
// opens, when the browser says it is back online, when the tablet is picked
// up again, and every two minutes while the page is on screen. A tick finds
// out for itself what it can reach, so asking too often costs a request or
// two, never a duplicate.
//
// Only for a lead scout, the one role the Pi gives the key to and takes a
// bundle from: the page says so when signed in, the offline token (C9)
// otherwise. Only where a service worker runs, which is https (open
// decision 9). Each report is dispatched on document as "tt:courier".
(function () {
  "use strict";

  var EVERY_MS = 2 * 60 * 1000;
  var FIRST_MS = 5 * 1000;
  // A worker that never answers must not stop every later tick.
  var GIVE_UP_MS = 3 * 60 * 1000;

  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;
  var script = document.currentScript;
  var leadPage = !!(script && script.dataset.lead === "true");

  function isLead() {
    if (leadPage) return true;
    var claims = window.ttToken && window.ttToken.claims();
    var roles = claims && claims.roles;
    return !!(roles && (roles.is_lead_scout || roles.is_admin));
  }

  var asked = 0;
  function tick() {
    var worker = navigator.serviceWorker.controller;
    if (!worker || document.visibilityState === "hidden" || !isLead()) return;
    if (asked && Date.now() - asked < GIVE_UP_MS) return;
    asked = Date.now();
    var channel = new MessageChannel();
    channel.port1.onmessage = function (event) {
      asked = 0;
      if (event.data) document.dispatchEvent(new CustomEvent("tt:courier", { detail: event.data }));
    };
    var bearer = window.ttToken ? window.ttToken.token() : null;
    worker.postMessage({ type: "tt-courier", bearer: bearer }, [channel.port2]);
  }

  window.setTimeout(tick, FIRST_MS);
  window.setInterval(tick, EVERY_MS);
  window.addEventListener("online", tick);
  document.addEventListener("visibilitychange", tick);
})();
