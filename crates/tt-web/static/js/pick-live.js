// The pick list, redrawn when someone else changes it (L14).
//
// Every change to the list goes out on the live stream (S8) as a
// pick_list_entry change, to the owning team only. A moment after the last
// one, the page is fetched once and the list, the teams not on it, and the
// add box's suggestions are swapped in. The lead's own changes are posted the
// ordinary way and come back as a page drawn after them.
//
// Nothing is swapped out from under someone mid-edit: while a field in the
// list has focus, the redraw waits until it does not. Panels that were open
// stay open.
//
// Without this script: the list as it was drawn, and a reload to see more.
(function () {
  "use strict";

  var anchor = document.getElementById("pick-stream");
  if (!anchor || !window.fetch || !window.DOMParser || !window.EventSource) return;

  var REGIONS = ["pick-rows", "pick-not-listed", "pick-candidates"];
  var timer = null;
  var waiting = false;

  // Typing a place, or choosing a colour, in the list.
  function busy() {
    var active = document.activeElement;
    var rows = document.getElementById("pick-rows");
    return !!(active && rows && rows.contains(active) && active.matches("input, select"));
  }

  function openPanels() {
    var ids = [];
    REGIONS.forEach(function (region) {
      var here = document.getElementById(region);
      if (!here) return;
      here.querySelectorAll("details[open][id]").forEach(function (panel) {
        ids.push(panel.id);
      });
    });
    return ids;
  }

  function redraw() {
    if (busy()) {
      waiting = true;
      return;
    }
    fetch(anchor.dataset.page, {
      credentials: "same-origin",
      cache: "no-store",
      headers: { Accept: "text/html" },
    })
      .then(function (response) {
        if (!response.ok) throw new Error("HTTP " + response.status);
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        // Not the pick list (a sign-in page, once a session expires): leave it.
        if (!doc.getElementById("pick-stream")) return;
        if (busy()) {
          waiting = true;
          return;
        }
        var open = openPanels();
        REGIONS.forEach(function (id) {
          var here = document.getElementById(id);
          var there = doc.getElementById(id);
          if (here && there && here.innerHTML !== there.innerHTML) {
            here.innerHTML = there.innerHTML;
          }
        });
        open.forEach(function (id) {
          var panel = document.getElementById(id);
          if (panel) panel.open = true;
        });
      })
      .catch(function () {
        // The next change tries again; a reload always works.
      });
  }

  document.addEventListener("focusout", function () {
    if (!waiting) return;
    // Focus moving from one field in the list to another is still busy.
    setTimeout(function () {
      if (waiting && !busy()) {
        waiting = false;
        redraw();
      }
    }, 0);
  });

  var source = new EventSource(anchor.dataset.stream);
  source.addEventListener("change", function (event) {
    var change;
    try {
      change = JSON.parse(event.data);
    } catch (e) {
      return;
    }
    if (change.entity !== "pick_list_entry") return;
    clearTimeout(timer);
    // A move can renumber every row: one fetch for the lot.
    timer = setTimeout(redraw, 300);
  });
})();
