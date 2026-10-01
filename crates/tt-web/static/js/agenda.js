// A scout's assignments, kept on the device for when the network drops (C8).
//
// The scouting page lists every robot the scout still has to watch, and
// carries the same list as data-agenda. This keeps the newest copy in
// localStorage -- which, unlike a service worker or OPFS, works over the
// event LAN's plain http -- and draws it on the offline page (C1), which is
// the same for everyone and so cannot carry it itself.
//
// One copy per device: the last scout to open the scouting page here. It
// says whose it is and how old it is, and that the lead may have changed it
// since. Signing out throws it away.
(function () {
  "use strict";

  var KEY = "tt-agenda:v1";
  var MAX_AGE_MS = 3 * 24 * 60 * 60 * 1000;

  function read() {
    try {
      var raw = window.localStorage.getItem(KEY);
      return raw ? JSON.parse(raw) : null;
    } catch (e) {
      return null;
    }
  }
  function write(value) {
    try {
      window.localStorage.setItem(KEY, JSON.stringify(value));
    } catch (e) {
      // Full, or private browsing: the page still has its list.
    }
  }
  function remove() {
    try {
      window.localStorage.removeItem(KEY);
    } catch (e) {}
  }

  var kept = read();
  if (kept && !(Date.now() - kept.savedAt < MAX_AGE_MS)) {
    remove();
    kept = null;
  }

  // The scouting page: this is now the copy to keep.
  var source = document.getElementById("agenda-keep");
  if (source) {
    try {
      kept = JSON.parse(source.dataset.agenda);
      kept.savedAt = Date.now();
      write(kept);
    } catch (e) {}
  }

  // A change to one of these robots, or a new one for this scout, pushed
  // while the page is open (S9): the kept list is now out of date. The page
  // says so itself; the offline page needs telling.
  document.addEventListener("tt:assignment-change", function (event) {
    var current = read();
    var change = event.detail;
    if (!current || !change) return;
    var row = change.op === "upsert" ? change.row : null;
    var ours =
      !!row &&
      ((current.user != null && row.scouter_id === current.user) ||
        (current.device != null && row.device_id === current.device));
    if (ours || (current.pks || []).indexOf(change.entity_pk) >= 0) {
      current.changedAt = current.changedAt || Date.now();
      write(current);
    }
  });

  document.addEventListener("submit", function (event) {
    var action = event.target.getAttribute("action") || "";
    if (action.indexOf("/api/auth/logout") === 0) remove();
  });

  var box = document.getElementById("offline-agenda");
  if (!box || !kept) return;

  function time(ms) {
    return new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }
  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }
  // The scouting page's rows, without the links: none of them would load.
  function list(duties) {
    var ol = el("ol", "duties");
    duties.forEach(function (d) {
      var row = el("div", "duty");
      var when = el("span", "duty-when");
      when.appendChild(el("strong", "", d.label + " · " + d.station));
      if (d.time) when.appendChild(el("small", "", d.time));
      var team = el("span", "duty-team");
      team.appendChild(el("strong", "", String(d.team)));
      if (d.name) team.appendChild(el("small", "", d.name));
      row.appendChild(when);
      row.appendChild(team);
      var li = el("li");
      li.appendChild(row);
      ol.appendChild(li);
    });
    return ol;
  }

  var header = el("div", "card-header");
  header.appendChild(el("h2", "", "Your assignments"));
  var body = el("div", "card-body stack");
  body.appendChild(
    el(
      "p",
      "hint",
      kept.scout + "'s list at " + kept.event_name + ", as of " + time(kept.savedAt) +
        ". The lead scout may have changed it since."
    )
  );
  if (kept.changedAt) {
    body.appendChild(
      el(
        "p",
        "alert alert-warning",
        "The lead scout changed your assignments at " + time(kept.changedAt) +
          ", after this list was kept. Check with them."
      )
    );
  }
  if (kept.upcoming && kept.upcoming.length) {
    body.appendChild(list(kept.upcoming));
  } else {
    body.appendChild(el("p", "", "No robots assigned in matches still to come."));
  }
  if (kept.missed && kept.missed.length) {
    body.appendChild(el("h3", "", "Still to record"));
    body.appendChild(list(kept.missed));
  }
  box.appendChild(header);
  box.appendChild(body);
  box.hidden = false;
})();
