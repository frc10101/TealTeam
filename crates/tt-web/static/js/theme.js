// Light or dark. Auto follows the device; a [data-theme-cycle] button steps
// Auto → Light → Dark and this device remembers it. Loaded in <head> without
// defer, so a page is drawn in the right theme from its first frame.
(function () {
  "use strict";

  var KEY = "tt-theme";
  var NEXT = { auto: "light", light: "dark", dark: "auto" };
  var LABEL = { auto: "Theme: Auto", light: "Theme: Light", dark: "Theme: Dark" };
  var root = document.documentElement;

  function chosen() {
    try {
      var t = localStorage.getItem(KEY);
      return NEXT[t] ? t : "auto";
    } catch (e) {
      return "auto";
    }
  }

  // The header's colour for the browser's own bar. Auto leaves each
  // <meta name="theme-color"> to its media query; a pick sets both.
  var BAR = { light: "#ffffff", dark: "#17171a" };

  function apply(theme) {
    if (theme === "auto") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
    var metas = document.querySelectorAll('meta[name="theme-color"]');
    for (var j = 0; j < metas.length; j++) {
      var m = metas[j];
      if (!m.hasAttribute("data-scheme")) m.setAttribute("data-scheme", m.getAttribute("media") || "");
      var scheme = m.getAttribute("data-scheme");
      if (theme === "auto") {
        if (scheme) m.setAttribute("media", scheme);
        m.setAttribute("content", /light/.test(scheme) ? BAR.light : BAR.dark);
      } else {
        m.removeAttribute("media");
        m.setAttribute("content", BAR[theme]);
      }
    }
    var buttons = document.querySelectorAll("[data-theme-cycle]");
    for (var i = 0; i < buttons.length; i++) buttons[i].textContent = LABEL[theme];
  }

  var current = chosen();
  apply(current);
  document.addEventListener("DOMContentLoaded", function () { apply(current); });

  document.addEventListener("click", function (e) {
    var button = e.target.closest && e.target.closest("[data-theme-cycle]");
    if (!button) return;
    current = NEXT[current];
    try { localStorage.setItem(KEY, current); } catch (err) { /* this page only */ }
    apply(current);
    // graph.js redraws in the new colours.
    document.dispatchEvent(new Event("tt-theme"));
  });
})();
