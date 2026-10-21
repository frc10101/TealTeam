// Compare teams (DR1): a changed choice shows its comparison at once, with
// no Show button, and + Add team's search narrows its list as you type.
//
// The form is a GET form, so every change is an ordinary navigation to a
// URL that can be bookmarked or sent. Without this script the Show button
// submits, and the whole list is there to pick from.
(function () {
  "use strict";

  var form = document.getElementById("compare-form");
  if (!form) return;
  var status = document.getElementById("compare-status");
  var show = form.querySelector("[data-compare-show]");
  if (show) show.hidden = true;

  form.addEventListener("change", function (e) {
    if (e.target.tagName !== "SELECT") return;
    if (status) status.textContent = "Updating…";
    form.submit();
  });

  var add = document.getElementById("compare-add");
  var search = document.getElementById("compare-search");
  if (!add || !search) return;
  var items = Array.prototype.slice.call(add.querySelectorAll("li[data-search]"));
  var none = document.getElementById("compare-none");

  function visible() { return items.filter(function (li) { return !li.hidden; }); }

  // By number or name, anywhere in either: "16" finds 1678 and 3161.
  search.addEventListener("input", function () {
    var q = search.value.trim().toLowerCase();
    items.forEach(function (li) { li.hidden = q !== "" && li.getAttribute("data-search").indexOf(q) < 0; });
    none.hidden = visible().length > 0;
  });

  // Enter adds the top match, rather than submitting the form unchanged.
  search.addEventListener("keydown", function (e) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    var first = visible()[0];
    if (first) first.querySelector("button").click();
  });

  add.addEventListener("toggle", function () {
    if (add.open) search.focus();
  });
})();
