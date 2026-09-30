// Unsaved scouting answers survive a reload, a crash, or a flat battery (C3).
//
// Every change to the form is written to localStorage, a moment after the
// last one, under the key the server put on the form: one scout, one event,
// one match, one robot, one version of the form. So a draft can only ever
// come back into its own form. On load a draft that differs from what the
// server rendered is put back, with a note saying so and a way to throw it
// away. The server names the draft to delete once a save is confirmed.
//
// Kept simple on purpose: the outbox (C7) will replace this storage.
(function () {
  "use strict";

  var PREFIX = "tt-draft:v1:";
  var DEBOUNCE_MS = 400;
  var MAX_AGE_MS = 3 * 24 * 60 * 60 * 1000;
  // Which form this is, not answers in it.
  var IDENTITY = { match: true, team: true, record_id: true };

  function read(key) {
    try {
      var raw = window.localStorage.getItem(key);
      return raw ? JSON.parse(raw) : null;
    } catch (e) {
      return null;
    }
  }
  function write(key, value) {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch (e) {
      // Full, or private browsing: the form still works, just unguarded.
    }
  }
  function remove(key) {
    try {
      window.localStorage.removeItem(key);
    } catch (e) {}
  }

  // A save the server confirmed: its draft is done with.
  document.querySelectorAll("[data-draft-clear]").forEach(function (el) {
    remove(el.getAttribute("data-draft-clear"));
  });

  // Drafts nobody came back for.
  try {
    var now = Date.now();
    for (var i = window.localStorage.length - 1; i >= 0; i--) {
      var k = window.localStorage.key(i);
      if (k && k.indexOf(PREFIX) === 0) {
        var old = read(k);
        if (!old || !(now - old.savedAt < MAX_AGE_MS)) remove(k);
      }
    }
  } catch (e) {}

  var form = document.getElementById("scout-form");
  if (!form || !form.dataset.draft) return;
  var key = form.dataset.draft;

  // The answers as they stand: {name: value}, a toggle as true or absent.
  function answers() {
    var out = {};
    Array.prototype.forEach.call(form.elements, function (el) {
      if (!el.name || IDENTITY[el.name]) return;
      if (el.type === "radio") {
        if (el.checked) out[el.name] = el.value;
      } else if (el.type === "checkbox") {
        if (el.checked) out[el.name] = true;
      } else if (el.type !== "submit" && el.type !== "button") {
        out[el.name] = el.value;
      }
    });
    return out;
  }

  function put(saved) {
    Array.prototype.forEach.call(form.elements, function (el) {
      if (!el.name || IDENTITY[el.name]) return;
      if (el.type === "radio") {
        el.checked = saved[el.name] === el.value;
      } else if (el.type === "checkbox") {
        el.checked = saved[el.name] === true;
      } else if (Object.prototype.hasOwnProperty.call(saved, el.name)) {
        el.value = saved[el.name];
      }
    });
  }

  var discarded = false;
  var timer = null;
  function save() {
    if (discarded) return;
    clearTimeout(timer);
    write(key, {
      savedAt: Date.now(),
      match: form.elements.match.value,
      team: form.elements.team.value,
      // The same idempotency key on a retry, so a save that did land but
      // whose answer was lost is not stored twice (D7).
      recordId: form.elements.record_id.value,
      answers: answers(),
    });
  }
  function soon() {
    clearTimeout(timer);
    timer = setTimeout(save, DEBOUNCE_MS);
  }

  function when(ms) {
    var at = new Date(ms);
    var today = at.toDateString() === new Date().toDateString();
    return today
      ? at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
      : at.toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" });
  }

  function announce(savedAt) {
    var note = document.createElement("div");
    note.className = "alert alert-info";
    note.setAttribute("role", "status");
    var text = document.createElement("p");
    text.textContent = "Restored your unsaved answers from " + when(savedAt) + ".";
    var discard = document.createElement("button");
    discard.type = "button";
    discard.className = "btn btn-secondary";
    discard.textContent = "Discard them and start over";
    discard.addEventListener("click", function () {
      discarded = true;
      clearTimeout(timer);
      remove(key);
      window.location.replace(window.location.href);
    });
    note.appendChild(text);
    note.appendChild(discard);
    form.parentNode.insertBefore(note, form);
  }

  if (form.hasAttribute("data-draft-posted")) {
    // The server is showing what was just posted: newer than any draft.
    save();
  } else {
    var draft = read(key);
    var ours =
      draft &&
      draft.answers &&
      draft.match === form.elements.match.value &&
      draft.team === form.elements.team.value;
    if (ours && JSON.stringify(draft.answers) !== JSON.stringify(answers())) {
      put(draft.answers);
      if (draft.recordId) form.elements.record_id.value = draft.recordId;
      announce(draft.savedAt);
    }
  }

  form.addEventListener("input", soon);
  form.addEventListener("change", soon);
  // Leaving, or the screen going off: write now, not in 400 ms.
  window.addEventListener("pagehide", save);
  // S11: the page is about to reload into a new version. Keep what is typed
  // now, without waiting for the debounce.
  document.addEventListener("tt:before-update", save);
  document.addEventListener("visibilitychange", function () {
    if (document.visibilityState === "hidden") save();
  });
})();
