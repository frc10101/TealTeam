// The connection chip (I11): can THIS device reach the TealTeam server?
//
// The layout renders every state the chip can be in, one shown; this only
// picks which. The words and the rule live in tt_core::link -- this is the
// same order as Link::classify: unreachable, then sending, then what needs
// review, then synced. The review count is the server's, as of this page
// (C10b); only a page that has it renders that state. What is held is the
// device's, counted from where it is kept, on every page -- the server's,
// one the worker made, and the offline shell alike (OF1): the page outboxes
// in localStorage (C13, outbox.js; every scout's, since the shell knows no
// one), and the device's outbox in its copy (C7), which only the worker can
// read. The chip reads "Offline · n held" while the server is gone, or
// Syncing while it is there and they are on their way.
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
  var held = 0;
  // What each queue holds: "local" is localStorage's, "device" the worker's.
  var counts = { local: 0, device: 0 };
  var timer = null;
  var checking = false;

  // Every scout's waiting entries in the page outboxes (outbox.js's format).
  function localHeld() {
    var n = 0;
    try {
      for (var i = 0; i < localStorage.length; i++) {
        var name = localStorage.key(i);
        if (!name || name.indexOf("tt-outbox:v1:") !== 0) continue;
        var kept = JSON.parse(localStorage.getItem(name) || "null");
        if (!kept || !Array.isArray(kept.observations)) continue;
        kept.observations.forEach(function (entry) {
          if (!entry.refused) n++;
        });
      }
    } catch (e) {}
    return n;
  }

  // The device's outbox, which only the worker reads. 0 without one.
  function deviceHeld() {
    var worker = navigator.serviceWorker && navigator.serviceWorker.controller;
    if (!worker) return Promise.resolve(0);
    return new Promise(function (resolve) {
      var channel = new MessageChannel();
      var late = setTimeout(function () { resolve(counts.device); }, 3000);
      channel.port1.onmessage = function (event) {
        clearTimeout(late);
        resolve(Number(event.data) || 0);
      };
      worker.postMessage({ type: "tt-queued" }, [channel.port2]);
    });
  }

  function recount() {
    counts.local = localHeld();
    show();
    return deviceHeld().then(function (n) {
      counts.device = n;
      show();
    });
  }

  function show() {
    held = counts.local + counts.device;
    var key = !reachable
      ? held ? "held" : "offline"
      : sending || held ? "syncing" : review ? "review" : "synced";
    var counted = chip.querySelector('[data-link="held"]');
    if (counted) {
      var label = counted.dataset.label.replace("{n}", held);
      var meaning = counted.dataset.meaning.replace("{n}", held);
      counted.querySelector("[data-link-label]").textContent = label;
      counted.querySelector("[data-link-meaning]").textContent = meaning;
      counted.title = meaning;
    }
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
    var was = reachable;
    fetch("/health", { cache: "no-store", credentials: "same-origin", signal: abort && abort.signal })
      .then(function (response) {
        reachable = true;
        // Back: what the outboxes hold can go now.
        if (!was) document.dispatchEvent(new Event("tt:reachable"));
        if (shell) return location.reload();
        return response.json().then(compare, function () {});
      })
      .catch(function () { reachable = false; })
      .then(function () {
        clearTimeout(giveUp);
        checking = false;
        recount();
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

  // For the outboxes (C13, C7): count again after a change, how many the
  // device's holds as a sync just found, a save sending itself (its submit is
  // prevented, so the listener above does not see it), and a save that got
  // no answer, which is worth a look at once.
  window.ttLink = {
    recount: recount,
    held: function (n, source) {
      counts[source || "device"] = Number(n) || 0;
      show();
    },
    sending: function (on) {
      sending = on;
      show();
    },
    check: check,
  };

  // Another tab changed a page outbox; the worker saved a form (OF1).
  window.addEventListener("storage", function (event) {
    if (!event.key || event.key.indexOf("tt-outbox:v1:") === 0) recount();
  });
  if (navigator.serviceWorker) {
    navigator.serviceWorker.addEventListener("message", function (event) {
      if (event.data && event.data.type === "tt-queued") recount();
    });
  }

  if (navigator.onLine === false) reachable = false;
  recount();
  if (shell) check();
  else schedule();
})();
