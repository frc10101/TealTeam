// The scouting form that holds what it cannot send (C13), end to end in a
// real Chromium over the DevTools protocol, over plain http. Not part of
// ./check.sh: it needs Chromium, Node 20+, and the sqlite3 shell.
//
//   cargo build -p tt-web
//   node --experimental-websocket outbox.mjs <empty folder> target/debug/tt-web
//   (SHOTS=1 in front also saves held.png and landed.png in the folder)
//
// It starts and stops the server itself on port 18426 with a fresh database.
// The page is opened as http://scout.lan:18426, mapped to 127.0.0.1 by
// Chromium's resolver, because 127.0.0.1 itself counts as a secure context
// and the event LAN's address does not: so no service worker, as at an event.
// The server is stopped under an open form, the form is saved twice (once for
// a robot not in the match), both are held, the server comes back, one lands
// and one is refused for the lead scout. Then a save whose session has run
// out goes with the offline token. Prints PASS or FAIL per check.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
const [D, BIN] = process.argv.slice(2).map((a) => resolve(a));
const PORT = Number(process.env.TT_PORT || 18426), CDP = Number(process.env.TT_CDP || 9341), local = `http://127.0.0.1:${PORT}`, lan = `http://scout.lan:${PORT}`;
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
const sql = (q) => execFileSync("sqlite3", [`${D}/t.db`, q]).toString().trim();

await start();
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40outbox.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const day = (n) => new Date(Date.now() + n * 864e5).toISOString().slice(0, 10);
const at = (h) => `${day(0)}T${h}:00:00.000Z`;
sql(`
  INSERT INTO events (tba_key, name, timezone, start_date, end_date, created_at, updated_at)
    VALUES ('2026c13', 'Held Open', 'America/Chicago', '${day(-1)}', '${day(1)}', 'x', 'x');
  INSERT INTO teams (team_number, name, created_at, updated_at) VALUES
    (10101, 'Teal Team', 'x', 'x'), (254, 'The Cheesy Poofs', 'x', 'x'), (1678, 'Citrus Circuits', 'x', 'x');
  INSERT INTO event_teams (event_key, team_number, created_at) VALUES
    ('2026c13', 10101, 'x'), ('2026c13', 254, 'x'), ('2026c13', 1678, 'x');
  INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, blue1, blue2, played, scheduled_at, created_at, updated_at) VALUES
    ('2026c13_qm1', '2026c13', 'qm', 1, 10101, 254, 1678, 2, 1, '${at(14)}', 'x', 'x'),
    ('2026c13_qm2', '2026c13', 'qm', 2, 254, 2, 10101, 1678, 0, '${at(15)}', 'x', 'x');
`);

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`,
  `--host-resolver-rules=MAP scout.lan 127.0.0.1`, "about:blank"], { stdio: "ignore" });
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
const shot = async (name) => {
  if (!process.env.SHOTS) return;
  const { data } = (await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true })).result;
  writeFileSync(`${D}/${name}.png`, Buffer.from(data, "base64"));
};
let failed = 0;
const check = (label, ok, detail = "") => { if (!ok) failed++; console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`); };
const until = async (expr, ms) => { for (let t = 0; t < ms; t += 250) { if (await evaluate(expr)) return true; await sleep(250); } return false; };
await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 760, deviceScaleFactor: 1, mobile: true });
await send("Network.enable");
await send("Network.setCookie", { name: "tt_session", value: session, url: lan });

// Every required answer given the plainest way, and the notes typed.
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
  return true; })()`);
const chip = () => evaluate(`(() => { const s = [...document.querySelectorAll('#link-chip [data-link]')].find(e => !e.hidden);
  return s ? s.dataset.link + ': ' + (s.querySelector('[data-link-label]') || s.firstChild).textContent.trim() : ''; })()`);
const chipKey = async () => (await chip()).split(":")[0];
const kept = () => evaluate(`JSON.parse(localStorage.getItem('tt-outbox:v1:1') || 'null')`);
const card = () => evaluate(`(() => { const b = document.getElementById('outbox'); return b && !b.hidden ? b.innerText : ''; })()`);

// 1. The form, over plain http.
await go(`${lan}/submission?event=2026c13&match=2026c13_qm2&team=254`);
check("the scouting form is open", await evaluate(`!!document.getElementById('scout-form')`));
check("  over plain http: no secure context, no service worker",
  (await evaluate(`isSecureContext`)) === false && (await evaluate(`!('serviceWorker' in navigator)`)));
check("  with the outbox script loaded", await evaluate(`!!window.ttOutbox`));
await until(`!!localStorage.getItem('tt-token:v2')`, 3000);
check("  and the offline token kept", await evaluate(`!!localStorage.getItem('tt-token:v2')`));
check("the chip says Synced", (await chipKey()) === "synced", await chip());

// 2. The Pi goes away under the open form; two saves are held.
await fill("held while the Pi was down");
await stop();
await evaluate(`document.getElementById('scout-form').requestSubmit()`);
await until(`!!document.getElementById('outbox') && !document.getElementById('outbox').hidden`, 12000);
const recordId = await evaluate(`document.querySelector('[name=record_id]').value`);
let text = await card();
check("the scout is told it is held, not saved", /Held on this tablet, not saved/.test(text) && /Q2 · 254/.test(text), text.replace(/\n/g, " | "));
check("  and the page stayed put", (await evaluate(`location.search`)).indexOf("saved=") < 0);
let box = await kept();
const entry = box?.observations?.[0];
check("the entry is in the device's outbox in the push's shape",
  box?.format === "tealteam-outbox" && box.observations.length === 1 && entry.client_record_id === recordId &&
  entry.match_key === "2026c13_qm2" && entry.team_number === 254 && entry.payload.notes === "held while the Pi was down" &&
  typeof entry.schema_version === "number" && !!entry.observed_at && entry.refused === null,
  JSON.stringify(entry));
check("  its answers typed as the form reads them",
  Object.values(entry?.payload || {}).every((v) => ["string", "number", "boolean"].includes(typeof v)) &&
  Object.values(entry?.payload || {}).some((v) => typeof v === "boolean"), JSON.stringify(entry?.payload));
await until(`(() => { const s = document.querySelector('#link-chip [data-link=held]'); return s && !s.hidden; })()`, 8000);
check("the chip says what is held", (await chip()) === "held: Offline · 1 held", await chip());

// The same page again, for a robot not in the match: the Pi will refuse it.
await evaluate(`(() => { const f = document.getElementById('scout-form');
  f.elements.team.value = '9999'; f.elements.record_id.value = '0190a000-0000-7000-8000-00000000c013';
  f.requestSubmit(); return true; })()`);
await until(`JSON.parse(localStorage.getItem('tt-outbox:v1:1')).observations.length === 2`, 12000);
check("a second save is held beside it", (await kept())?.observations?.length === 2);
await until(`document.querySelector('#link-chip [data-link=held]').textContent.includes('2 held')`, 8000);
check("  and the chip counts both", (await chip()) === "held: Offline · 2 held", await chip());
await shot("held");

// 3. The Pi comes back: what is held goes by itself.
await start();
const landed = await until(`!localStorage.getItem('tt-outbox:v1:1') ||
  JSON.parse(localStorage.getItem('tt-outbox:v1:1')).observations.every(e => e.refused)`, 20000);
check("once the server answers, the outbox is pushed", landed);
text = await card();
check("  the good one is saved, and the page says so", /Saved to the server: Q2 · 254/.test(text), text.replace(/\n/g, " | "));
check("  the other is refused with the server's reason", /Refused by the server: Q2 · 9999\. Not saved: team 9999 is not in 2026c13_qm2/.test(text));
check("  and nothing is held any more", !/Held on this tablet/.test(text));
const row = sql(`SELECT team_number, scouter_id, json_extract(payload, '$.notes') FROM observations WHERE client_record_id = '${recordId}'`);
check("the Pi has the observation, as Sam's", row === "254|1|held while the Pi was down", row);
check("  and the refusal waits for the lead scout", sql(`SELECT count(*) FROM refused_entries WHERE resolved_at IS NULL`) === "1");
box = await kept();
check("  the device keeps only the refused one", box?.observations?.length === 1 && !!box.observations[0].refused, JSON.stringify(box));
check("  its draft is gone with it", (await evaluate(`Object.keys(localStorage).filter(k => k.startsWith('tt-draft:')).length`)) === 0);
await until(`!document.querySelector('#link-chip [data-link=held]') || document.querySelector('#link-chip [data-link=held]').hidden`, 8000);
check("the chip leaves held", !["held", "offline"].includes(await chipKey()), await chip());
await shot("landed");
await evaluate(`document.querySelector('#outbox button').click()`);
check("the refused one can be dismissed on the device", !(await kept()));

// 4. A session that ran out: the server answers with sign-in, so the save
// is held and goes at once with the token.
await go(`${lan}/submission?event=2026c13&match=2026c13_qm2&team=1678`);
await fill("after the session ran out");
await send("Network.deleteCookies", { name: "tt_session", url: lan });
await evaluate(`document.getElementById('scout-form').requestSubmit()`);
await until(`/Saved to the server/.test(document.getElementById('outbox').innerText)`, 12000);
text = await card();
check("a lapsed session's save lands with the token", /Saved to the server: Q2 · 1678/.test(text), text.replace(/\n/g, " | "));
const next = await evaluate(`document.querySelector('#outbox a.btn')?.getAttribute('href')`);
check("  Continue goes where a save would have", next === "/submission?event=2026c13&match=2026c13_qm2&saved=1678", next);
check("  recorded as Sam", sql(`SELECT scouter_id FROM observations WHERE team_number = 1678`) === "1");
check("no errors on the page", errors.length === 0, errors.join(" / "));

ws.close(); chrome.kill(); await stop();
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
