// The assignment grid, kept current one cell at a time (S9, REBUILD_SPEC.md
// 12.8).
//
// Two things used to re-render the whole grid: the lead's own Save, and the
// 30-second refresh that picks up everyone else's. Now:
//
//   - A Save in the match editor is posted with fetch. The page the 303 leads
//     to is read, and only the editor, the notice, and the saved match's cells
//     are swapped in. Anything but a redirect -- a refused save and its
//     reasons -- is posted again the ordinary way, so the page shows it.
//   - An assignment change on the live stream (S8) marks its cell. A moment
//     later one fetch of the grid refreshes just the marked cells.
//
// The cells are the page's own markup, found by id (`match_key:team`, which is
// the change's entity_pk), so there are no fragment routes to keep in step.
// Without this script: plain posts, 303s, and live.js's polling.
(function () {
  "use strict";

  var anchor = document.getElementById("grid-stream");
  if (!anchor || !window.fetch || !window.DOMParser) return;

  function parse(html) {
    return new DOMParser().parseFromString(html, "text/html");
  }

  // Put `there`'s content into `here`, and outline it if it changed.
  function swapCell(here, there) {
    if (!here || !there || here.innerHTML === there.innerHTML) return;
    here.className = there.className;
    here.innerHTML = there.innerHTML;
    here.classList.add("slot-changed");
    setTimeout(function () {
      here.classList.remove("slot-changed");
    }, 2500);
  }

  function swapRegion(id, doc) {
    var here = document.getElementById(id);
    var there = doc.getElementById(id);
    if (here && there) here.innerHTML = there.innerHTML;
  }

  // ── Everyone else's changes, from the stream ────────────────────────────

  var marked = {};
  var timer = null;

  function refreshMarked() {
    var ids = Object.keys(marked);
    marked = {};
    if (!ids.length) return;
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
        var doc = parse(html);
        // Not the grid (a sign-in page, once a session expires): leave it.
        if (!doc.getElementById("grid-stream")) return;
        ids.forEach(function (id) {
          swapCell(document.getElementById(id), doc.getElementById(id));
        });
        swapRegion("coverage", doc);
      })
      .catch(function () {
        // live.js's polling still runs underneath.
      });
  }

  if (window.EventSource && anchor.dataset.stream) {
    var source = new EventSource(anchor.dataset.stream);
    source.addEventListener("change", function (event) {
      var change;
      try {
        change = JSON.parse(event.data);
      } catch (e) {
        return;
      }
      if (change.entity !== "assignment") return;
      marked[change.entity_pk] = true;
      clearTimeout(timer);
      // Auto-distribute changes dozens at once: one fetch for the lot.
      timer = setTimeout(refreshMarked, 300);
    });
  }

  // ── The lead's own Save, in place ───────────────────────────────────────

  var bypass = false;
  document.addEventListener("submit", function (event) {
    var form = event.target;
    if (bypass || !form.matches || !form.matches("form[data-inplace]")) return;
    event.preventDefault();
    var button = event.submitter;
    var data = new URLSearchParams(new FormData(form));
    if (button && button.name) data.append(button.name, button.value);
    var saved = form.elements.match ? form.elements.match.value : "";

    function theOrdinaryWay() {
      bypass = true;
      if (form.requestSubmit) form.requestSubmit(button || undefined);
      else form.submit();
    }

    fetch(form.action, {
      method: "POST",
      body: data,
      credentials: "same-origin",
      headers: { Accept: "text/html" },
    })
      .then(function (response) {
        // Only a 303 back to the grid is a save; anything else is shown by
        // the ordinary post.
        if (!response.ok || !response.redirected) throw new Error("not saved in place");
        return response.text().then(function (html) {
          return { url: response.url, doc: parse(html) };
        });
      })
      .then(function (result) {
        var doc = result.doc;
        if (!doc.getElementById("grid-stream")) throw new Error("not the grid");
        var row = doc.getElementById(saved);
        if (row) {
          row.querySelectorAll("td[id]").forEach(function (cell) {
            swapCell(document.getElementById(cell.id), cell);
          });
        }
        swapRegion("grid-notice", doc);
        swapRegion("coverage", doc);
        // The editor: the same match again, the next one, or closed.
        var editor = document.getElementById("edit");
        var next = doc.getElementById("edit");
        if (editor && next) editor.replaceWith(document.importNode(next, true));
        else if (editor) editor.remove();
        history.replaceState(null, "", result.url);
      })
      .catch(theOrdinaryWay);
  });
})();
