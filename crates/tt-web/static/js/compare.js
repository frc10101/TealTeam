// Compare teams (DR1): a changed choice shows its comparison at once, with
// no Show button. The form is a GET form, so this is an ordinary navigation
// to a URL that can be bookmarked or sent.
(function () {
  "use strict";

  var form = document.getElementById("compare-form");
  if (!form) return;
  var status = document.getElementById("compare-status");
  var show = form.querySelector("[data-compare-show]");
  if (show) show.hidden = true;

  form.addEventListener("change", function (e) {
    var input = e.target;
    var set = input.closest("[data-kind]");
    if (set && input.checked) {
      var max = Number(set.getAttribute("data-max"));
      var on = set.querySelectorAll("input:checked").length;
      if (on > max) {
        input.checked = false;
        if (status) status.textContent = "Up to " + max + " teams at once. Tap one off first.";
        return;
      }
    }
    form.submit();
  });
})();
