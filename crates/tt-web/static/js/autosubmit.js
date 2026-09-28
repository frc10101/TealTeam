// Selects that act the moment they change (U2, U4, U8).
//
//   <form method="get"><select name="event" data-autosubmit>…</select>
//   <noscript><button>Go</button></noscript></form>
//
// The select sits in a GET form, so submitting it is an ordinary navigation to
// a URL with the choice in it: the header's event switcher reloads the current
// page for the new event, the scouting page's match select opens that match.
// Without JavaScript the <noscript> Go button does the same thing.
//
// This replaces the retired app's Unpoly `[tt-change]`, which rendered a
// fragment instead. A navigation also leaves a bookmarkable URL behind.
(function () {
  "use strict";

  document.querySelectorAll("select[data-autosubmit]").forEach(function (select) {
    select.addEventListener("change", function () {
      select.form.submit();
    });
  });
})();
