// What the scouting form could not send, held on this device until the
// server answers (C13).
//
// Over the event LAN's plain http there is no service worker, OPFS, or wasm
// worker (sync.js, snapshot.js), so this is page script and localStorage,
// like draft.js and agenda.js.
//
// Saving. The form's own post goes by fetch. When the server answers, the
// page is the server's: a save (its redirect to the confirmation) is followed
// there, and a form shown again with errors is posted the ordinary way to
// show it. When nothing answers -- the network dropped, the Pi is restarting
// -- or something in between answers 502-504 for it, the entry is held: typed
// as the push wants it, kept here, and the scout is told it is held, not
// saved. A sign-in page in place of an answer (the session ran out) holds it
// too, and it goes at once with the offline token (C9).
//
// Sending. What is held goes to POST /api/sync/push when a page loads, when
// link.js sees the server again, when the browser says it is online, every
// 10 s while anything is held, and before an update reloads the page (S11).
// A "recorded" receipt clears the entry and its draft (C3): a page has no
// change log to wait on, and a reply lost on the way only sends it again,
// which its record id makes harmless (D7). A refusal stays, with the reason,
// and is not sent again; the server has kept it for the lead scout (C10).
//
// The format, which C15's downloadable page shares. localStorage key
// "tt-outbox:v1:{user id}", one per scout, so a shared tablet never pushes one
// scout's entries as another's. The value is an exported outbox, the shape
// tt_client::outbox's export_outbox writes, which the push takes as it is:
//
//   { "format": "tealteam-outbox", "version": 2, "observations": [{
//       "client_record_id", "match_key", "team_number", "payload",
//       "schema_version", "observed_at",       <- tt_core::outbox::QueuedObservation
//       "queued_at", "attempts", "tried_at",
//       "refused",                             <- the server's reason, or null
//       "draft"                                <- the C3 draft to drop once recorded
//   }] }
//
// "payload" is {field key: answer}, typed as tt_core::form::read_answers
// types a post: a counter a number, a toggle true or false, a choice or notes
// a string, and a blank left out. "schema_version" is the form's version (the
// page's tt-form), and "observed_at" the device's clock, which the server
// corrects (S12).
(function () {
  "use strict";

  var script = document.currentScript;
  var user = script ? Number(script.dataset.user) : 0;
  if (!user || !window.fetch) return;

  var KEY = "tt-outbox:v1:" + user;
  var FORMAT = "tealteam-outbox";
  var MAX_PER_PUSH = 100;
  var RETRY_MS = 10000;
  var SAVE_TIMEOUT_MS = 10000;
  var FIELD_PREFIX = "f.";
  var WIRE = ["client_record_id", "match_key", "team_number", "payload", "schema_version", "observed_at"];

  function meta(name) {
    var el = document.querySelector('meta[name="' + name + '"]');
    return el ? el.content : "";
  }

  function load() {
    try {
      var kept = JSON.parse(window.localStorage.getItem(KEY) || "null");
      if (kept && kept.format === FORMAT && Array.isArray(kept.observations)) return kept.observations;
    } catch (e) {}
    return [];
  }
  // False when the device would not keep it: full, or private browsing.
  function store(list) {
    try {
      if (list.length) {
        window.localStorage.setItem(KEY, JSON.stringify({ format: FORMAT, version: 2, observations: list }));
      } else {
        window.localStorage.removeItem(KEY);
      }
      return true;
    } catch (e) {
      return false;
    }
  }
  function waiting(list) {
    return list.filter(function (entry) {
      return !entry.refused;
    });
  }

  // 2026now_qm14 -> Q14 · 254, as handoff.js names a form.
  function label(entry) {
    var m = /_(qm|ef|qf|sf|f)(\d+)(?:m(\d+))?$/.exec(entry.match_key || "");
    var match = !m ? entry.match_key : m[1] === "qm" ? "Q" + m[2] : m[1].toUpperCase() + m[2] + (m[3] ? "-" + m[3] : "");
    return match + " · " + entry.team_number;
  }
  function when(iso) {
    return new Date(iso).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }

  // ── The chip and the card ───────────────────────────────────────────────

  var box = document.getElementById("outbox");
  // Why what is held has not gone yet: null while it simply waits for an
  // answer, or the server's word for why it would not take them.
  var why = null;
  var delivered = [];
  var noRoom = false;

  function element(tag, className, text) {
    var el = document.createElement(tag);
    if (className) el.className = className;
    if (text) el.textContent = text;
    return el;
  }

  var WHY = {
    signin: "The server wants you to sign in again before it takes them. Sign in, then open the scouting page and they go.",
    update: "The server was updated. Reload this page and they go.",
    refused: "The server would not take them just now. Trying again.",
  };

  function render() {
    var list = load();
    var held = waiting(list);
    if (window.ttLink) window.ttLink.recount();
    if (!box) return;
    box.textContent = "";

    if (noRoom) {
      box.appendChild(element("p", "alert alert-error",
        "Not saved: the server did not answer, and this tablet had no room to hold it. " +
        "Keep this page open and press Save again once the chip says Synced."));
    }
    if (held.length) {
      var note = element("div", "alert alert-warning");
      note.setAttribute("role", "status");
      note.appendChild(element("p", "", null)).appendChild(element("strong", "", "Held on this tablet, not saved."));
      note.appendChild(element("p", "", WHY[why] ||
        "The server did not answer. " + (held.length === 1 ? "It is" : "They are") +
        " sent as soon as it does; keep this page open."));
      var rows = element("ul", "");
      held.forEach(function (entry) {
        rows.appendChild(element("li", "", label(entry) + ", held at " + when(entry.queued_at)));
      });
      note.appendChild(rows);
      box.appendChild(note);
    }
    delivered.forEach(function (entry) {
      var done = element("div", "alert alert-success");
      done.setAttribute("role", "status");
      done.appendChild(element("p", "", "Saved to the server: " + label(entry) + "."));
      if (form && entry.client_record_id === form.elements.record_id.value) {
        var next = element("a", "btn btn-primary", "Continue");
        next.href = "/submission?event=" + encodeURIComponent(entry.match_key.split("_")[0]) +
          "&match=" + encodeURIComponent(entry.match_key) + "&saved=" + entry.team_number;
        done.appendChild(element("p", "", null)).appendChild(next);
      }
      box.appendChild(done);
    });
    list.forEach(function (entry) {
      if (!entry.refused) return;
      var refused = element("div", "alert alert-error");
      refused.appendChild(element("p", "", "Refused by the server: " + label(entry) + ". " + entry.refused));
      refused.appendChild(element("p", "", "A lead scout decides what happens to it."));
      var dismiss = element("button", "btn btn-secondary", "Dismiss");
      dismiss.type = "button";
      dismiss.addEventListener("click", function () {
        store(load().filter(function (e) {
          return e.client_record_id !== entry.client_record_id;
        }));
        render();
      });
      refused.appendChild(dismiss);
      box.appendChild(refused);
    });
    box.hidden = !box.firstChild;
  }

  // ── Sending ─────────────────────────────────────────────────────────────

  function wire(entry) {
    var out = {};
    WIRE.forEach(function (name) {
      out[name] = entry[name];
    });
    return out;
  }

  function forgetDraft(key) {
    if (!key) return;
    if (form && form.dataset.draft === key && window.ttDraft && window.ttDraft.forget) {
      window.ttDraft.forget();
    } else {
      try {
        window.localStorage.removeItem(key);
      } catch (e) {}
    }
  }

  // What the server said of each entry, onto what is kept now (which may
  // have gained an entry while the push was on its way).
  function settle(sent, receipts) {
    var answered = {};
    receipts.forEach(function (receipt) {
      answered[receipt.client_record_id] = receipt;
    });
    var now = new Date().toISOString();
    var tried = {};
    sent.forEach(function (entry) {
      tried[entry.client_record_id] = true;
    });
    var kept = [];
    load().forEach(function (entry) {
      var receipt = answered[entry.client_record_id];
      if (tried[entry.client_record_id]) {
        entry.attempts = (entry.attempts || 0) + 1;
        entry.tried_at = now;
      }
      if (receipt && receipt.outcome === "recorded") {
        forgetDraft(entry.draft);
        delivered.push(entry);
        return;
      }
      if (receipt && receipt.outcome === "refused") entry.refused = receipt.reason || "Refused.";
      kept.push(entry);
    });
    store(kept);
  }

  var flushing = null;
  function flush() {
    if (flushing) return flushing;
    var batch = waiting(load()).slice(0, MAX_PER_PUSH);
    if (!batch.length) return Promise.resolve();

    var headers = { "content-type": "application/json" };
    // A lapsed session is no reason to keep them (C9), but only this scout's
    // token says who they are.
    var token = window.ttToken && window.ttToken.token();
    var claims = window.ttToken && window.ttToken.claims();
    if (token && claims && Number(claims.sub) === user) headers.authorization = "Bearer " + token;

    if (window.ttLink) window.ttLink.sending(true);
    var more = false;
    flushing = fetch("/api/sync/push?schema=" + encodeURIComponent(meta("tt-schema")), {
      method: "POST",
      headers: headers,
      body: JSON.stringify({ version: 2, observations: batch.map(wire) }),
      credentials: "same-origin",
      redirect: "manual",
      cache: "no-store",
    })
      .then(
        function (response) {
          if (response.ok) {
            return response.json().then(function (reply) {
              settle(batch, reply.receipts || []);
              why = null;
              more = waiting(load()).length > 0 && batch.length === MAX_PER_PUSH;
            });
          }
          settle(batch, []);
          why = response.status === 401 ? "signin" : response.status === 409 ? "update" : "refused";
        },
        function () {
          why = null;
          if (window.ttLink) window.ttLink.check();
        }
      )
      .catch(function () {})
      .then(function () {
        flushing = null;
        if (window.ttLink) window.ttLink.sending(false);
        render();
        if (more) return flush();
      });
    return flushing;
  }

  // ── Holding the form ────────────────────────────────────────────────────

  var form = document.getElementById("scout-form");

  function entryOf() {
    var payload = {};
    Array.prototype.forEach.call(form.elements, function (el) {
      if (!el.name || el.name.indexOf(FIELD_PREFIX) !== 0 || el.disabled) return;
      var key = el.name.slice(FIELD_PREFIX.length);
      if (el.type === "checkbox") {
        payload[key] = el.checked;
      } else if (el.type === "radio") {
        if (el.checked) payload[key] = el.value;
      } else if (el.type === "number") {
        var n = el.value.trim();
        // Not a whole number goes as typed, for the server to refuse with why.
        if (n !== "") payload[key] = /^-?\d+$/.test(n) ? Number(n) : n;
      } else {
        var text = el.value.replace(/\r\n/g, "\n").trim();
        if (text) payload[key] = text;
      }
    });
    var now = new Date().toISOString();
    return {
      client_record_id: form.elements.record_id.value,
      match_key: form.elements.match.value,
      team_number: Number(form.elements.team.value),
      payload: payload,
      schema_version: Number(meta("tt-form")),
      observed_at: now,
      queued_at: now,
      attempts: 0,
      tried_at: null,
      refused: null,
      draft: form.dataset.draft || null,
    };
  }

  function hold(entry, reason) {
    // The same form saved twice is one entry, the newer answers.
    var list = load().filter(function (e) {
      return e.client_record_id !== entry.client_record_id;
    });
    list.push(entry);
    noRoom = !store(list);
    why = reason || null;
    delivered = delivered.filter(function (e) {
      return e.client_record_id !== entry.client_record_id;
    });
    render();
    if (box && !box.hidden) box.scrollIntoView({ block: "start" });
    if (reason === "signin") flush();
    else if (window.ttLink) window.ttLink.check();
  }

  if (form) {
    var posting = false;
    form.addEventListener("submit", function (event) {
      if (event.defaultPrevented) return;
      event.preventDefault();
      if (posting) return;
      posting = true;
      var entry = entryOf();
      var abort = window.AbortController ? new AbortController() : null;
      var giveUp = setTimeout(function () {
        if (abort) abort.abort();
      }, SAVE_TIMEOUT_MS);
      if (window.ttLink) window.ttLink.sending(true);
      var away = false;
      fetch(form.action, {
        method: "POST",
        body: new URLSearchParams(new FormData(form)),
        credentials: "same-origin",
        redirect: "follow",
        signal: abort && abort.signal,
      })
        .then(
          function (response) {
            // OF1: no server, and the service worker saved it into the
            // device's outbox instead. Its confirmation is made there too.
            var saved = response.headers.get("x-tealteam-saved");
            if (saved) {
              away = true;
              return window.location.assign(saved);
            }
            var path = new URL(response.url, window.location.href).pathname;
            if (response.redirected && path === "/sign-in") return hold(entry, "signin");
            if (response.status >= 502 && response.status <= 504) return hold(entry);
            away = true;
            if (response.redirected && response.ok) return window.location.assign(response.url);
            // The server's own answer, most likely the form with what is
            // wrong with it: post it the ordinary way to show it.
            HTMLFormElement.prototype.submit.call(form);
          },
          function () {
            hold(entry);
          }
        )
        .then(function () {
          clearTimeout(giveUp);
          if (away) return;
          posting = false;
          if (window.ttLink) window.ttLink.sending(false);
        });
    });
  }

  // ── When to send ────────────────────────────────────────────────────────

  window.ttOutbox = {
    entries: load,
    flush: flush,
    // The outbox as a file, in tt_client's export shape.
    exportText: function () {
      var list = load();
      if (!list.length) return null;
      return JSON.stringify({
        format: FORMAT,
        version: 2,
        exported_at: new Date().toISOString(),
        schema: Number(meta("tt-schema")) || null,
        observations: list,
      }, null, 2);
    },
  };

  render();
  flush();
  window.addEventListener("online", flush);
  document.addEventListener("tt:reachable", flush);
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) flush();
  });
  // S11: send what is held before the page reloads into a new build.
  document.addEventListener("tt:before-update", function (event) {
    if (event.detail && event.detail.waitFor) event.detail.waitFor.push(flush());
  });
  setInterval(function () {
    if (!document.hidden && waiting(load()).length) flush();
  }, RETRY_MS);
})();
