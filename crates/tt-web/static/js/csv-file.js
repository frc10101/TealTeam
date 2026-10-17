// Picking a CSV file fills a textarea with it (F3).
//
//   <input type="file" data-fills="schedule" hidden>
//   <textarea id="schedule" name="schedule"></textarea>
//
// The form posts the textarea, never the file, so pasting and picking end the
// same way and the text can be checked before importing. The picker starts
// hidden: without JavaScript it could not fill anything, and pasting works.
(function () {
  "use strict";

  document.querySelectorAll("input[type=file][data-fills]").forEach(function (input) {
    var target = document.getElementById(input.dataset.fills);
    if (!target || !window.FileReader) return;
    input.hidden = false;
    input.addEventListener("change", function () {
      var file = input.files && input.files[0];
      if (!file) return;
      var reader = new FileReader();
      reader.onload = function () {
        target.value = String(reader.result || "");
      };
      reader.readAsText(file);
    });
  });
})();
