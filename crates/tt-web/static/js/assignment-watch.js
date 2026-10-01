// A scout hears when the lead changes their assignment (S9).
//
// The scouting page carries who this is (user, tablet), the robot they are on
// if it was assigned, and the event's match labels. The live stream (S8) says
// when any assignment changes; this picks out the ones about this scout and
// says so in words:
//
//   - the robot on screen was given to someone else, or unassigned;
//   - they were given a robot, which with the first is a move.
//
// It never changes the form. A scout already watching the robot can finish
// and save; the server takes an observation of any robot in the match.
(function () {
  "use strict";

  var box = document.getElementById("assignment-watch");
  if (!box || !window.EventSource) return;
  var watch;
  try {
    watch = JSON.parse(box.dataset.watch);
  } catch (e) {
    return;
  }

  function mine(row) {
    return (
      !!row &&
      ((watch.user != null && row.scouter_id === watch.user) ||
        (watch.device != null && row.device_id === watch.device))
    );
  }

  // "2026now_qm14:254" -> "Q14 · Team 254".
  function describe(pk) {
    var at = pk.lastIndexOf(":");
    var key = pk.slice(0, at);
    return (watch.labels[key] || key) + " · Team " + pk.slice(at + 1);
  }

  var lost = null;
  var gained = [];
  var timer = null;

  function say() {
    var parts = [];
    if (lost) {
      parts.push(
        "The lead scout took " + describe(lost) + " off your list. " +
          "If you are already watching it, finish and save; otherwise go to your next robot."
      );
    }
    if (gained.length) {
      parts.push(
        (lost ? "You now have " : "New assignment: ") + gained.map(describe).join(", ") + "."
      );
    }
    if (!parts.length) return;
    box.textContent = "";
    box.className = "alert " + (lost ? "alert-warning" : "alert-info");
    box.setAttribute("role", lost ? "alert" : "status");
    parts.forEach(function (text) {
      var p = document.createElement("p");
      p.textContent = text;
      box.appendChild(p);
    });
    var link = document.createElement("a");
    link.className = "btn btn-primary";
    link.href = watch.href;
    link.textContent = "Open my next assignment";
    var p = document.createElement("p");
    p.appendChild(link);
    box.appendChild(p);
    box.hidden = false;
  }

  var source = new EventSource(watch.stream);
  source.addEventListener("change", function (event) {
    var change;
    try {
      change = JSON.parse(event.data);
    } catch (e) {
      return;
    }
    if (change.entity !== "assignment") return;
    // agenda.js marks the copy kept for the offline page out of date (C8).
    document.dispatchEvent(new CustomEvent("tt:assignment-change", { detail: change }));
    var ours = change.op === "upsert" && mine(change.row);
    if (change.entity_pk === watch.current) {
      if (!ours) lost = change.entity_pk;
    } else if (ours && gained.indexOf(change.entity_pk) < 0) {
      gained.push(change.entity_pk);
    }
    clearTimeout(timer);
    // A move is a removal and an addition: say them together.
    timer = setTimeout(say, 500);
  });
})();
