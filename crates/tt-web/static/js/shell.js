// Install the service worker (C1), which keeps a copy of the offline page so a
// reload with no server shows that instead of the browser's error.
//
// Only in a secure context -- https, or the device itself. Over plain http a
// browser has no service workers at all, so this does nothing, says nothing,
// and the site works exactly as it did (open decision 9).
(function () {
  "use strict";

  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;
  navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(function () {
    // The site works without it; nothing to tell a scout.
  });
})();
