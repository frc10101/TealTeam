// The connection chip (I11): can THIS device reach the TealTeam server?
//
// The layout renders every state the chip can be in, one shown; this only
// picks which. The words and the rule live in tt_core::link -- this is the
// same order as Link::classify: unreachable, then sending, then synced.
//
// Offline is observed, never a mode. Nothing here can be switched on or off.
//
// The server's own internet is a separate question (the lead scout page's
// "Server's internet"); a Pi with no uplink still takes every save.
(function () {
  "use strict";

  var chip = document.getElementById("link-chip");
  if (!chip) return;

  // Check often while the server is gone, so the chip clears quickly once the
  // tablet is back on the LAN; rarely while it is there.
  var CONNECTED_MS = 30000;
  var DISCONNECTED_MS = 5000;
  var TIMEOUT_MS = 5000;

  var reachable = true; // The page itself just came from the server.
  var sending = false;
  var timer = null;
  var checking = false;

  function show() {
    var key = !reachable ? "offline" : sending ? "syncing" : "synced";
    chip.querySelectorAll("[data-link]").forEach(function (state) {
      state.hidden = state.dataset.link !== key;
    });
  }

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(check, reachable ? CONNECTED_MS : DISCONNECTED_MS);
  }

  function check() {
    // A tablet with its screen off should not poll the Pi all afternoon.
    if (checking || document.hidden) return schedule();
    checking = true;
    var abort = window.AbortController ? new AbortController() : null;
    var giveUp = setTimeout(function () { if (abort) abort.abort(); }, TIMEOUT_MS);

    // Any answer at all means the server is there; only no answer is offline.
    fetch("/health", { cache: "no-store", credentials: "same-origin", signal: abort && abort.signal })
      .then(function () { reachable = true; })
      .catch(function () { reachable = false; })
      .then(function () {
        clearTimeout(giveUp);
        checking = false;
        show();
        schedule();
      });
  }

  // navigator.onLine is only trustworthy when it says false: true means "some
  // network", not "the Pi". So going offline is believed at once, and coming
  // back is checked.
  window.addEventListener("offline", function () {
    reachable = false;
    show();
    schedule();
  });
  window.addEventListener("online", check);
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) check();
  });

  // A save is on its way until the next page replaces this one.
  document.addEventListener("submit", function (event) {
    if (event.defaultPrevented || (event.target.method || "").toLowerCase() !== "post") return;
    sending = true;
    show();
  });
  // Back to a page from the history cache: whatever was sending has landed or
  // failed, so look again.
  window.addEventListener("pageshow", function (event) {
    if (!event.persisted) return;
    sending = false;
    check();
  });

  if (navigator.onLine === false) reachable = false;
  show();
  schedule();
})();
