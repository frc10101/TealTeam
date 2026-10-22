// Scouting on with no server (OF1), end to end in a real Chromium over the
// DevTools protocol, in a secure context so the service worker and the wasm
// client run. Not part of ./check.sh: it needs Chromium, Node 20+, and the
// sqlite3 shell.
//
//   deploy/build-client.sh && cargo build -p tt-web
//   node --experimental-websocket offline-scouting.mjs <empty folder> target/debug/tt-web
//
// It starts the server itself on port 18431 with a fresh database, and opens
// it as http://127.0.0.1, which counts as a secure context, as https with the
// CA trusted (H1) does at an event. "Offline" is the server process stopped:
// DevTools' offline emulation applies to the page, not to the service
// worker, whose fetches would still reach a running server.
//
// The scout opens their form once with the server up, so the device has its
// copy and knows who scouts here. Then, with the server stopped: the open
// form saves (outbox.js's fetch, answered by the worker); the next two
// assignments are opened from the made pages' links and saved the plain
// way; a reload, the team page, the graph and help are made on the device; a
// lead page is the offline shell; and the chip says "3 held" on every one.
// The server comes back, and the Pi has the three observations, once each.
// Prints PASS or FAIL per check.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { resolve } from "node:path";
const [D, BIN] = process.argv.slice(2).map((a) => resolve(a));
const PORT = Number(process.env.TT_PORT || 18431), CDP = Number(process.env.TT_CDP || 9352), local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(D, { recursive: true });
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });

let server = null;
async function start() {
  server = spawn(BIN, [], { cwd: D, env: { ...process.env, PORT: String(PORT), TLS_PORT: "off", DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
  throw new Error("server did not start");
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }
const sql = (q) => execFileSync("sqlite3", [`${D}/t.db`, q]).toString().trim();

await start();
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40of1.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const day = (n) => new Date(Date.now() + n * 864e5).toISOString().slice(0, 10);
const at = (h) => `${day(0)}T${h}:00:00.000Z`;
sql(`
  INSERT INTO events (tba_key, name, timezone, start_date, end_date, created_at, updated_at)
    VALUES ('2026of1', 'Scouting On', 'America/Chicago', '${day(-1)}', '${day(1)}', 'x', 'x');
  INSERT INTO teams (team_number, name, created_at, updated_at) VALUES
    (10101, 'Teal Team', 'x', 'x'), (254, 'The Cheesy Poofs', 'x', 'x'), (1678, 'Citrus Circuits', 'x', 'x'),
    (2, 'Two', 'x', 'x'), (3, 'Three', 'x', 'x'), (4, 'Four', 'x', 'x');
  INSERT INTO event_teams (event_key, team_number, created_at) VALUES
    ('2026of1', 10101, 'x'), ('2026of1', 254, 'x'), ('2026of1', 1678, 'x'), ('2026of1', 2, 'x'), ('2026of1', 3, 'x'), ('2026of1', 4, 'x');
  INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, blue1, blue2, played, scheduled_at, created_at, updated_at) VALUES
    ('2026of1_qm1', '2026of1', 'qm', 1, 10101, 254, 1678, 2, 1, '${at(14)}', 'x', 'x'),
    ('2026of1_qm2', '2026of1', 'qm', 2, 254, 2, 10101, 1678, 0, '${at(15)}', 'x', 'x'),
    ('2026of1_qm3', '2026of1', 'qm', 3, 1678, 3, 4, 254, 0, '${at(16)}', 'x', 'x'),
    ('2026of1_qm4', '2026of1', 'qm', 4, 2, 4, 3, 10101, 0, '${at(17)}', 'x', 'x');
  INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, assigned_by, created_at, updated_at) VALUES
    ('2026of1_qm2', 254, '2026of1', 1, 1, 'x', 'x'),
    ('2026of1_qm3', 1678, '2026of1', 1, 1, 'x', 'x'),
    ('2026of1_qm4', 2, '2026of1', 1, 1, 'x', 'x');
`);

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); const errors = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); }
  else if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description);
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => {
  const r = (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result;
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
let failed = 0;
const check = (label, ok, detail = "") => { if (!ok) failed++; console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`); };
const until = async (expr, ms) => { for (let t = 0; t < ms; t += 250) { try { if (await evaluate(expr)) return true; } catch {} await sleep(250); } return false; };
await send("Page.enable"); await send("Runtime.enable"); await send("Network.enable");
await send("Network.setCookie", { name: "tt_session", value: session, url: local });

const fill = (notes) => evaluate(`(() => {
  const form = document.getElementById('scout-form');
  const groups = new Set();
  for (const el of form.elements) {
    if (!el.required || !el.name.startsWith('f.')) continue;
    if (el.type === 'radio' && !groups.has(el.name)) { groups.add(el.name); el.checked = true; }
    else if (el.type === 'number' && el.value === '') el.value = el.min || '0';
    else if (el.tagName === 'TEXTAREA' && !el.value) el.value = 'x';
  }
  const box = form.querySelector('textarea[name="f.notes"]'); box.value = ${JSON.stringify(notes)};
  box.dispatchEvent(new Event('input', { bubbles: true }));
  return form.elements.match.value + ':' + form.elements.team.value; })()`);
const chip = () => evaluate(`(() => { const s = [...document.querySelectorAll('#link-chip [data-link]')].find(e => !e.hidden);
  return s ? s.dataset.link + ': ' + (s.querySelector('[data-link-label]') || s.firstChild).textContent.trim() : ''; })()`);
const madeHere = () => evaluate(`!!document.getElementById('device-page')`);
const saved = () => evaluate(`(() => { const s = document.querySelector('.alert-success'); return s ? s.innerText : ''; })()`);
// Submit, then wait for the next page to land.
const submit = async (how) => {
  await evaluate(`(window.__left = false, addEventListener('pagehide', () => { window.__left = true; }), true)`);
  await evaluate(how === "plain" ? `HTMLFormElement.prototype.submit.call(document.getElementById('scout-form'))`
    : `document.getElementById('scout-form').requestSubmit()`);
  await sleep(2500);
};

// 1. Online once: the form, the worker, the device's copy, and who scouts here.
await go(`${local}/submission?event=2026of1`);
check("online: the server's form for the first assignment", !(await madeHere()) && (await evaluate(`!!document.getElementById('scout-form')`)));
check("  in a secure context, with a service worker", await until(`isSecureContext && !!navigator.serviceWorker.controller`, 10000));
const ready = await until(`navigator.storage.getDirectory().then(d => d.getFileHandle('tealteam.sqlite3')).then(() => true, () => false)`, 15000);
check("  the device's copy is in OPFS", ready);
await until(`!!localStorage.getItem('tt-token:v2')`, 5000);
// The first sync (3 s in) keeps who scouts here; give it a moment.
await sleep(5000);
check("  the chip says Synced", (await chip()).startsWith("synced"), await chip());

// 2. The server goes. The open form saves through outbox.js and the worker.
const first = await fill("of1 one");
await stop();
await submit("fetch");
check("the open form saved with no server: a made confirmation", await madeHere(), await evaluate("location.href"));
check("  that says it is held on this tablet", (await saved()).includes("on this tablet"), await saved());
check("  for " + first, (await evaluate("location.search")).includes("saved=254"));
check("  and nothing in the page outbox: one queue", (await evaluate(`localStorage.getItem('tt-outbox:v1:1')`)) === null);
await until(`[...document.querySelectorAll('#link-chip [data-link]')].some(e => !e.hidden && e.dataset.link === 'held')`, 8000);
check("  the chip says 1 held", (await chip()) === "held: Offline · 1 held", await chip());

// 3. The next assignment, from the made page's link, saved the plain way.
const next = await evaluate(`(() => { const a = [...document.querySelectorAll('a')].find(a => /team 1678 in Q3/.test(a.textContent)); return a ? a.href : ''; })()`);
check("the made page offers the next assignment, Q3 · 1678", !!next, next);
await go(next);
check("  which opens offline, made on the device", (await madeHere()) && (await evaluate(`!!document.getElementById('scout-form')`)));
const second = await fill("of1 two");
await submit("plain");
check("  and saves: " + second, (await saved()).includes("1678"), await saved());
check("  the chip says 2 held", (await chip()) === "held: Offline · 2 held", await chip());

// 4. The one after, from the scouting tab with no match named: the agenda
// moved on locally.
await go(`${local}/submission?event=2026of1`);
const third = await evaluate(`(() => { const f = document.getElementById('scout-form'); return f ? f.elements.match.value + ':' + f.elements.team.value : ''; })()`);
check("Scout with nothing named opens the next one, Q4 · 2", third === "2026of1_qm4:2", third);
await fill("of1 three");
await submit("plain");
check("  and saves", (await saved()).includes("team 2"), await saved());

// 5. One per robot per match, on the device too.
await go(`${local}/submission?event=2026of1&match=2026of1_qm2&team=254`);
check("a robot already saved is not offered again", (await evaluate(`document.body.innerText`)).includes("already recorded team 254"));

// 6. Reload, and the other pages, with the server still gone.
await send("Page.reload"); await sleep(2000);
check("a reload is made on the device", await madeHere());
check("  the chip still says 3 held", (await chip()) === "held: Offline · 3 held", await chip());
for (const [path, what] of [["/teams?event=2026of1&team=254", "the team page"], ["/graph?event=2026of1", "the graph"], ["/help", "help"]]) {
  await go(local + path);
  check(`${what} is made on the device`, await madeHere(), await evaluate("document.title"));
  check(`  and its chip says 3 held`, (await chip()) === "held: Offline · 3 held", await chip());
}
await go(`${local}/lead-scout?event=2026of1`);
check("a lead page is the offline shell", await evaluate(`!!document.getElementById('offline-shell')`));
check("  whose chip says 3 held too", (await chip()) === "held: Offline · 3 held", await chip());

// 7. The server comes back: the made page reloads into the server's, and the
// device's outbox goes, once.
await start();
await go(`${local}/submission?event=2026of1`);
check("back: the server's page", !(await madeHere()));
await until(`[...document.querySelectorAll('#link-chip [data-link]')].some(e => !e.hidden && e.dataset.link === 'synced')`, 30000);
check("  the chip says Synced", (await chip()).startsWith("synced"), await chip());
const rows = sql(`SELECT n || ':' || c FROM (SELECT json_extract(payload, '$.notes') AS n, COUNT(*) AS c FROM observations GROUP BY n) ORDER BY 1`);
check("the Pi has the three, once each", rows === "of1 one:1\nof1 three:1\nof1 two:1", JSON.stringify(rows));
check("  as Sam's, for the right robots", sql(`SELECT group_concat(match_key || ':' || team_number || ':' || scouter_id, ' ') FROM (SELECT * FROM observations ORDER BY match_key)`) ===
  "2026of1_qm2:254:1 2026of1_qm3:1678:1 2026of1_qm4:2:1");
check("no script errors", errors.length === 0, errors.join(" | "));

chrome.kill(); await stop();
console.log(failed ? `${failed} FAILED` : "all passed");
process.exit(failed ? 1 : 0);
