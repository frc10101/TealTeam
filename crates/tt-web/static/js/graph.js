// The graph view (U21): draws the chosen teams and metrics with uPlot, and
// redraws the moment a chip is tapped, with no reload.
//
// The page holds every team on offer in #graph's data-graph, so a tap only
// redraws. The URL is kept in step (replaceState), so the chart on screen is
// always the one a bookmark or a shared link opens.
//
// A chip turned on takes the first free slot: its colour (a team) or line
// style (a metric). Turning another off never repaints it, so a line does
// not change colour under the reader's eye.
//
// Without this script, or without uPlot, the chips are a GET form with a Show
// button, and the numbers are tables.
(function () {
  "use strict";

  var root = document.getElementById("graph");
  var form = document.getElementById("graph-form");
  if (!root || !form || typeof uPlot === "undefined") return;

  var data = JSON.parse(root.getAttribute("data-graph"));
  var status = document.getElementById("graph-status");
  var tables = document.getElementById("graph-tables");
  var readout = document.getElementById("graph-readout");
  var DASH = [[], [10, 6], [2, 5]];
  var style = getComputedStyle(document.documentElement);
  var ink = style.getPropertyValue("--gray-300").trim();
  var rule = style.getPropertyValue("--gray-700").trim();
  var surface = style.getPropertyValue("--gray-900").trim();
  var plot = null;

  var teams = {};
  data.teams.forEach(function (t) { teams[t.number] = t; });
  var metrics = {};
  data.metrics.forEach(function (m, i) { metrics[m.key] = { label: m.label, index: i }; });

  // The chips redraw on their own.
  form.querySelector("[data-graph-show]").hidden = true;

  function say(text) { status.textContent = text; }

  function chips(kind) {
    return Array.prototype.slice.call(form.querySelectorAll('[data-kind="' + kind + '"] input'));
  }

  function slot(input) { return Number(input.closest(".chip").getAttribute("data-slot")) || 0; }

  // The chips turned on, in slot order.
  function chosen(kind) {
    return chips(kind)
      .filter(function (i) { return i.checked; })
      .map(function (i) { return { value: i.value, slot: slot(i) }; })
      .sort(function (a, b) { return a.slot - b.slot; });
  }

  function query(season) {
    var params = new URLSearchParams();
    var event = form.querySelector('input[name="event"]');
    if (event) params.append("event", event.value);
    params.append("chosen", "1");
    chosen("team").forEach(function (c) { params.append("team", c.value); });
    chosen("metric").forEach(function (c) { params.append("metric", c.value); });
    if (season) params.append("span", "season");
    return "?" + params.toString();
  }

  function ordinal(n) {
    var tens = n % 100, ones = n % 10;
    if (tens < 11 || tens > 13) {
      if (ones === 1) return n + "st";
      if (ones === 2) return n + "nd";
      if (ones === 3) return n + "rd";
    }
    return n + "th";
  }

  function cell(tag, text, className) {
    var el = document.createElement(tag);
    el.textContent = text;
    if (className) el.className = className;
    return el;
  }

  // The same tables the server draws, for what is on now.
  function redrawTables(onTeams, onMetrics) {
    tables.textContent = "";
    tables.parentNode.hidden = !onTeams.length;
    onTeams.forEach(function (t) {
      var team = teams[t.value];
      var table = document.createElement("table");
      table.className = "data-table";
      var caption = document.createElement("caption");
      var swatch = cell("span", "", "swatch");
      swatch.setAttribute("data-slot", t.slot);
      caption.appendChild(swatch);
      caption.appendChild(document.createTextNode(team.number + (team.name ? " · " + team.name : "")));
      table.appendChild(caption);
      var head = document.createElement("tr");
      var match = cell("th", "Match");
      match.scope = "col";
      head.appendChild(match);
      onMetrics.forEach(function (m) {
        var th = cell("th", metrics[m.value].label, "num");
        th.scope = "col";
        head.appendChild(th);
      });
      table.createTHead().appendChild(head);
      var body = table.createTBody();
      team.points.forEach(function (p) {
        var row = body.insertRow();
        var th = cell("th", p.match);
        th.scope = "row";
        row.appendChild(th);
        onMetrics.forEach(function (m) {
          var v = p.v[metrics[m.value].index];
          row.appendChild(cell("td", v === null ? "—" : String(v), "num"));
        });
      });
      tables.appendChild(table);
    });
  }

  function draw() {
    var onTeams = chosen("team").filter(function (t) { return teams[t.value]; });
    var onMetrics = chosen("metric").filter(function (m) { return metrics[m.value]; });
    redrawTables(onTeams, onMetrics);
    if (plot) { plot.destroy(); plot = null; }
    readout.hidden = true;
    root.setAttribute("data-lines", 0);
    if (!onTeams.length || !onMetrics.length) {
      say(onTeams.length ? "Tap a metric to draw it." : "Tap a team to draw it.");
      return;
    }

    var length = 0;
    onTeams.forEach(function (t) { length = Math.max(length, teams[t.value].points.length); });
    var xs = [];
    for (var i = 1; i <= length; i++) xs.push(i);
    var columns = [xs];
    var series = [{}];
    var colours = [null];
    onTeams.forEach(function (t) {
      var team = teams[t.value];
      var colour = style.getPropertyValue("--series-" + t.slot).trim();
      onMetrics.forEach(function (m) {
        var at = metrics[m.value].index;
        colours.push(colour);
        columns.push(xs.map(function (x) {
          var p = team.points[x - 1];
          return p ? p.v[at] : null;
        }));
        series.push({
          label: team.number + " · " + metrics[m.value].label,
          stroke: colour,
          width: 2,
          dash: DASH[m.slot - 1] || [],
          spanGaps: true,
          points: { size: 8, width: 2, fill: surface },
        });
      });
    });
    say("");

    var axis = {
      stroke: ink,
      grid: { stroke: rule, width: 1 },
      ticks: { stroke: rule, width: 1 },
    };
    plot = new uPlot({
      width: size().width,
      height: size().height,
      scales: { x: { time: false } },
      series: series,
      axes: [
        Object.assign({}, axis, {
          incrs: [1, 2, 5, 10],
          values: function (u, splits) { return splits.map(function (v) { return Number.isInteger(v) ? v : ""; }); },
        }),
        axis,
      ],
      // Tap to read, never drag: a drag would fight the page's scroll on a
      // phone, and zooming is not what this chart is for.
      // A tap still starts a selection box, which would be left on screen.
      cursor: {
        drag: { x: false, y: false, setScale: false },
        // A tapped match is marked with solid dots in each line's colour.
        points: { size: 10, fill: function (u, i) { return colours[i]; } },
        // The line sits on the match being read, not between two.
        move: function (u, left, top) {
          var idx = u.posToIdx(left);
          return [idx == null ? left : u.valToPos(u.data[0][idx], "x"), top];
        },
      },
      select: { show: false },
      // The chips are the legend; a tap fills in the readout below.
      legend: { show: false },
      hooks: { setCursor: [function (u) { read(u.cursor.idx, onTeams, onMetrics); }] },
    }, columns, root);
    // How many lines are drawn, for tests/browser/graph.mjs.
    root.setAttribute("data-lines", series.length - 1);

    // uPlot follows the mouse; on a touch screen, a tap moves its cursor.
    plot.over.addEventListener("pointerdown", function (e) {
      if (e.pointerType === "mouse") return;
      var box = plot.over.getBoundingClientRect();
      plot.setCursor({ left: e.clientX - box.left, top: e.clientY - box.top });
    });
  }

  // What the tapped match was: a row per team, a column per metric. The x
  // is each team's nth match, so each row names its own.
  function read(idx, onTeams, onMetrics) {
    if (idx == null) return;
    readout.textContent = "";
    readout.hidden = false;
    var table = document.createElement("table");
    table.className = "data-table";
    table.appendChild(cell("caption", "Their " + ordinal(idx + 1) + " scouted match"));
    var head = document.createElement("tr");
    var corner = cell("th", "Team");
    corner.scope = "col";
    head.appendChild(corner);
    onMetrics.forEach(function (m) {
      var th = cell("th", "", "num");
      th.scope = "col";
      var swatch = cell("span", "", "line-swatch");
      swatch.setAttribute("data-slot", m.slot);
      th.appendChild(swatch);
      th.appendChild(document.createTextNode(metrics[m.value].label));
      head.appendChild(th);
    });
    table.createTHead().appendChild(head);
    var body = table.createTBody();
    onTeams.forEach(function (t) {
      var p = teams[t.value].points[idx];
      var row = body.insertRow();
      var th = cell("th", "");
      th.scope = "row";
      var swatch = cell("span", "", "swatch");
      swatch.setAttribute("data-slot", t.slot);
      th.appendChild(swatch);
      th.appendChild(document.createTextNode(t.value));
      if (p) th.appendChild(cell("span", " " + p.match, "muted"));
      row.appendChild(th);
      onMetrics.forEach(function (m) {
        var v = p ? p.v[metrics[m.value].index] : null;
        row.appendChild(cell("td", v === null ? "—" : String(v), "num"));
      });
    });
    readout.appendChild(table);
  }

  function size() {
    var width = Math.max(root.clientWidth, 240);
    return { width: width, height: Math.round(Math.min(360, Math.max(220, width * 0.6))) };
  }

  form.addEventListener("change", function (e) {
    var input = e.target;
    // Other events' numbers are not on this page: that one is a navigation.
    if (input.name === "span") {
      location.search = query(input.checked);
      return;
    }
    var set = input.closest("[data-kind]");
    if (!set) return;
    var chip = input.closest(".chip");
    if (input.checked) {
      var taken = chips(set.getAttribute("data-kind"))
        .filter(function (i) { return i.checked && i !== input; })
        .map(slot);
      var max = Number(set.getAttribute("data-max"));
      if (taken.length >= max) {
        input.checked = false;
        say("Up to " + max + " at once. Tap one off first.");
        return;
      }
      var free = 1;
      while (taken.indexOf(free) >= 0) free++;
      chip.setAttribute("data-slot", free);
    } else {
      chip.setAttribute("data-slot", "0");
    }
    draw();
    var season = form.querySelector('input[name="span"]');
    history.replaceState(null, "", query(season && season.checked));
  });

  var resizing = 0;
  window.addEventListener("resize", function () {
    clearTimeout(resizing);
    resizing = setTimeout(function () { if (plot) plot.setSize(size()); }, 100);
  });

  draw();
})();
