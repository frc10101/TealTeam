// Live regions (U8): parts of a page that keep themselves current.
//
//   <div id="upstream-status" data-live="/lead-scout?event=2026mabil" data-live-every="30">
//
// Every `data-live-every` seconds the region fetches the page named in
// `data-live`, finds the element there with the region's own id, and swaps in
// its contents. The page is the fragment: there are no fragment routes, and the
// markup exists once, in the page's template. Without this script the region is
// simply what the page rendered.
//
// This replaces the retired app's Unpoly `[tt-src]` polling. S8's server push
// can later trigger the same refresh instead of a timer.
(function () {
  "use strict";

  var MIN_SECONDS = 5;
  var DEFAULT_SECONDS = 30;

  function start(region) {
    if (!region.id) return;
    var seconds = Number(region.dataset.liveEvery) || DEFAULT_SECONDS;
    var every = Math.max(MIN_SECONDS, seconds) * 1000;
    var busy = false;
    var lastTry = Date.now();

    function refresh() {
      // A tablet with its screen off should not poll the Pi all afternoon.
      if (busy || document.hidden) return;
      // Swapping would drop the focus from under someone's finger. Next time.
      if (region.contains(document.activeElement)) return;
      busy = true;
      lastTry = Date.now();

      fetch(region.dataset.live, {
        credentials: "same-origin",
        cache: "no-store",
        headers: { Accept: "text/html" },
      })
        .then(function (response) {
          if (!response.ok) throw new Error("HTTP " + response.status);
          return response.text();
        })
        .then(function (html) {
          // DOMParser runs no scripts. A missing id means the fetch landed
          // somewhere else -- the sign-in page, once a session expires -- and
          // that must not be swapped into this page.
          var page = new DOMParser().parseFromString(html, "text/html");
          var fresh = page.getElementById(region.id);
          if (!fresh) throw new Error("#" + region.id + " is not on " + region.dataset.live);
          // Unchanged content is left alone, so a text selection survives.
          if (fresh.innerHTML !== region.innerHTML) region.innerHTML = fresh.innerHTML;
          region.removeAttribute("data-live-failed");
        })
        .catch(function () {
          // Keep what is there, and say it has stopped updating. Content that
          // looks live but is not is how stale rankings cause bad picks.
          region.setAttribute("data-live-failed", "");
        })
        .then(function () {
          busy = false;
        });
    }

    setInterval(refresh, every);
    document.addEventListener("visibilitychange", function () {
      if (!document.hidden && Date.now() - lastTry >= every) refresh();
    });
  }

  document.querySelectorAll("[data-live]").forEach(start);
})();
