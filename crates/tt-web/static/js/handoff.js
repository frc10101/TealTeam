// A scout's unsaved forms, handed to a lead scout by QR code (S13).
//
// When the tablet cannot reach the server, the forms it kept (draft.js, C3)
// go out as codes, shown in turn, for a lead scout who can reach it to scan
// (static/js/receive.js). The codes never carry the scout's token (T1):
// they name its holder (who, which tablet, when issued) and carry an HMAC
// over the forms by the device key token.js keeps beside it, which the
// server derives again, so it records them as this scout, from this
// tablet. The lead's screen then shows a receipt with the server's HMAC by
// the same key over the codes' nonce; scanned here and checked, it lets the
// tablet delete the forms the server answered for, recorded or refused, as
// they were when the codes were made. A refused one waits for a lead scout
// on the server, as a refused push does (C10).
//
// What the codes last showed is kept as tt-handoff:v1:{user} -- the nonce,
// the key that made the MAC, and a digest of each form as sent -- so a
// receipt still settles after a reload or a fresh token, an old receipt
// answers nothing, and a form edited since is kept.
//
// Only the signed-in scout's own forms, by the token's user: on a shared
// tablet, the others' wait for their own sign-in.
(function () {
  "use strict";

  var panel = document.querySelector("[data-handoff]");
  if (!panel || !window.ttQr || !window.ttScan || !window.ttToken || !window.ttHmac) return;

  var PREFIX = "tt-draft:v1:";
  var SHOWN = "tt-handoff:v1:";
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

  function nonce() {
    var bytes = new Uint8Array(16);
    if (window.crypto && window.crypto.getRandomValues) {
      window.crypto.getRandomValues(bytes);
    } else {
      for (var i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
    }
    return window.ttHmac.toBase64Url(bytes);
  }

  // What a form said, for telling whether it changed since it was sent.
  function digest(match, team, answers) {
    var text = JSON.stringify([match, team, answers]);
    return window.ttHmac.toBase64Url(window.ttHmac.sha256(new TextEncoder().encode(text)));
  }

  function remember(who, shown) {
    try {
      window.localStorage.setItem(SHOWN + who.sub, JSON.stringify(shown));
    } catch (e) {}
  }

  function showCode() {
    var holder = window.ttToken.holder();
    var key = window.ttToken.key();
    var forms = chosen();
    if (!holder || !key) return render();
    if (!forms.length) return say("Tick the forms to send.");
    // tt_core::qr::{Handoff, HandoffBody}: the MAC is over these exact bytes.
    var shown = { nonce: nonce(), at: Date.now(), key: window.ttHmac.toBase64Url(key), forms: {} };
    forms.forEach(function (form) {
      shown.forms[form.record_id] = digest(form.match, form.team, form.answers);
    });
    var body = JSON.stringify({ scout: holder, nonce: shown.nonce, at: shown.at, forms: forms });
    var mac = window.ttHmac.mac(key, "handoff", body);
    var bytes = new TextEncoder().encode(JSON.stringify({ body: body, mac: mac }));
    window.ttQr
      .frames("H", bytes)
      .then(function (texts) {
        hideCode();
        remember(holder, shown);
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

  // The receipt's own words, if the server made it for the codes this
  // tablet last showed; else why not, for the scout.
  function check(signed) {
    var who = claims();
    if (!who) return { why: "Sign in on this tablet to read a receipt." };
    var shown = read(SHOWN + who.sub);
    if (!shown || !shown.key) return { why: "This tablet has shown no codes to answer. Nothing changed." };
    var key = window.ttHmac.fromBase64Url(shown.key);
    if (!signed || typeof signed.body !== "string" || !window.ttHmac.holds(key, "receipt", signed.body, signed.mac)) {
      return { why: "That receipt was not made by the server for this tablet's codes. Nothing changed." };
    }
    var body = JSON.parse(signed.body);
    if (body.nonce !== shown.nonce) {
      return { why: "That receipt is for codes this tablet showed before. Nothing changed." };
    }
    return { who: who, shown: shown, receipts: body.receipts || [] };
  }

  function settle(checked) {
    var mine = drafts(checked.who.sub);
    var recorded = 0;
    var refused = [];
    checked.receipts.forEach(function (receipt) {
      var sent = checked.shown.forms[receipt.client_record_id];
      mine.forEach(function (entry) {
        if (entry.draft.recordId !== receipt.client_record_id) return;
        // Edited since the codes were made: what the server has is older.
        if (sent !== digest(entry.draft.match, entry.draft.team, entry.draft.answers)) return;
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
      say("That receipt is for forms this tablet no longer has, or has changed since. Nothing changed.");
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
          var checked = check(JSON.parse(new TextDecoder().decode(done.bytes)));
          if (checked.why) say(checked.why);
          else settle(checked);
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
