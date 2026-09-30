// Which section this page is in (U16): its nav link gets aria-current, which
// the tab bar shows as the lit tab. By the first part of the path, so
// /lead-scout/rankings lights Lead Scout. A form re-shown at its /api/ address
// lights nothing, which is harmless. Without this script no tab is lit.
(function () {
  "use strict";

  function section(path) {
    return path.split("/")[1] || "";
  }

  var here = section(location.pathname);
  document.querySelectorAll(".nav-links a").forEach(function (link) {
    if (section(link.pathname) === here) link.setAttribute("aria-current", "page");
  });
})();
