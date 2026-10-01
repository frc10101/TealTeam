// The lead scout's scanner (S13): reads a tablet's forms off its screen and
// posts them to the server, which records them and answers with a receipt.
//
// It keeps each part it has not seen, in any order, and posts the texts as
// they were read; the server puts them together and checks them again
// (tt_core::qr). The ring and the count show how far it has come; a part
// that was missed comes round again on the tablet. A code of another
// transfer starts over on it: the last tablet has gone.
(function () {
  "use strict";

  var root = document.querySelector("[data-scanner]");
  var form = document.getElementById("handoff-post");
  if (!root || !form || !window.ttQr || !window.ttScan) return;
  root.hidden = false;

  var video = root.querySelector("video");
  var ring = root.querySelector("[data-ring]");
  var count = root.querySelector("[data-count]");
  var status = root.querySelector("[data-status]");
  var cameraButton = root.querySelector("[data-camera]");
  var photoInput = root.querySelector("[data-photo]");
  var photoHint = root.querySelector("[data-photo-hint]");
  var restart = root.querySelector("[data-restart]");

  var assembler = new window.ttQr.Assembler();
  var kept = {};
  var camera = null;
  var sent = false;

  function say(text) {
    status.textContent = text;
  }

  function show(progress) {
    var share = progress.count ? (100 * progress.have) / progress.count : 0;
    ring.setAttribute("stroke-dasharray", share + " 100");
    count.textContent = progress.count ? progress.have + " / " + progress.count : "0 / ?";
    restart.hidden = !progress.count;
    root.classList.toggle("scanner-done", progress.count > 0 && progress.have === progress.count);
  }

  function startOver() {
    assembler.reset();
    kept = {};
    show(assembler.progress());
  }

  function take(texts) {
    if (sent) return;
    var heard = null;
    texts.forEach(function (text) {
      var progress;
      try {
        progress = assembler.add(text);
      } catch (e) {
        if (!e.other) {
          heard = e.message;
          return;
        }
        startOver();
        heard = "A code from another tablet: starting over on it.";
        progress = assembler.add(text);
      }
      if (progress.kind === "R") {
        startOver();
        heard = "That is a receipt from the server, for a tablet to scan. Scan the code on the scout's tablet.";
        return;
      }
      if (progress.fresh) kept[window.ttQr.parse(text).index] = text.trim();
    });
    var progress = assembler.progress();
    show(progress);
    if (progress.count && progress.have === progress.count) return send();
    if (heard) return say(heard);
    if (progress.count) {
      say("Read " + progress.have + " of " + progress.count + ". Still to read: part " + assembler.missing().join(", ") + ".");
    }
  }

  function send() {
    sent = true;
    if (camera) camera.stop();
    if (navigator.vibrate) navigator.vibrate(80);
    say("Every part is read. Recording…");
    var texts = Object.keys(kept)
      .sort(function (a, b) {
        return a - b;
      })
      .map(function (i) {
        return kept[i];
      });
    form.elements.frames.value = texts.join("\n");
    form.submit();
  }

  function streamCamera() {
    cameraButton.hidden = true;
    say("Starting the camera…");
    window.ttScan
      .stream(video, take)
      .then(function (running) {
        camera = running;
        video.hidden = false;
        if (!assembler.progress().count) say("Hold the camera to the code on the scout's tablet.");
      })
      .catch(function (e) {
        video.hidden = true;
        cameraButton.hidden = false;
        say(
          e && e.name === "NotAllowedError"
            ? "The camera was not allowed. Allow it and tap Use the camera, or take photos instead."
            : (e && e.message) || "The camera could not start. Take photos of the code instead."
        );
      });
  }

  photoInput.addEventListener("change", function () {
    var files = Array.prototype.slice.call(photoInput.files || []);
    photoInput.value = "";
    if (!files.length) return;
    say("Reading the photo…");
    files
      .reduce(function (done, file) {
        return done.then(function (found) {
          return window.ttScan.photo(file).then(function (texts) {
            return found.concat(texts);
          });
        });
      }, Promise.resolve([]))
      .then(function (texts) {
        if (!texts.length) {
          say("No code found in that photo. Fill the picture with the code and hold still.");
          return;
        }
        take(texts);
      })
      .catch(function (e) {
        say((e && e.message) || "That photo could not be read.");
      });
  });

  cameraButton.addEventListener("click", streamCamera);
  restart.addEventListener("click", function () {
    startOver();
    say("Starting over. Hold the camera to the code.");
  });

  show(assembler.progress());
  if (window.ttScan.canStream()) {
    streamCamera();
  } else {
    photoHint.hidden = false;
    say("Take a photo of the code on the scout's tablet.");
  }
})();
