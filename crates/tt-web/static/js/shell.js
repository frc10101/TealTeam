// Install the service worker (C1), which keeps a copy of the offline page so a
// reload with no server shows that instead of the browser's error.
//
// Only in a secure context -- https, or the device itself. Over plain http a
// browser has no service workers at all, so this does nothing, says nothing,
// and the site works exactly as it did (open decision 9).
//
// A device too full for the copy (Q6) is told so: the layout renders the
// warnings hidden, and this shows the one that applies. With room for only
// the offline page and its files, the worker keeps those and marks the cache
// (src/sw.js); this adds the rest when it next fits. With room for nothing,
// the install fails and the worker says so; the next page tries again.
(function () {
  "use strict";

  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;

  var PARTIAL = "/offline?partial";
  var RETRY_KEY = "tt-shell-retry";
  var RETRY_MS = 5 * 60 * 1000;

  function warn(state) {
    document.querySelectorAll("[data-shell-full]").forEach(function (el) {
      el.hidden = el.dataset.shellFull !== state;
    });
  }

  // Only running out of space is the scout's to fix. Any other failure (the
  // server restarting mid-install) is retried by the next page, unannounced.
  navigator.serviceWorker.addEventListener("message", function (event) {
    var data = event.data || {};
    if (data.type === "tt-shell" && data.state === "none" && data.reason === "QuotaExceededError") warn("none");
  });
  navigator.serviceWorker.startMessages();

  // Not on every page: on a full device each try downloads the rest only to
  // be refused. localStorage may be full too, and then it just tries.
  function due() {
    try {
      var last = Number(localStorage.getItem(RETRY_KEY));
      if (last && Date.now() - last < RETRY_MS) return false;
      localStorage.setItem(RETRY_KEY, String(Date.now()));
    } catch (e) {}
    return true;
  }

  // A cache the worker could only half fill: warn, and try to finish it.
  function partial() {
    if (!window.caches) return;
    caches
      .keys()
      .then(function (names) {
        var mine = names.filter(function (name) { return name.indexOf("tealteam-shell-") === 0; });
        return Promise.all(
          mine.map(function (name) {
            return caches.open(name).then(function (cache) {
              return cache.match(PARTIAL).then(function (mark) {
                return mark && mark.json().then(function (why) { return { cache: cache, why: why }; });
              });
            });
          }),
        );
      })
      .then(function (found) {
        var half = found.filter(Boolean)[0];
        if (!half) return warn("");
        if (half.why.reason === "QuotaExceededError") warn("partial");
        if (!due()) return;
        var rest = (half.why.rest || []).map(function (url) { return new Request(url, { cache: "reload" }); });
        return half.cache
          .addAll(rest)
          .then(function () { return half.cache.delete(PARTIAL); })
          .then(function () { warn(""); });
      })
      .catch(function () {
        // Still no room; the warning stays.
      });
  }

  navigator.serviceWorker
    .register("/sw.js", { scope: "/" })
    .then(function (registration) {
      var worker = registration.installing || registration.waiting;
      if (!worker) return partial();
      worker.addEventListener("statechange", function () {
        if (worker.state === "activated") partial();
      });
    })
    .catch(function () {
      // The site works without it; nothing to tell a scout.
    });
})();
