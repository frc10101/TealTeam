// The downloadable scouting page (C15): a scout's form, assignments and
// offline token in one file, opened from the tablet with no server.
//
// GET /submission/download writes the file with this script inside it,
// first, then counter.js, draft.js, and the QR handoff's scripts (qr.js,
// scan.js, hmac.js, handoff.js) unchanged. The page's data is the JSON in
// #tt-file-data: the Pi's address, who and where, the event's matches and
// teams, this scout's assignments, and the token and device key (T1).
//
// It runs in the file's own origin, so nothing here is shared with the
// pages the Pi serves. Like them it is page script and localStorage only:
// no service worker, no OPFS, nothing that needs a secure context.
//
// Which robot is in the address, #m=<match key>&t=<team>; choosing another
// reloads the file on it, so draft.js and counter.js meet one form each
// time. With none, the first assignment not saved here opens, else the
// schedule.
//
// Saving puts the entry in the outbox, C13's format and key
// ("tt-outbox:v1:{user}", tt_client's export shape), and keeps the draft
// (C3) the handoff reads, as outbox.js does when the Pi is out of reach.
// What is held goes to the Pi's POST /api/sync/push with "Authorization:
// Bearer <token>~<key>" whenever it answers: on load, every 15 s while
// anything is held, when the browser says it is online, and on Send now.
// A recorded receipt drops the entry and its draft; a refusal stays with
// the reason, and the Pi keeps its own copy for the lead (C10). With no Pi
// in reach the same drafts go by QR (handoff.js, S13), or as a file (Save
// them as a file) that a lead uploads on the scan page: a HandoffBody with
// a MAC by the device key for "file" (tt_core::qr::FormsFile). Either
// road's receipt, scanned, lets go of the drafts it answers; the outbox
// entries then go once more, which their record ids make harmless.
//
// window.ttToken stands in for token.js: the same five calls, from the
// token and key in the file, while the token lasts.
(function () {
  "use strict";

  var holder = document.getElementById("tt-file-data");
  if (!holder) return;
  var data = JSON.parse(holder.textContent);
  var USER = data.user;
  var OUTBOX = "tt-outbox:v1:" + USER;
  var SAVED_HERE = "tt-file-saved:v1:" + USER + ":" + data.event;
  var DRAFT_PREFIX = "tt-draft:v1:";
  var SHOWN = "tt-handoff:v1:";
  var FORMAT = "tealteam-outbox";
  var MAX_PER_PUSH = 100;
  var RETRY_MS = 15000;
  var PUSH_TIMEOUT_MS = 10000;
  var FIELD_PREFIX = "f.";
  var WIRE = ["client_record_id", "match_key", "team_number", "payload", "schema_version", "observed_at"];

  function $(selector) {
    return document.querySelector(selector);
  }
  function element(tag, className, text) {
    var el = document.createElement(tag);
    if (className) el.className = className;
    if (text) el.textContent = text;
    return el;
  }
  function read(key) {
    try {
      return JSON.parse(window.localStorage.getItem(key) || "null");
    } catch (e) {
      return null;
    }
  }
  function write(key, value) {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
      return true;
    } catch (e) {
      return false;
    }
  }
  function remove(key) {
    try {
      window.localStorage.removeItem(key);
    } catch (e) {}
  }

  // ── The token, as token.js keeps it ────────────────────────────────────

  var HEADER = "v4.public.";
  var SIGNATURE_BYTES = 64;
  function bytesOf(base64url) {
    var binary = window.atob(base64url.replace(/-/g, "+").replace(/_/g, "/"));
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }
  var claims = null;
  try {
    var body = bytesOf(data.token.slice(HEADER.length).split(".")[0]);
    claims = JSON.parse(new TextDecoder().decode(body.subarray(0, body.length - SIGNATURE_BYTES)));
  } catch (e) {}
  function live() {
    return !!claims && Date.parse(claims.exp) > Date.now();
  }
  window.ttToken = {
    token: function () {
      return live() ? data.token + "~" + data.key : null;
    },
    claims: function () {
      return live() ? claims : null;
    },
    key: function () {
      return live() ? bytesOf(data.key) : null;
    },
    holder: function () {
      return live() ? { sub: claims.sub, dev: claims.dev, ep: claims.ep, iat: claims.iat } : null;
    },
  };

  // ── The schedule ───────────────────────────────────────────────────────

  var byKey = {};
  data.matches.forEach(function (m) {
    byKey[m.key] = m;
  });
  function station(m, team) {
    var at = m.red.indexOf(team);
    if (at >= 0) return "Red " + (at + 1);
    at = m.blue.indexOf(team);
    return at >= 0 ? "Blue " + (at + 1) : "";
  }
  function teamName(team) {
    return data.teams[String(team)] || "";
  }
  function savedHere() {
    return read(SAVED_HERE) || [];
  }
  function done(key, team) {
    var pk = key + ":" + team;
    return savedHere().indexOf(pk) >= 0 || data.recorded.indexOf(pk) >= 0;
  }
  function href(key, team) {
    return "#m=" + encodeURIComponent(key) + "&t=" + team;
  }
  function go(to) {
    window.location.hash = to;
    window.location.reload();
  }
  function label(entry) {
    var m = byKey[entry.match_key];
    return (m ? m.label : entry.match_key) + " · " + entry.team_number;
  }

  // ── Which robot ────────────────────────────────────────────────────────

  var target = null;
  var hash = /^#m=([^&]+)&t=(\d+)$/.exec(window.location.hash);
  if (hash) {
    var m = byKey[decodeURIComponent(hash[1])];
    var team = Number(hash[2]);
    if (m && station(m, team)) target = { match: m, team: team };
  }
  var choosing = window.location.hash === "#choose";
  if (!target && !choosing) {
    data.duties.some(function (duty) {
      if (done(duty[0], duty[1]) || !byKey[duty[0]]) return false;
      target = { match: byKey[duty[0]], team: duty[1] };
      return true;
    });
  }

  var form = document.getElementById("scout-form");

  function uuidv7() {
    var b = new Uint8Array(16);
    window.crypto.getRandomValues(b);
    var ms = Date.now();
    for (var i = 5; i >= 0; i--) {
      b[i] = ms % 256;
      ms = Math.floor(ms / 256);
    }
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    var hex = Array.prototype.map
      .call(b, function (x) {
        return (x < 16 ? "0" : "") + x.toString(16);
      })
      .join("");
    return [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join("-");
  }

  if (target) {
    var key = target.match.key;
    var draftKey = [DRAFT_PREFIX + USER, data.event, key, target.team, data.form].join(":");
    var kept = read(draftKey);
    form.elements.match.value = key;
    form.elements.team.value = String(target.team);
    // The draft's id, so saving again is the same observation (D7).
    form.elements.record_id.value = (kept && kept.recordId) || uuidv7();
    form.dataset.draft = draftKey;
    form.hidden = false;
    var card = $("#file-target");
    card.querySelector("[data-file-team]").textContent = target.team;
    card.querySelector("[data-file-team-name]").textContent = teamName(target.team);
    card.querySelector("[data-file-where]").textContent =
      target.match.label + " · " + station(target.match, target.team) + (target.match.time ? " · " + target.match.time : "");
    card.querySelector("[data-file-recorded]").hidden = !done(key, target.team);
    card.hidden = false;
  } else {
    form.remove();
    form = null;
  }

  document.querySelectorAll("[data-file-choose]").forEach(function (link) {
    link.addEventListener("click", function (event) {
      event.preventDefault();
      go("choose");
    });
  });

  // The schedule, when no robot is open.
  var picker = $("#file-picker");
  if (!target) {
    var select = $("#file-match");
    var robots = picker.querySelector("[data-file-robots]");
    data.matches.forEach(function (m) {
      var option = element("option", "", m.label + (m.time ? " · " + m.time : ""));
      option.value = m.key;
      select.appendChild(option);
    });
    var open = data.duties.filter(function (duty) {
      return !done(duty[0], duty[1]);
    })[0];
    if (open && byKey[open[0]]) select.value = open[0];
    var drawRobots = function () {
      var m = byKey[select.value];
      robots.textContent = "";
      ["red", "blue"].forEach(function (color) {
        var list = element("ul", "alliance " + color);
        list.setAttribute("aria-label", color + " alliance");
        m[color].forEach(function (team, i) {
          var item = element("li");
          var slot = color === "red" ? "Red " : "Blue ";
          if (!team) {
            var empty = element("span", "robot empty");
            empty.appendChild(element("span", "robot-station", slot + (i + 1)));
            empty.appendChild(element("strong", "", "—"));
            item.appendChild(empty);
          } else {
            var link = element("a", "robot");
            link.href = href(m.key, team);
            link.appendChild(element("span", "robot-station", slot + (i + 1)));
            link.appendChild(element("strong", "", String(team)));
            if (done(m.key, team)) link.appendChild(element("span", "robot-done", "Saved"));
            link.addEventListener("click", function (event) {
              event.preventDefault();
              go(link.getAttribute("href").slice(1));
            });
            item.appendChild(link);
          }
          list.appendChild(item);
        });
        robots.appendChild(list);
      });
    };
    select.addEventListener("change", drawRobots);
    drawRobots();
    picker.hidden = false;
  }

  // The assignments, ticked as they are saved.
  function drawDuties() {
    var list = $("[data-file-duties]");
    list.textContent = "";
    $("[data-file-no-duties]").hidden = data.duties.length > 0;
    data.duties.forEach(function (duty) {
      var m = byKey[duty[0]];
      if (!m) return;
      var item = element("li");
      var link = element("a", "duty");
      link.href = href(m.key, duty[1]);
      if (target && target.match.key === m.key && target.team === duty[1]) link.setAttribute("aria-current", "true");
      var when = element("span", "duty-when");
      when.appendChild(element("strong", "", (done(m.key, duty[1]) ? "✓ " : "") + m.label + " · " + station(m, duty[1])));
      if (m.time) when.appendChild(element("small", "", m.time));
      var who = element("span", "duty-team");
      who.appendChild(element("strong", "", String(duty[1])));
      if (teamName(duty[1])) who.appendChild(element("small", "", teamName(duty[1])));
      link.appendChild(when);
      link.appendChild(who);
      link.addEventListener("click", function (event) {
        event.preventDefault();
        go(link.getAttribute("href").slice(1));
      });
      item.appendChild(link);
      list.appendChild(item);
    });
  }
  drawDuties();

  // ── The outbox ─────────────────────────────────────────────────────────

  function load() {
    var kept = read(OUTBOX);
    return kept && kept.format === FORMAT && Array.isArray(kept.observations) ? kept.observations : [];
  }
  function store(list) {
    if (!list.length) {
      remove(OUTBOX);
      return true;
    }
    return write(OUTBOX, { format: FORMAT, version: 1, observations: list });
  }
  function waiting(list) {
    return list.filter(function (entry) {
      return !entry.refused;
    });
  }

  var box = document.getElementById("outbox");
  var why = null;
  var delivered = [];
  var justSaved = null;
  var noRoom = false;
  var linkNote = $("#file-link");
  var status = $("[data-file-status]");

  function reach(up, text) {
    linkNote.dataset.state = up ? "up" : "down";
    linkNote.textContent = text;
  }

  var WHY = {
    token:
      "The server would not take this page's sign-in: it has run out, or you were signed out everywhere. " +
      "Hand them to a lead scout by code or as a file, and download a fresh page.",
    refused: "The server would not take them just now. Trying again.",
  };

  function render() {
    var list = load();
    var held = waiting(list);
    box.textContent = "";
    if (noRoom) {
      box.appendChild(element("p", "alert alert-error", "Not saved: this tablet had no room to keep it. Free some space and press Save again."));
    }
    if (justSaved) {
      var saved = element("div", "alert alert-success");
      saved.setAttribute("role", "status");
      saved.appendChild(element("p", "", "Saved " + label(justSaved) + " on this tablet."));
      var next = data.duties.filter(function (duty) {
        return !done(duty[0], duty[1]) && byKey[duty[0]];
      })[0];
      var action = element("a", "btn btn-primary", next ? "Scout " + byKey[next[0]].label + " · " + next[1] : "Choose the next robot");
      action.href = next ? href(next[0], next[1]) : "#choose";
      action.addEventListener("click", function (event) {
        event.preventDefault();
        go(action.getAttribute("href").slice(1));
      });
      saved.appendChild(element("p")).appendChild(action);
      box.appendChild(saved);
    }
    if (held.length) {
      var note = element("div", "alert alert-warning");
      note.setAttribute("role", "status");
      note.appendChild(element("p")).appendChild(element("strong", "", "Held on this tablet, not on the server yet."));
      note.appendChild(element("p", "", WHY[why] || (held.length === 1 ? "It goes" : "They go") + " to the server as soon as this tablet reaches it."));
      var rows = element("ul");
      held.forEach(function (entry) {
        rows.appendChild(element("li", "", label(entry)));
      });
      note.appendChild(rows);
      box.appendChild(note);
    }
    delivered.forEach(function (entry) {
      box.appendChild(element("p", "alert alert-success", "On the server: " + label(entry) + "."));
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
    drawDuties();
  }

  function forgetDraft(key) {
    if (!key) return;
    if (form && form.dataset.draft === key && window.ttDraft && window.ttDraft.forget) window.ttDraft.forget();
    else remove(key);
  }

  function settle(sent, receipts) {
    var answered = {};
    receipts.forEach(function (receipt) {
      answered[receipt.client_record_id] = receipt;
    });
    var tried = {};
    sent.forEach(function (entry) {
      tried[entry.client_record_id] = true;
    });
    var now = new Date().toISOString();
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
    var credential = window.ttToken.token();
    if (!credential) {
      why = "token";
      render();
      return Promise.resolve();
    }
    var abort = window.AbortController ? new AbortController() : null;
    var giveUp = setTimeout(function () {
      if (abort) abort.abort();
    }, PUSH_TIMEOUT_MS);
    var more = false;
    // From a file: no cookie of the Pi's goes, and none is wanted. The
    // credential says who, and the Pi takes it as from the tablet it names.
    flushing = fetch(data.pi + "/api/sync/push", {
      method: "POST",
      mode: "cors",
      credentials: "omit",
      cache: "no-store",
      headers: { "content-type": "application/json", authorization: "Bearer " + credential },
      body: JSON.stringify({ observations: batch.map(wire) }),
      signal: abort && abort.signal,
    })
      .then(
        function (response) {
          reach(true, "The server is in reach.");
          if (response.ok) {
            return response.json().then(function (reply) {
              settle(batch, reply.receipts || []);
              why = null;
              more = waiting(load()).length > 0 && batch.length === MAX_PER_PUSH;
            });
          }
          settle(batch, []);
          why = response.status === 401 ? "token" : "refused";
        },
        function () {
          why = null;
          reach(false, "The server is out of reach. What you save is kept on this tablet.");
        }
      )
      .catch(function () {})
      .then(function () {
        clearTimeout(giveUp);
        flushing = null;
        render();
        if (more) return flush();
      });
    return flushing;
  }

  function wire(entry) {
    var out = {};
    WIRE.forEach(function (name) {
      out[name] = entry[name];
    });
    return out;
  }

  // Whether the Pi answers, when there is nothing to send: an opaque
  // answer is still an answer.
  function probe() {
    fetch(data.pi + "/health", { mode: "no-cors", credentials: "omit", cache: "no-store" }).then(
      function () {
        reach(true, "The server is in reach.");
      },
      function () {
        reach(false, "The server is out of reach. What you save is kept on this tablet.");
      }
    );
  }

  // ── Saving ─────────────────────────────────────────────────────────────

  // Typed as tt_core::form::read_answers types a post, as outbox.js does.
  function entryOf() {
    var payload = {};
    Array.prototype.forEach.call(form.elements, function (el) {
      if (!el.name || el.name.indexOf(FIELD_PREFIX) !== 0 || el.disabled) return;
      var name = el.name.slice(FIELD_PREFIX.length);
      if (el.type === "checkbox") {
        payload[name] = el.checked;
      } else if (el.type === "radio") {
        if (el.checked) payload[name] = el.value;
      } else if (el.type === "number") {
        var n = el.value.trim();
        if (n !== "") payload[name] = /^-?\d+$/.test(n) ? Number(n) : n;
      } else {
        var text = el.value.replace(/\r\n/g, "\n").trim();
        if (text) payload[name] = text;
      }
    });
    var now = new Date().toISOString();
    return {
      client_record_id: form.elements.record_id.value,
      match_key: form.elements.match.value,
      team_number: Number(form.elements.team.value),
      payload: payload,
      schema_version: Number(data.form),
      observed_at: now,
      queued_at: now,
      attempts: 0,
      tried_at: null,
      refused: null,
      draft: form.dataset.draft || null,
    };
  }

  if (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var entry = entryOf();
      var list = load().filter(function (e) {
        return e.client_record_id !== entry.client_record_id;
      });
      list.push(entry);
      noRoom = !store(list);
      if (!noRoom) {
        var pk = entry.match_key + ":" + entry.team_number;
        var here = savedHere();
        if (here.indexOf(pk) < 0) here.push(pk);
        write(SAVED_HERE, here);
        justSaved = entry;
        form.hidden = true;
        $("#file-target").hidden = true;
      }
      render();
      box.scrollIntoView({ block: "start" });
      flush();
    });
  }

  // ── As a file ──────────────────────────────────────────────────────────

  // The scout's drafts they pressed Save on, as handoff.js hands them over.
  function savedForms() {
    var out = [];
    try {
      for (var i = 0; i < window.localStorage.length; i++) {
        var key = window.localStorage.key(i);
        if (!key || key.indexOf(DRAFT_PREFIX + USER + ":") !== 0) continue;
        var parts = key.split(":");
        var draft = read(key);
        if (parts.length !== 7 || !draft || !draft.recordId || !draft.answers || !draft.tried) continue;
        out.push({
          record_id: draft.recordId,
          match: draft.match,
          team: draft.team,
          form_version: Number(parts[6]),
          saved_at: draft.savedAt,
          answers: draft.answers,
        });
      }
    } catch (e) {}
    return out;
  }

  function nonce() {
    var bytes = new Uint8Array(16);
    window.crypto.getRandomValues(bytes);
    return window.ttHmac.toBase64Url(bytes);
  }

  $("[data-file-export]").addEventListener("click", function () {
    if (window.ttDraft) window.ttDraft.save();
    var who = window.ttToken.holder();
    var key = window.ttToken.key();
    if (!who || !key || !window.ttHmac) {
      status.textContent = "This page's sign-in has run out, so a file from it would be refused. Download a fresh page while the server is in reach.";
      return;
    }
    var forms = savedForms();
    if (!forms.length) {
      status.textContent = "Nothing saved on this tablet is waiting to be sent.";
      return;
    }
    var shown = { nonce: nonce(), at: Date.now(), key: data.key, forms: {} };
    forms.forEach(function (f) {
      var text = JSON.stringify([f.match, f.team, f.answers]);
      shown.forms[f.record_id] = window.ttHmac.toBase64Url(window.ttHmac.sha256(new TextEncoder().encode(text)));
    });
    var body = JSON.stringify({ scout: who, nonce: shown.nonce, at: shown.at, forms: forms });
    var file = { format: "tealteam-forms", version: 1, body: body, mac: window.ttHmac.mac(key, "file", body) };
    // The lead's receipt for this file, scanned under the handoff panel,
    // settles these forms as one for the codes would.
    write(SHOWN + who.sub, shown);
    var blob = new Blob([JSON.stringify(file, null, 2)], { type: "application/json" });
    var link = document.createElement("a");
    var stamp = new Date().toISOString().slice(0, 16).replace(/[-:T]/g, "");
    link.href = URL.createObjectURL(blob);
    link.download = "tealteam-forms-" + data.event + "-" + USER + "-" + stamp + ".json";
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(function () {
      URL.revokeObjectURL(link.href);
    }, 10000);
    status.textContent =
      (forms.length === 1 ? "1 form" : forms.length + " forms") +
      " saved as a file. A lead scout uploads it on Scan a tablet's forms; it is good until this page's sign-in runs out.";
  });

  $("[data-file-send]").addEventListener("click", function () {
    status.textContent = waiting(load()).length ? "Sending…" : "Nothing is waiting to be sent.";
    flush().then(function () {
      if (waiting(load()).length) status.textContent = "";
      else if (delivered.length) status.textContent = "Everything saved here is on the server.";
    });
  });

  if (!live()) {
    reach(false, "This page's sign-in has run out. Download a fresh page while the server is in reach.");
  }
  render();
  if (waiting(load()).length) flush();
  else probe();
  window.addEventListener("online", function () {
    flush();
  });
  setInterval(function () {
    if (!document.hidden && waiting(load()).length) flush();
  }, RETRY_MS);
})();
