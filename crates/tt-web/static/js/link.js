// The connection chip (I11): can THIS device reach the TealTeam server?
//
// The layout renders every state the chip can be in, one shown; this only
// picks which. The words and the rule live in tt_core::link -- this is the
// same order as Link::classify: unreachable, then sending, then what needs
// review, then synced. The review count is the server's, as of this page
// (C10b); only a page that has it renders that state.
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

  // The page itself just came from the server -- unless it is the offline
  // shell (C1), which the service worker served from its cache because the
  // server did not answer, or a page the worker made from the device's copy
  // (C5). Those check at once, and reload the moment the server is back, so
  // the scout gets the server's page. At /offline itself there is nothing
  // better to reload into, and doing so would reload forever.
  var shell =
    (!!document.getElementById("offline-shell") || !!document.getElementById("device-page")) &&
    location.pathname !== "/offline";
  var reachable = !shell;
  var sending = false;
  var review = !!chip.querySelector('[data-link="review"]');
  var timer = null;
  var checking = false;

  function show() {
    var key = !reachable ? "offline" : sending ? "syncing" : review ? "review" : "synced";
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
      .then(function (response) {
        reachable = true;
        if (shell) return location.reload();
        return response.json().then(compare, function () {});
      })
      .catch(function () { reachable = false; })
      .then(function () {
        clearTimeout(giveUp);
        checking = false;
        show();
        schedule();
      });
  }

  // S11: the page's own versions, from the layout's <meta> tags.
  function stamp(name) {
    var meta = document.querySelector('meta[name="' + name + '"]');
    return meta ? meta.content : "";
  }
  var built = { build: stamp("tt-build"), schema: stamp("tt-schema"), form: stamp("tt-form") };

  // A server running another build than the one this page came from: a
  // deploy happened. Block the page, and reload into the new version once
  // everything unsaved is kept. Listeners of tt:before-update may add
  // promises to detail.waitFor -- the outbox (C7) will, to push first.
  var updating = false;
  function compare(health) {
    if (updating || !built.build || !health || !health.build) return;
    // The form is compared too: a new season file changes what the scouting
    // form asks without changing a template.
    if (
      health.build === built.build &&
      String(health.schema) === built.schema &&
      String(health.form) === built.form
    ) return;
    updating = true;

    var banner = document.getElementById("update-banner");
    if (!banner) return location.reload();
    var formChanged = String(health.form) !== built.form;
    banner.querySelector("[data-update-form]").hidden = !formChanged;
    // Nothing under the banner can be reached, by pointer or by keyboard.
    Array.prototype.forEach.call(document.body.children, function (el) {
      if (el !== banner) el.inert = true;
    });
    banner.hidden = false;
    var button = banner.querySelector("[data-update-reload]");
    button.focus();
    button.addEventListener("click", function () {
      button.disabled = true;
      var before = new CustomEvent("tt:before-update", { detail: { waitFor: [] } });
      document.dispatchEvent(before);
      var patience = new Promise(function (done) { setTimeout(done, 5000); });
      Promise.race([Promise.allSettled(before.detail.waitFor), patience]).then(function () {
        location.reload();
      });
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
  if (shell) check();
  else schedule();
})();
