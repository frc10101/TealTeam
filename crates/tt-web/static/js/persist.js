// Keep this device's copy of TealTeam's data (C2).
//
// Browsers may clear a site's storage when the device runs low on space,
// unless the site asks to keep it and the browser agrees. Chrome decides by
// itself (an installed app is almost always kept), Firefox asks the person,
// and Safari never asks: it keeps the data of a Home Screen app and clears a
// tab's after 7 days without a visit.
//
// Only in a secure context: https, or the device itself. Over the event LAN's
// plain http, navigator.storage does not exist -- see open decision 9.
//
// The account page's "This device" card shows the outcome; the words are in
// its template, and this only picks one.
(function () {
  "use strict";

  function show(state) {
    document.querySelectorAll("[data-storage]").forEach(function (el) {
      el.hidden = el.dataset.storage !== state;
    });
  }

  if (!window.isSecureContext) return show("insecure");
  var storage = navigator.storage;
  if (!storage || !storage.persisted || !storage.persist) return show("unsupported");

  // Ask once in a tab and once more after installing, when Chrome is far
  // likelier to agree. Firefox shows a prompt, which must not come back on
  // every page.
  var installed = window.matchMedia && matchMedia("(display-mode: standalone)").matches;
  var key = "tt_persist_asked_" + (installed ? "app" : "tab");

  storage
    .persisted()
    .then(function (kept) {
      if (kept) return true;
      var asked = null;
      try {
        asked = localStorage.getItem(key);
        localStorage.setItem(key, new Date().toISOString());
      } catch (e) {
        // No localStorage: asking each time is the lesser harm.
      }
      return asked ? false : storage.persist();
    })
    .then(function (kept) {
      show(kept ? "persisted" : "best-effort");
    })
    .catch(function () {
      show("unsupported");
    });

  var usage = document.querySelector("[data-storage-usage]");
  if (usage && storage.estimate) {
    storage.estimate().then(function (e) {
      if (!e.quota) return;
      var mb = function (n) { return (n / 1e6).toFixed(1) + " MB"; };
      usage.textContent = "Using " + mb(e.usage || 0) + " of about " + mb(e.quota) + " on this device.";
      usage.hidden = false;
    });
  }
})();
