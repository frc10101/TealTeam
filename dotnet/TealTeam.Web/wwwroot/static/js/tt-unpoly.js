// TealTeam Unpoly integration layer.
//
// Bridges the two places where Unpoly's model differs from the fragment
// patterns this app was built around:
//
//   1. Server-driven full navigation (what htmx did with the HX-Redirect
//      response header). The server emits an `tt:navigate` event via the
//      `X-Up-Events` response header; we turn it into a real navigation.
//
//   2. Self-loading / polling regions and change-driven loads. Unpoly extracts
//      the target selector *from* the response, but several endpoints here
//      return a bare inner fragment. `[tt-src]` fetches and sets innerHTML
//      (htmx-style) and re-compiles; `[tt-change]` renders a select's target
//      through Unpoly on change.

(function () {
  if (typeof up === "undefined") {
    return;
  }

  // (1) Server-requested navigation. Emitted from the server as:
  //     X-Up-Events: [{"type":"tt:navigate","url":"/"}]
  up.on("tt:navigate", function (event) {
    window.location.assign(event.url || "/");
  });

  // (2a) Self-loading region that mirrors:
  //      hx-get + hx-trigger="load, every Ns, someEvent from:body" + hx-swap=innerHTML
  //
  //   <div tt-src="/hx/..." tt-load tt-poll="30000" tt-on="eventA,eventB"></div>
  up.compiler("[tt-src]", function (el) {
    var url = el.getAttribute("tt-src");
    var interval = parseInt(el.getAttribute("tt-poll") || "0", 10);
    var events = (el.getAttribute("tt-on") || "")
      .split(",")
      .map(function (s) { return s.trim(); })
      .filter(Boolean);

    var busy = false;
    function load() {
      if (busy) return;
      busy = true;
      up.request(url, { cache: false })
        .then(function (response) {
          el.innerHTML = response.text;
          up.hello(el);
        })
        .catch(function () {})
        .then(function () { busy = false; });
    }

    if (el.hasAttribute("tt-load")) {
      load();
    }
    var timer = interval ? setInterval(load, interval) : null;
    var subscriptions = events.map(function (ev) { return up.on(ev, load); });

    return function () {
      if (timer) clearInterval(timer);
      subscriptions.forEach(function (off) { off(); });
    };
  });

  // (2b) Change-driven fragment render, mirroring a <select> with
  //      hx-get + hx-trigger="change[, load]". The endpoint returns a fragment
  //      whose root element matches tt-target, and Unpoly swaps it.
  //
  //   <select tt-change="/path?event_id={value}" tt-target="#foo" tt-load>
  up.compiler("[tt-change]", function (el) {
    function go() {
      var template = el.getAttribute("tt-change");
      var url = template.replace("{value}", encodeURIComponent(el.value || ""));
      up.render({ url: url, target: el.getAttribute("tt-target"), cache: false })
        .catch(function () {});
    }
    el.addEventListener("change", go);
    if (el.hasAttribute("tt-load") && el.value) {
      go();
    }
  });
})();
