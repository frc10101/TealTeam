// A scout's unsaved forms, handed to a lead scout by QR code (S13).
//
// When the tablet cannot reach the server, the forms it kept (draft.js, C3)
// go out as codes, shown in turn, for a lead scout who can reach it to scan
// (static/js/receive.js). They travel with the scout's offline token (C9),
// so the server records them as this scout, from this tablet. The lead's
// screen then shows a receipt; scanned here, it lets the tablet delete the
// forms the server answered for, recorded or refused. A refused one waits
// for a lead scout on the server, as a refused push does (C10).
//
// Only the signed-in scout's own forms, by the token's user: on a shared
// tablet, the others' wait for their own sign-in.
(function () {
  "use strict";

  var panel = document.querySelector("[data-handoff]");
  if (!panel || !window.ttQr || !window.ttScan || !window.ttToken) return;

  var PREFIX = "tt-draft:v1:";
  var SHOW_MS = 600;
  var $ = function (name) {
    return panel.querySelector("[data-handoff-" + name + "]");
  };
  var list = $("forms");
  var none = $("none");
  var signin = $("signin");
  var showButton = $("show");
  var code = $("code");
  var framesBox = $("frames");
  var part = $("part");
  var pauseButton = $("pause");
  var video = $("video");
  var photoInput = $("photo");
  var status = $("status");

  var cycling = null;
  var wakeLock = null;
  var camera = null;

  function say(text) {
    status.textContent = text;
  }

  function read(key) {
    try {
      var raw = window.localStorage.getItem(key);
      return raw ? JSON.parse(raw) : null;
    } catch (e) {
      return null;
    }
  }

  // tt-draft:v1:{user}:{event}:{match}:{team}:{form version}
  function drafts(user) {
    var out = [];
    try {
      for (var i = 0; i < window.localStorage.length; i++) {
        var key = window.localStorage.key(i);
        if (!key || key.indexOf(PREFIX + user + ":") !== 0) continue;
        var parts = key.split(":");
        var draft = read(key);
        if (parts.length !== 7 || !draft || !draft.recordId || !draft.answers) continue;
        out.push({ key: key, version: Number(parts[6]), draft: draft });
      }
    } catch (e) {}
    return out.sort(function (a, b) {
      return b.draft.savedAt - a.draft.savedAt;
    });
  }

  // 2026now_qm14 -> Q14, 2026now_sf2m1 -> SF2-1.
  function label(matchKey) {
    var m = /_(qm|ef|qf|sf|f)(\d+)(?:m(\d+))?$/.exec(matchKey || "");
    if (!m) return matchKey;
    if (m[1] === "qm") return "Q" + m[2];
    return m[1].toUpperCase() + m[2] + (m[3] ? "-" + m[3] : "");
  }

  function when(ms) {
    var at = new Date(ms);
    return at.toDateString() === new Date().toDateString()
      ? at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
      : at.toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" });
  }

  function claims() {
    return window.ttToken.claims();
  }

  function render() {
    Array.prototype.slice.call(list.querySelectorAll("label")).forEach(function (el) {
      el.remove();
    });
    var who = claims();
    signin.hidden = !!who;
    var found = who ? drafts(who.sub) : [];
    none.hidden = !who || found.length > 0;
    list.hidden = !found.length;
    showButton.hidden = !found.length;
    found.forEach(function (entry) {
      var row = document.createElement("label");
      row.className = "check handoff-form";
      var box = document.createElement("input");
      box.type = "checkbox";
      box.value = entry.key;
      // Ones the scout pressed Save on are finished; the rest may be half done.
      box.checked = !!entry.draft.tried;
      var text = document.createElement("span");
      text.textContent =
        label(entry.draft.match) +
        " · Team " +
        entry.draft.team +
        " · " +
        when(entry.draft.savedAt) +
        (entry.draft.tried ? "" : " · Save not pressed");
      row.appendChild(box);
      row.appendChild(text);
      list.appendChild(row);
    });
  }

  function chosen() {
    return Array.prototype.slice
      .call(list.querySelectorAll("input:checked"))
      .map(function (box) {
        var entry = read(box.value);
        return {
          record_id: entry.recordId,
          match: entry.match,
          team: entry.team,
          form_version: Number(box.value.split(":")[6]),
          saved_at: entry.savedAt,
          answers: entry.answers,
        };
      });
  }

  function hideCode() {
    if (cycling) cycling.stop();
    cycling = null;
    code.hidden = true;
    framesBox.textContent = "";
    if (wakeLock) wakeLock.release().catch(function () {});
    wakeLock = null;
  }

  function showCode() {
    var token = window.ttToken.token();
    var forms = chosen();
    if (!token) return render();
    if (!forms.length) return say("Tick the forms to send.");
    var body = new TextEncoder().encode(JSON.stringify({ token: token, forms: forms }));
    window.ttQr
      .frames("H", body)
      .then(function (texts) {
        hideCode();
        texts.forEach(function (text) {
          var figure = document.createElement("figure");
          figure.className = "qr-frame";
          figure.appendChild(window.ttQr.draw(text));
          framesBox.appendChild(figure);
        });
        code.hidden = false;
        pauseButton.textContent = "Pause";
        cycling = window.ttQr.cycle(framesBox, SHOW_MS, function (at, count) {
          part.textContent =
            count > 1 ? "Part " + (at + 1) + " of " + count + ", shown in turn" : "One code: hold it still for the lead.";
        });
        say(forms.length === 1 ? "Showing 1 form." : "Showing " + forms.length + " forms.");
        code.scrollIntoView({ block: "start" });
        // The screen stays on while the lead scans, where the browser allows.
        if (navigator.wakeLock) {
          navigator.wakeLock
            .request("screen")
            .then(function (lock) {
              wakeLock = lock;
            })
            .catch(function () {});
        }
      })
      .catch(function (e) {
        say(e.message);
      });
  }

  // ── The receipt ───────────────────────────────────────────────────────────

  var assembler = new window.ttQr.Assembler();

  function stopCamera() {
    if (camera) camera.stop();
    camera = null;
    video.hidden = true;
  }

  function settle(receipts) {
    var who = claims();
    var mine = who ? drafts(who.sub) : [];
    var recorded = 0;
    var refused = [];
    receipts.forEach(function (receipt) {
      mine.forEach(function (entry) {
        if (entry.draft.recordId !== receipt.client_record_id) return;
        try {
          window.localStorage.removeItem(entry.key);
        } catch (e) {}
        if (receipt.outcome === "recorded") recorded++;
        else refused.push(label(entry.draft.match) + " · Team " + entry.draft.team + ": " + receipt.reason);
      });
    });
    hideCode();
    render();
    if (!recorded && !refused.length) {
      say("That receipt is for forms this tablet no longer has. Nothing changed.");
      return;
    }
    var text = "The server has " + recorded + (recorded === 1 ? " of your forms." : " of your forms.");
    if (refused.length) {
      text += " Refused, and waiting for a lead scout to decide: " + refused.join("; ") + ".";
    }
    say(text);
    if (navigator.vibrate) navigator.vibrate(80);
  }

  function take(texts) {
    var heard = null;
    texts.forEach(function (text) {
      try {
        assembler.add(text);
      } catch (e) {
        if (!e.other) {
          heard = e.message;
          return;
        }
        assembler.reset();
        assembler.add(text);
      }
      if (assembler.kind === "H") {
        assembler.reset();
        heard = "That is a tablet's code, not the server's receipt.";
      }
    });
    var progress = assembler.progress();
    if (progress.count && progress.have === progress.count) {
      stopCamera();
      assembler
        .finish()
        .then(function (done) {
          assembler.reset();
          var receipt = JSON.parse(new TextDecoder().decode(done.bytes));
          settle(receipt.receipts || []);
        })
        .catch(function (e) {
          assembler.reset();
          say(e.message);
        });
      return;
    }
    if (heard) say(heard);
    else if (progress.count) say("Read " + progress.have + " of " + progress.count + " parts of the receipt.");
  }

  $("receipt").addEventListener("click", function () {
    if (camera) return;
    if (!window.ttScan.canStream()) {
      photoInput.click();
      return;
    }
    say("Starting the camera…");
    window.ttScan
      .stream(video, take)
      .then(function (running) {
        camera = running;
        video.hidden = false;
        say("Hold the camera to the receipt on the lead's screen.");
      })
      .catch(function () {
        say("The camera could not start here. Take a photo of the receipt instead.");
        photoInput.click();
      });
  });

  photoInput.addEventListener("change", function () {
    var file = photoInput.files && photoInput.files[0];
    photoInput.value = "";
    if (!file) return;
    say("Reading the photo…");
    window.ttScan
      .photo(file)
      .then(function (texts) {
        if (texts.length) take(texts);
        else say("No code found in that photo. Fill the picture with the receipt and hold still.");
      })
      .catch(function (e) {
        say(e.message || "That photo could not be read.");
      });
  });

  showButton.addEventListener("click", showCode);
  $("hide").addEventListener("click", hideCode);
  $("next").addEventListener("click", function () {
    if (!cycling) return;
    cycling.pause();
    pauseButton.textContent = "Resume";
    cycling.next();
  });
  pauseButton.addEventListener("click", function () {
    if (!cycling) return;
    if (cycling.paused()) {
      cycling.resume();
      pauseButton.textContent = "Pause";
    } else {
      cycling.pause();
      pauseButton.textContent = "Resume";
    }
  });
  panel.addEventListener("toggle", function () {
    if (panel.open) {
      // What is typed in this page's form goes too.
      if (window.ttDraft) window.ttDraft.save();
      render();
    } else {
      hideCode();
      stopCamera();
    }
  });

  panel.hidden = false;
  render();
  // While the server is still here: the reader, for a receipt scanned
  // without it.
  setTimeout(window.ttScan.preload, 2000);
})();
