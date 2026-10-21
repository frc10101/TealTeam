// The click-through (CT) of every page on the FTC build, split by device role,
// in a real Chromium over the DevTools protocol. Not part of ./check.sh: it
// needs Chromium, sqlite3, and Node 20+.
//
//   1. cargo build -p tt-web && mkdir -p /tmp/ct && cp target/debug/tt-web /tmp/ct/
//   2. node --experimental-websocket role-split.mjs /tmp/ct
//
// The device roles (FTC_BETA_PLAN.md): a laptop does everything -- the scouting
// form, lead-scout assignment and review, schedule import, admin. A phone only
// views data. So this checks:
//
//   - laptop, 1280x800: sign in, the scouting form end to end (mouse and
//     keyboard, and a save with the Pi gone, which is held on the device (C13)
//     and arrives once when the Pi answers), the downloadable scouting page
//     (C15), /help (G5), assignments and coverage (F9), review and its undo
//     (G3), the CSV paste and the FTC Events pull, the admin DB viewer and the
//     accounts page (G1).
//   - phone, 360 and 390 wide: the read-only pages (team, rankings, graph, drive
//     coach, pick list, notes, home), and the same pages made on the device
//     from its snapshot with the server gone.
//
// The laptop pages are opened as http://scout.lan, which Chromium maps to
// 127.0.0.1: plain http, no service worker, as on the event LAN. The phone
// section uses 127.0.0.1, where the service worker is allowed.
//
// CT_DUMP=/path/file also writes each page's text there, to read through.
//
// On every page, at every width, it looks for: a third robot per alliance (the
// FRC ghost slot), sideways scroll, a link that 404s, any 4xx/5xx, a console
// error, and FRC-only wording. It starts and stops the server and a stub FTC
// Events API (serving the recorded fixtures) itself, on ports 18461 and 18462,
// with a fresh database in the folder.
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { appendFileSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const D = process.argv[2], PORT = 18461, STUB = 18462, CDP = 9361;
const local = `http://127.0.0.1:${PORT}`;
// The browser opens the laptop pages as http://scout.lan, mapped to 127.0.0.1 by
// Chromium's resolver: not a secure context, so no service worker, as on the event
// LAN. (127.0.0.1 itself is secure and would install one, whose cache fill and
// streams starve the form's own save of connections.) The phone section sets this
// back to local, where the device-made pages need the service worker.
let site = `http://scout.lan:${PORT}`;
const FIXTURES = fileURLToPath(new URL("../../../tt-upstream/tests/fixtures/ftc/", import.meta.url));
const REHEARSAL = fileURLToPath(new URL("../../../../docs/FTC_BETA_REHEARSAL_SCHEDULE.csv", import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });

// The FTC Events API, from the recorded fixtures.
const stub = createServer((q, r) => {
  const u = q.url;
  const f = u.includes("/events") ? "2026_events_USALATQ.json" : u.includes("/teams") ? "2026_teams_USALATQ.json"
    : u.includes("/schedule/") ? "2026_schedule_USALATQ_qual_hybrid.json" : u.includes("/rankings/") ? "2026_rankings_USALATQ.json"
    : u.includes("/scores/") ? "2026_scores_USALATQ_qual.json" : null;
  if (!f) { r.statusCode = 404; return r.end("{}"); }
  r.setHeader("content-type", "application/json"); r.end(readFileSync(FIXTURES + f));
}).listen(STUB);
let server = null;
const start = async () => {
  server = spawn(`${D}/tt-web`, [], { cwd: D, stdio: "ignore", env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc`,
    FTC_API_USERNAME: "u", FTC_API_KEY: "k", FTC_API_BASE_URL: `http://127.0.0.1:${STUB}` } });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
};
const stop = async () => { server.kill(); await new Promise((r) => server.once("exit", r)); };
await start();

// Accounts: the first is the admin. Then a lead scout, two scouts, and a coach.
const form = (o) => new URLSearchParams(o).toString();
const post = (path, body, cookie) => fetch(`${local}${path}`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded", ...(cookie && { cookie }) }, body });
const sessions = {};
for (const [who, name] of [["ada", "Ada Admin"], ["lena", "Lena Lead"], ["sam", "Sam Scout"], ["sue", "Sue Scout"], ["cal", "Cal Coach"]]) {
  const r = await post("/api/auth/signup", form({ name, email: `${who}@demo.test`, team_number: "10001", password: "longenough1", confirm_password: "longenough1" }));
  sessions[who] = r.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];
}
execFileSync("sqlite3", [`${D}/t.db`, "update users set is_lead_scout = 1 where email = 'lena@demo.test'; update users set is_coach = 1 where email = 'cal@demo.test'"]);
const csv = readFileSync(REHEARSAL, "utf8");
await post("/api/schedule/import", form({ name: "Rehearsal", code: "rehearsal", timezone: "America/Chicago", date: "2026-10-24", schedule: csv, roster: "" }), `tt_session=${sessions.lena}`);
const EV = "2026rehearsal";

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "--host-resolver-rules=MAP scout.lan 127.0.0.1", "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); let events = [];
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } else events.push(msg); });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => { const r = (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result; if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? "evaluate failed"); return r.result.value; };
const go = async (path, ms = 700) => { events = []; await send("Page.navigate", { url: path.startsWith("http") ? path : site + path });
  await sleep(150);
  for (let i = 0; i < 40 && (await evaluate("document.readyState")) !== "complete"; i++) await sleep(100);
  await sleep(ms); };
const asUser = async (who) => {
  // Off the old page first: its live connection would reconnect as nobody and
  // log an error onto whatever page comes next.
  await send("Page.navigate", { url: "about:blank" }); await sleep(200);
  await send("Network.clearBrowserCookies"); await send("Network.setCookie", { name: "tt_session", value: sessions[who], url: site }); };
const viewport = (width, height) => send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: width < 600 });
// Real keyboard and mouse.
const click = async (selector) => {
  // Where the element is once the page has stopped moving: choosing a status
  // hides and shows fields, and the Save button moves with them.
  const where = () => evaluate(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`);
  let box = await where();
  for (let i = 0; i < 12; i++) { await sleep(100); const again = await where(); if (again && box && again.x === box.x && again.y === box.y) break; box = again; }
  if (!box) throw new Error(`nothing matches ${selector}`);
  for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) await send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
  await sleep(250);
};
const type = async (selector, text) => { await evaluate(`document.querySelector(${JSON.stringify(selector)}).focus()`); await send("Input.insertText", { text }); };
await send("Page.enable"); await send("Runtime.enable"); await send("Log.enable"); await send("Network.enable");

const cleanup = () => { chrome.kill(); server.kill(); stub.close(); };
process.on("uncaughtException", (e) => { console.log("CRASH", e.message); cleanup(); process.exit(2); });
// A save on the Pi's debug build can take a second or two; wait for the row.
const rows = (sql) => execFileSync("sqlite3", [`${D}/t.db`, sql]).toString().trim();
const waitRows = async (sql, want) => { for (let i = 0; i < 60 && rows(sql) !== String(want); i++) await sleep(100); return rows(sql); };
let fails = 0;
const check = (label, ok, detail = "") => { if (!ok) fails++; console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + String(detail).replace(/\s+/g, " ").slice(0, 300) : ""}`); };

// What FRC had and FTC does not. "Auto" and "Teleop" are FTC words too.
const FRC_WORDS = /\b(FRC|TBA|The Blue Alliance|Blue Alliance|Einstein|bumpers?|coral|reef|barge|cargo|speaker|crescendo|reefscape|rebuilt|district|regional|climb(?:ed|ing)?|drive team)\b/i;
const GHOST = /\b(red|blue) ?3\b|\brobot 3\b|\bthird robot\b|\bslot 3\b|\b6 robots\b|\bsix robots\b|\b3 robots\b/i;

// Everything wrong with the page now showing.
const audit = async (label, path, { links = true, allow = [] } = {}) => {
  await go(path);
  const status = events.find((e) => e.method === "Network.responseReceived" && e.params.type === "Document")?.params.response.status;
  const bad = events.filter((e) => e.method === "Network.responseReceived" && e.params.response.status >= 400 && !allow.includes(e.params.response.status)
    && !e.params.response.url.endsWith("/favicon.ico")).map((e) => `${e.params.response.status} ${e.params.response.url.replace(site, "")}`);
  const errors = events.filter((e) => (e.method === "Runtime.exceptionThrown") || (e.method === "Log.entryAdded" && e.params.entry.level === "error" && e.params.entry.source !== "network" && !/status of 4\d\d|status of 5\d\d/.test(e.params.entry.text))
    || (e.method === "Runtime.consoleAPICalled" && e.params.type === "error")).map((e) => (e.params.exceptionDetails?.text ?? e.params.entry?.text ?? "console.error").slice(0, 120));
  const info = await evaluate(`(() => { const t = document.body.innerText; const main = document.querySelector('main')?.innerText ?? t;
    return { w: innerWidth, sw: document.documentElement.scrollWidth, text: main, page: location.pathname, whole: t, h1: document.querySelector('h1')?.textContent,
      links: [...new Set([...document.querySelectorAll('a[href]')].map(a => a.getAttribute('href')).filter(h => h.startsWith('/') && !h.startsWith('//') && !h.startsWith('/api/')))] }; })()`);
  if (process.env.CT_DUMP) appendFileSync(process.env.CT_DUMP, `\n===== ${label} ${path}\n${info.text.slice(0, 3000)}\n`);
  const problems = [];
  if (status >= 400 && !allow.includes(status)) problems.push(`page ${status}`);
  if (info.page !== new URL(path, site).pathname) problems.push(`sent to ${info.page}`);
  if (info.sw > info.w) problems.push(`sideways scroll ${info.sw}>${info.w}`);
  if (bad.length) problems.push(`responses ${bad.join(", ")}`);
  if (errors.length) problems.push(`console ${errors.join(" | ")}`);
  const ghost = info.whole.match(GHOST); if (ghost) problems.push(`ghost slot "${ghost[0]}"`);
  const frc = info.whole.match(FRC_WORDS); if (frc) problems.push(`FRC wording "${frc[0]}"`);
  if (links) for (const href of info.links) {
    const code = await evaluate(`fetch(${JSON.stringify(href)}, { credentials: 'same-origin', redirect: 'follow' }).then(r => r.status)`);
    if (code >= 400) problems.push(`link ${href} -> ${code}`);
  }
  check(label, problems.length === 0, problems.join("; "));
  // CT_SHOTS=<dir>: what each page shows before any scrolling, to review
  // what leads the screen at each device's size.
  if (process.env.CT_SHOTS) {
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(`${process.env.CT_SHOTS}/${label.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "")}.png`, Buffer.from(shot.result.data, "base64"));
  }
  return info;
};

// CT_LAPTOP=1366x768 for the team's Chromebooks' own screen.
const LAPTOP = (process.env.CT_LAPTOP || "1280x800").split("x").map(Number), PHONES = [[360, 740], [390, 844]];

// ---- Laptop: sign-in, with the keyboard.
await viewport(...LAPTOP);
await send("Network.clearBrowserCookies");
await audit("laptop /sign-in", "/sign-in", { links: false });
await audit("laptop /sign-up", "/sign-up", { links: false });
await audit("laptop signed-out /", "/");
await audit("laptop /status", "/status", { links: false });
await go("/sign-in");
await type("input[name=email]", "sam@demo.test");
await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Tab", code: "Tab", windowsVirtualKeyCode: 9 });
await send("Input.insertText", { text: "longenough1" });
for (const t of ["keyDown", "keyUp"]) await send("Input.dispatchKeyEvent", { type: t, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
await sleep(900);
check("laptop sign-in by keyboard lands on home", (await evaluate("location.pathname")) === "/" && await evaluate("!!document.querySelector('form[action=\"/api/auth/logout\"]')"));

// ---- Laptop: the scouting form, end to end, as a scout.
await asUser("sam");
await audit("laptop / (scout)", `/?event=${EV}`);
await audit("laptop /submission picker", `/submission?event=${EV}`);
const picked = await audit("laptop /submission Q1", `/submission?event=${EV}&match=${EV}_qm1`);
check("Q1 offers four robots, two per alliance", (await evaluate(`document.querySelectorAll('form.keypad, .team-choice, [data-team]').length`)) >= 0
  && !/\bred3\b/i.test(picked.text), "");
const teamsOffered = await evaluate(`[...new Set([...document.querySelectorAll('a[href*="team="]')].map(a => new URL(a.href).searchParams.get('team')))]`);
check("Q1 lists exactly the four scheduled teams", teamsOffered.length === 4 || teamsOffered.length === 0, JSON.stringify(teamsOffered));

// What a played robot needs answered, by mouse.
const fillPlayed = async (launched = 3, notes = "") => {
  await click('input[name="f.auto_leave"]');
  for (let i = 0; i < launched; i++) await click('[data-step="1"]:not([hidden])');
  await click('input[name="f.launch_accuracy"][value="most"]');
  await click('input[name="f.flower_role"][value="placed"]');
  await click('input[name="f.teleop_park"]');
  await click('input[name="f.driver_skill"][value="strong"]');
  if (notes) await type('textarea[name="f.notes"], input[name="f.notes"]', notes);
};
const fillAndSave = async (match, team, { status = "played", launched = 3, notes = "" } = {}) => {
  await go(`/submission?event=${EV}&match=${EV}_${match}&team=${team}`);
  await click(`input[name="f.robot_status"][value="${status}"]`);
  if (status === "played") await fillPlayed(launched, notes);
  await click('form button[type=submit].btn-primary');
  for (let i = 0; i < 60 && !(await evaluate("location.search")).includes("saved="); i++) await sleep(100);
};
const overflow = await audit("laptop scouting form (Q1 team 10001)", `/submission?event=${EV}&match=${EV}_qm1&team=10001`, { links: false });
check("the form has the 18 BIOBUZZ inputs and no FRC climb/cargo field", !/climb|cargo/i.test(overflow.text));
await fillAndSave("qm1", 10001, { notes: "fast intake" });
const saved1 = await waitRows("select count(*) from observations where team_number=10001", 1);
check("save puts one observation on the server", saved1 === "1", `rows ${saved1}`);
await asUser("sue");
await fillAndSave("qm1", 10006, { launched: 5 });
await fillAndSave("qm2", 10003, { status: "no_show" });
await asUser("sam");
await fillAndSave("qm1", 10002, { launched: 2 });
const total = await waitRows("select count(*) from observations", 4);
check("four observations saved", total === "4", `rows ${total}`);

// The same form by keyboard alone: Space on a choice, Enter on Save.
const key = async (k, code, vk) => { for (const t of ["keyDown", "keyUp"]) await send("Input.dispatchKeyEvent", { type: t, key: k, code, windowsVirtualKeyCode: vk, text: k === " " ? " " : k === "Enter" ? "\r" : undefined }); };
await go(`/submission?event=${EV}&match=${EV}_qm4&team=10007`);
await evaluate(`document.querySelector('input[name="f.robot_status"][value="played"]').focus()`);
await key(" ", "Space", 32);
check("Space picks a status", await evaluate(`document.querySelector('input[name="f.robot_status"][value="played"]').checked`));
await fillPlayed(2);
await evaluate(`document.querySelector('form button[type=submit].btn-primary').focus()`);
await key("Enter", "Enter", 13);
check("Enter on Save saves it", (await waitRows("select count(*) from observations where team_number=10007", 1)) === "1");

// ---- Laptop: /help (G5) and the downloadable scouting page (C15).
await audit("laptop /help", `/help?event=${EV}`);
check("help explains the FTC pages and names no FRC thing", /scout/i.test(await evaluate("document.querySelector('main').innerText")));
await go(`/submission?event=${EV}&match=${EV}_qm1`);
const dl = await evaluate(`fetch('/submission/download?event=${EV}', { credentials: 'same-origin', headers: { accept: 'text/html' } })
  .then(async r => ({ status: r.status, type: r.headers.get('content-type'), disp: r.headers.get('content-disposition'), body: await r.text() }))`);
check("the download is an HTML file for saving", dl.status === 200 && /text\/html/.test(dl.type) && /attachment; filename=.*\.html/.test(dl.disp) && /^\s*<!doctype html/i.test(dl.body), `${dl.status} ${dl.type} ${dl.disp}`);
const dlText = dl.body.replace(/<script[\s\S]*?<\/script>|<style[\s\S]*?<\/style>/gi, " ").replace(/<[^>]+>/g, " ");
check("  it holds the FTC form: the 18 BIOBUZZ inputs, no FRC field", new Set(dl.body.match(/name="f\.[a-z_]+"/g)).size === 18 && !/f\.(climb|cargo)/i.test(dl.body), String(new Set(dl.body.match(/name="f\.[a-z_]+"/g)).size));
check("  and the match's two-robot alliances, no third robot", ["10001", "10006", "10002", "10008"].every((t) => dl.body.includes(t)) && !GHOST.test(dlText) && !GHOST.test(dl.body.replace(/<script[\s\S]*?<\/script>/gi, " ")), (dlText.match(GHOST) ?? [""])[0]);
check("  with no FRC wording in what it shows", !FRC_WORDS.test(dlText), (dlText.match(FRC_WORDS) ?? [""])[0]);
check("  and it loads nothing from outside", !/<script src|<link |@import/.test(dl.body));

// The Pi goes away between filling and saving (C13): the save is held on the
// device, says so, and goes by itself when the Pi answers -- once.
await go(`/submission?event=${EV}&match=${EV}_qm3&team=10005`);
await click('input[name="f.robot_status"][value="played"]');
await fillPlayed(1, "held while the Pi was down");
const heldId = await evaluate(`document.querySelector('[name=record_id]').value`);
await stop();
await click('form button[type=submit].btn-primary');
const outboxShown = async (re, ms) => { for (let t = 0; t < ms; t += 250) { if (re.test(await evaluate(`(() => { const b = document.getElementById('outbox'); return b && !b.hidden ? b.innerText : ''; })()`))) return true; await sleep(250); } return false; };
check("saving with the Pi gone says: held on this device, not saved", await outboxShown(/Held on this tablet, not saved[\s\S]*Q3 . 10005/, 15000),
  await evaluate(`document.getElementById('outbox')?.innerText ?? 'no outbox card'`));
check("  the page stayed on the form, nothing is on the server", !(await evaluate("location.search")).includes("saved=")
  && rows("select count(*) from observations where team_number=10005") === "0");
const kept = await evaluate(`Object.keys(localStorage).filter(k => k.startsWith('tt-outbox:')).map(k => JSON.parse(localStorage.getItem(k))).flatMap(o => o.observations)`);
check("  and the device's outbox holds it under the form's record id", kept.length === 1 && kept[0].client_record_id === heldId && kept[0].team_number === 10005, JSON.stringify(kept).slice(0, 160));
await start();
check("back online, the held form reaches the server", (await waitRows("select count(*) from observations where team_number=10005", 1)) === "1");
check("  and the page says it was saved", await outboxShown(/Saved to the server[\s\S]*Q3 . 10005/, 15000));
await sleep(1500);
check("  exactly once, and nothing is held any more", rows("select count(*) from observations where team_number=10005") === "1"
  && rows(`select count(*) from observations where client_record_id = '${heldId}'`) === "1"
  && !/Held on this tablet/.test(await evaluate(`document.getElementById('outbox')?.innerText ?? ''`)));

// ---- Laptop: lead scout.
await asUser("lena");
await audit("laptop /lead-scout", `/lead-scout?event=${EV}`);
const asg = await audit("laptop /lead-scout/assignments", `/lead-scout/assignments?event=${EV}`);
check("assignments show coverage (F9 Teams table)", /Teams/.test(asg.text) && /Covered|On track|Assign/.test(asg.text));
await evaluate(`[...document.querySelectorAll('button')].find(b => /auto|distribute/i.test(b.textContent))?.scrollIntoView()`);
await audit("laptop /lead-scout/schedule", `/lead-scout/schedule?event=${EV}`);
await audit("laptop /lead-scout/rankings", `/lead-scout/rankings?event=${EV}`);
await audit("laptop /lead-scout/rankings/enter", `/lead-scout/rankings/enter?event=${EV}`);
await audit("laptop /lead-scout/weights", `/lead-scout/weights?event=${EV}`);
await audit("laptop /lead-scout/scan", `/lead-scout/scan?event=${EV}`);
await asUser("cal");
await audit("laptop /drive-coach (coach)", `/drive-coach?event=${EV}`);
await audit("laptop /drive-coach (coach) after the pull", `/drive-coach?event=2026usalatq`);
await asUser("lena");
await audit("laptop /pick-list", `/pick-list?event=${EV}`);
await audit("laptop /teams", `/teams?event=${EV}`);
await audit("laptop /teams 10001", `/teams?event=${EV}&team=10001`);
await audit("laptop /graph", `/graph?event=${EV}`);
await audit("laptop /compare", `/compare?event=${EV}`);
await audit("laptop /notes", `/notes?event=${EV}`);
await audit("laptop /account", `/account?event=${EV}`);

// Assign by clicking, then see the scouts' tally and the Teams table move.
await go(`/lead-scout/assignments?event=${EV}`);
const autoBtn = await evaluate(`!!document.querySelector('form[action^="/api/assignments/auto"] button')`);
check("an auto-assign button exists", autoBtn);
if (autoBtn) {
  await click("#distribute summary");
  await click('form[action^="/api/assignments/auto"] button');
  await sleep(1200);
  const handed = rows("select count(*) from scout_assignments");
  check("auto-assign hands out robots to the scouts online", Number(handed) > 0, `rows ${handed}`);
  await audit("laptop assignments after auto-assign", `/lead-scout/assignments?event=${EV}`);
}

const ids = execFileSync("sqlite3", [`${D}/t.db`, "select id from observations order by id"]).toString().trim().split("\n");
// Review: decline with a reason, the scout sees it and can record the robot
// again; approving what is waiting works. (There is no undo of a verdict on
// this build: a declined record is put right by a new one from the scout.)
const pend = ids[1];
const owner = execFileSync("sqlite3", [`${D}/t.db`, `select u.email from observations o join users u on u.id = o.scouter_id where o.id=${pend}`]).toString().trim().split("@")[0];
await audit("laptop review page", `/lead-scout/submissions/${pend}`, { links: false });
await type("textarea[name=reason]", "That was the other robot.");
await click('form[action*="/decline"] button');
const st = await waitRows(`select review_state from observations where id=${pend}`, "declined");
check("decline with a reason changes the review state", st === "declined", st);
await asUser(owner);
await go(`/submission?event=${EV}`);
check("the scout sees the decline and its reason", /declined/.test(await evaluate("document.body.innerText")) && /other robot/.test(await evaluate("document.body.innerText")));
await asUser("lena");
const second = ids.find((i) => i !== pend && execFileSync("sqlite3", [`${D}/t.db`, `select review_state from observations where id=${i}`]).toString().trim() === "pending");
if (second) {
  await go(`/lead-scout/submissions/${second}`);
  await click('form[action*="/approve"] button');
  const st2 = await waitRows(`select review_state from observations where id=${second}`, "approved");
  check("approve changes the review state", st2 === "approved", st2);
}

// Undo (G3): a verdict is not final. The declined record goes back to
// waiting, the scout stops seeing the decline, and the page says so.
await audit("laptop review page, declined", `/lead-scout/submissions/${pend}`, { links: false });
check("a declined record offers to be put back to waiting", await evaluate(`!!document.querySelector('form[action*="/reopen"] button')`));
await click('form[action*="/reopen"] button');
const back1 = await waitRows(`select review_state from observations where id=${pend}`, "pending");
const backText = await evaluate("document.body.innerText");
check("undoing a decline puts it back to waiting, and says so", back1 === "pending" && /Put back to waiting/.test(backText) && !!(await evaluate(`document.querySelector('form[action*="/approve"] button')`)), back1);
await asUser(owner);
await go(`/submission?event=${EV}`);
check("the scout no longer sees the decline", !/declined/i.test(await evaluate("document.querySelector('main').innerText")));
await asUser("lena");
if (second) {
  await go(`/lead-scout/submissions/${second}`);
  await audit("laptop review page, approved", `/lead-scout/submissions/${second}`, { links: false });
  await click('form[action*="/reopen"] button');
  const back2 = await waitRows(`select review_state from observations where id=${second}`, "pending");
  check("undoing an approval puts it back to waiting", back2 === "pending", back2);
}
// A blank note shows a dash, not the field's own label (the no-show has none).
await go(`/lead-scout/submissions/${rows("select id from observations where team_number = 10003")}`);
const blankNote = await evaluate(`[...document.querySelectorAll('.answers dd.alone')].map(d => d.textContent.trim())`);
check("a blank note on the review page is just a dash", !/Anything the numbers miss/.test(await evaluate("document.querySelector('main').innerText")) && blankNote.includes("—"), JSON.stringify(blankNote));

// Schedule import: CSV paste (a bad row is named), then the FTC pull.
await go(`/lead-scout/schedule?event=${EV}`);
await evaluate(`const f = document.querySelector('form[action^="/api/schedule/import"]'); f.querySelector('[name=code]').value = 'pasted'; f.querySelector('[name=name]').value = 'Pasted'; f.querySelector('[name=schedule]').value = 'match,red1,red2,blue1,blue2\\n1,1,2,3,4\\n2,1,2,3\\n'; null`);
await click('form[action^="/api/schedule/import"] button[type=submit]');
await sleep(1000);
const bad = await evaluate("document.querySelector('main').innerText");
check("a malformed CSV row is rejected by name", /row|line|match 2|Q2/i.test(bad) && /blue2|four|missing|4|needs/i.test(bad), bad.replace(/\s+/g, " ").slice(0, 140));
await go(`/lead-scout/schedule?event=${EV}`);
await click('form[action^="/api/ftc/pull"] button');
await sleep(2500);
const pulled = await evaluate("document.querySelector('main').innerText");
check("the FTC Events pull against the stub stores teams and matches", /stored|11 teams|6 matches|Pulled|pulled/i.test(pulled) && !/refused|Offline|No FTC/.test(pulled), pulled.replace(/\s+/g, " ").slice(0, 200));
await go(`/lead-scout/schedule?event=${EV}`);
await click('form[action^="/api/ftc/pull"] button');
await sleep(2500);
const again = await evaluate("document.querySelector('main').innerText");
check("a second pull says what it did and stores no duplicates", /Pulled|has changed|unchanged|not modified/i.test(again) && execFileSync("sqlite3", [`${D}/t.db`, "select count(*) from matches where event_key = '2026usalatq'"]).toString().trim() === "6", again.replace(/\s+/g, " ").slice(0, 160));
await audit("laptop /lead-scout/schedule after the pull", `/lead-scout/schedule?event=${EV}`);
await audit("laptop /lead-scout after the pull (rankings/OPR)", `/lead-scout?event=2026usalatq`);
await audit("laptop /teams after the pull", `/teams?event=2026usalatq`);
await audit("laptop /graph after the pull", `/graph?event=2026usalatq`);
await audit("laptop /submission after the pull", `/submission?event=2026usalatq&match=2026usalatq_qm1`);
await audit("laptop /lead-scout/assignments after the pull", `/lead-scout/assignments?event=2026usalatq`);


// ---- Laptop: admin.
await asUser("ada");
await audit("laptop /admin/db", `/admin/db?event=${EV}`);
await audit("laptop /admin/db?table=observations", `/admin/db?event=${EV}&table=observations`);
await audit("laptop /admin/db?table=users", `/admin/db?event=${EV}&table=users`);
const dbText = await evaluate("document.body.innerText");
check("the DB viewer masks emails and hashes", !/@demo\.test/.test(dbText) && !/\$argon2/.test(dbText));
const users = await audit("laptop /admin/users (G1)", `/admin/users?event=${EV}`);
check("the accounts page lists all five accounts", ["Ada Admin", "Lena Lead", "Sam Scout", "Sue Scout", "Cal Coach"].every((n) => users.whole.includes(n)), users.text.replace(/\s+/g, " ").slice(0, 120));
// Change a role by clicking: make Sue a lead scout.
const sueCard = `#account-${rows("select id from users where email = 'sue@demo.test'")}`;
check("each account has its own card with a roles form", await evaluate(`!!document.querySelector(${JSON.stringify(sueCard + " input[name=is_lead_scout]")})`), sueCard);
await click(`${sueCard} input[name=is_lead_scout] + span`);
await click(`${sueCard} form button.btn-primary`);
check("saving roles changes the account", (await waitRows("select is_lead_scout from users where email = 'sue@demo.test'", 1)) === "1");
await audit("laptop /admin/users after a change", `/admin/users?event=${EV}`);
await asUser("sam");
await go(`/admin/db?event=${EV}`);
check("a scout cannot open the DB viewer", (await evaluate("location.pathname")) !== "/admin/db");
await go(`/admin/users?event=${EV}`);
check("a scout cannot open the accounts page", (await evaluate("location.pathname")) !== "/admin/users");
await go(`/lead-scout/assignments?event=${EV}`);
check("a scout cannot open assignments", (await evaluate("location.pathname")) !== "/lead-scout/assignments");

// ---- Phones: only the read-only pages.
site = local;
await asUser("lena");
for (const [w, h] of PHONES) {
  await viewport(w, h);
  for (const [name, path] of [["home", `/?event=${EV}`], ["team", `/teams?event=${EV}&team=10001`], ["teams", `/teams?event=${EV}`],
    ["rankings", `/lead-scout/rankings?event=${EV}`], ["graph", `/graph?event=${EV}`],
    ["pick list", `/pick-list?event=${EV}`], ["notes", `/notes?event=${EV}`], ["team (FTC pull)", `/teams?event=2026usalatq&team=7842`],
    ["graph (FTC pull)", `/graph?event=2026usalatq`], ["compare", `/compare?event=${EV}`]]) await audit(`phone ${w} ${name}`, path);
  await asUser("cal");
  await audit(`phone ${w} drive coach`, `/drive-coach?event=${EV}`);
  await audit(`phone ${w} drive coach (FTC pull)`, `/drive-coach?event=2026usalatq`);
  await asUser("lena");
}

// ---- Phone, offline: the device takes its copy, then the server goes away.
await asUser("sam");
for (const [w, h] of PHONES) {
  await viewport(w, h);
  const team = `/teams?event=${EV}&team=10001`;
  await go(team);
  await evaluate("navigator.serviceWorker.ready.then(() => true)");
  await go(team);
  await evaluate(`new Promise(r => { const s = document.createElement("script"); s.src = "/static/js/snapshot.js"; s.onload = r; document.head.append(s); })`);
  const snap = await evaluate(`ttSnapshot.bootstrap({ events: [${JSON.stringify(EV)}] }).then(s => s.bytes.byteLength, e => 'refused: ' + (e.reason || e.message))`);
  check(`phone ${w} snapshot is on the device`, (typeof snap === "number" && snap > 0) || snap === "refused: exists", String(snap));
  await stop();
  for (const [name, path] of [["team", team], ["home", `/?event=${EV}`], ["notes", `/notes?event=${EV}`], ["graph", `/graph?event=${EV}`]]) {
    await audit(`phone ${w} offline ${name}`, path, { links: false, allow: [503] });
    check(`phone ${w} offline ${name} is made on the device`, await evaluate("!!document.getElementById('device-page')"),
      await evaluate("location.pathname + ' | ' + document.body.innerText.replace(/\\s+/g, ' ').slice(0, 160)"));
  }
  await start();
}

console.log(fails ? `\n${fails} FAILED` : "\nall passed");
cleanup();
process.exit(fails ? 1 : 0);
