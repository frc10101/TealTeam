// Counter buttons on the scouting form (U4).
//
// Every counter is a plain number input that works without this script. This
// adds the large − and + buttons either side of it, which the template renders
// hidden so a page without JavaScript never shows buttons that do nothing.
(function () {
  "use strict";

  document.querySelectorAll("[data-counter]").forEach(function (counter) {
    var input = counter.querySelector("input");
    if (!input) return;

    counter.querySelectorAll("[data-step]").forEach(function (button) {
      button.hidden = false;
      button.addEventListener("click", function () {
        var min = Number(input.min);
        var max = Number(input.max);
        var current = parseInt(input.value, 10);
        if (isNaN(current)) current = min;
        var next = current + Number(button.dataset.step);
        input.value = String(Math.min(max, Math.max(min, next)));
        // Let anything watching the form (C3's draft saving) see the change.
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    });
  });
})();
