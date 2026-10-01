// A scout's assignments kept on the device (C8), end to end in a real
// Chromium over the DevTools protocol. Not part of ./check.sh: it needs
// Chromium, Node 20+, and the sqlite3 shell.
//
//   cargo build -p tt-web
//   node --experimental-websocket agenda.mjs <empty folder> target/debug/tt-web [LAN IP]
//   (SHOTS=1 in front also saves page.png and offline.png in the folder)
//
// It starts and stops the server itself on port 18421 with a fresh database,
// seeds an event with sqlite3, and prints PASS or FAIL per check. With a LAN
// address it also checks the copy is kept over plain http, where there is no
// service worker to show the offline page.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
const [D, BIN, LAN] = process.argv.slice(2).map((a, i) => (i < 2 ? resolve(a) : a));
const PORT = 18421, CDP = 9336, local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(D, { recursive: true });
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });

let server = null;
async function start() {
  server = spawn(BIN, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
  throw new Error("server did not start");
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }

await start();
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40agenda.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

// Q1 played, Q2 and Q3 to come. Sam (user 1) has 254 in all of Q1-Q2 and
// 1678 in Q3, and recorded none of them.
const day = (n) => new Date(Date.now() + n * 864e5).toISOString().slice(0, 10);
const at = (h) => `${day(0)}T${h}:00:00.000Z`;
execFileSync("sqlite3", [`${D}/t.db`, `
  INSERT INTO events (tba_key, name, timezone, start_date, end_date, created_at, updated_at)
    VALUES ('2026c8', 'Offline Open', 'America/Chicago', '${day(-1)}', '${day(1)}', 'x', 'x');
  INSERT INTO teams (team_number, name, created_at, updated_at) VALUES
    (10101, 'Teal Team', 'x', 'x'), (254, 'The Cheesy Poofs', 'x', 'x'), (1678, 'Citrus Circuits', 'x', 'x');
  INSERT INTO event_teams (event_key, team_number, created_at) VALUES
    ('2026c8', 10101, 'x'), ('2026c8', 254, 'x'), ('2026c8', 1678, 'x');
  INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, red3, blue1, blue2, blue3, played, scheduled_at, created_at, updated_at) VALUES
    ('2026c8_qm1', '2026c8', 'qm', 1, 10101, 254, 1, 1678, 2, 3, 1, '${at(14)}', 'x', 'x'),
    ('2026c8_qm2', '2026c8', 'qm', 2, 1, 2, 254, 10101, 3, 4, 0, '${at(15)}', 'x', 'x'),
    ('2026c8_qm3', '2026c8', 'qm', 3, 1, 2, 3, 4, 1678, 10101, 0, '${at(16)}', 'x', 'x');
  INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, created_at, updated_at) VALUES
    ('2026c8_qm1', 254, '2026c8', 1, 'x', 'x'), ('2026c8_qm2', 254, '2026c8', 1, 'x', 'x'),
    ('2026c8_qm3', 1678, '2026c8', 1, 'x', 'x');
`]);

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); const logs = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); }
  else if (msg.method === "Runtime.exceptionThrown") logs.push(msg.params.exceptionDetails.exception?.description);
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => {
  const r = (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result;
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
// SHOTS=1 saves each view as a PNG in the folder, to look at.
const shot = async (name) => {
  if (!process.env.SHOTS) return;
  const { data } = (await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true })).result;
  writeFileSync(`${D}/${name}.png`, Buffer.from(data, "base64"));
};
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 760, deviceScaleFactor: 1, mobile: true });
for (const url of LAN ? [local, `http://${LAN}:${PORT}`] : [local]) await send("Network.setCookie", { name: "tt_session", value: session, url });
const kept = () => evaluate(`JSON.parse(localStorage.getItem('tt-agenda:v1'))`);
const rows = (sel) => evaluate(`[...document.querySelectorAll('${sel} .duty')].map(d => d.innerText.replace(/\\s+/g, ' ').trim())`);

// 1. The scouting page lists every robot to come and keeps the list.
await go(`${local}/submission?event=2026c8`);
await evaluate(`navigator.serviceWorker.ready`);
const page = await rows("#my-assignments");
check("the page lists Q2 and Q3, in order", page.length === 2 && /^Q2 · Red 3/.test(page[0]) && /^Q3 · Blue 2/.test(page[1]), JSON.stringify(page));
check("each row has the event's clock time", page.every((r) => /\d:\d\d [AP]M C[DS]T/.test(r)), JSON.stringify(page));
const copy = await kept();
check("the list is kept on the device", copy && copy.scout === "Sam" && copy.upcoming.length === 2 && copy.missed.length === 1 && copy.savedAt > 0, JSON.stringify(copy));
await shot("page");
const wide = await evaluate(`document.documentElement.scrollWidth <= innerWidth`);
check("no sideways scroll at 390px", wide);

// 2. The lead changes one of these robots while the page is open (S9).
await evaluate(`document.dispatchEvent(new CustomEvent('tt:assignment-change', { detail: { entity: 'assignment', op: 'delete', entity_pk: '2026c8_qm9:1' } })), 0`);
check("someone else's change leaves the copy alone", !(await kept()).changedAt);
await evaluate(`document.dispatchEvent(new CustomEvent('tt:assignment-change', { detail: { entity: 'assignment', op: 'delete', entity_pk: '2026c8_qm3:1678' } })), 0`);
check("removing one of Sam's robots marks the copy changed", (await kept()).changedAt > 0);

// 3. The server is gone: the offline page shows the kept list.
await stop();
await go(`${local}/submission?event=2026c8`);
const shell = await evaluate(`({ shell: !!document.getElementById('offline-shell'), shown: !document.getElementById('offline-agenda').hidden,
  text: document.getElementById('offline-agenda').innerText, links: document.querySelectorAll('#offline-agenda a').length })`);
check("the offline page shows the list", shell.shell && shell.shown, JSON.stringify(shell));
const off = await rows("#offline-agenda");
await shot("offline");
check("the same rows as the page", JSON.stringify(off.slice(0, 2)) === JSON.stringify(page), JSON.stringify(off));
check("it says whose and how old", /Sam's list at Offline Open, as of \d/.test(shell.text), shell.text);
check("it says the lead changed it since", /changed your assignments at/.test(shell.text), shell.text);
check("the missed robot is under Still to record", /Still to record/.test(shell.text) && /^Q1 · Red 2/.test(off[2] || ""), JSON.stringify(off));
check("no links to pages that would not load", shell.links === 0);

// 4. Back online, the page replaces the copy; signing out (last) throws it away.
await start();
await go(`${local}/submission?event=2026c8`);
check("a fresh copy clears the changed mark", !(await kept()).changedAt);
// 5. Plain http, as on the event LAN: no worker, but the copy is still kept.
if (LAN) {
  await go(`http://${LAN}:${PORT}/submission?event=2026c8`);
  const lan = await evaluate(`({ sw: 'serviceWorker' in navigator, kept: !!localStorage.getItem('tt-agenda:v1') })`);
  check("over plain http the list is kept anyway", !lan.sw && lan.kept, JSON.stringify(lan));
  await go(`${local}/submission?event=2026c8`);
}
await evaluate(`document.querySelector('form[action="/api/auth/logout"] button').click(), 0`);
await sleep(1500);
check("signing out removes the copy", (await evaluate(`localStorage.getItem('tt-agenda:v1')`)) === null);
check("no script errors", logs.length === 0, logs.join(" | "));

ws.close(); chrome.kill(); await stop();
