// Pages made on the device (C5), end to end in a real Chromium, over the
// DevTools protocol. Not part of ./check.sh: it needs Chromium, sqlite3,
// Node 20+, and wasm-bindgen's CLI (see deploy/build-client.sh).
//
//   1. deploy/build-client.sh --debug && cargo build -p tt-web
//      mkdir -p /tmp/devp && cp target/debug/tt-web /tmp/devp/
//   2. node --experimental-websocket device-pages.mjs /tmp/devp [this machine's LAN IP]
//
// It starts and stops the server itself on port 18424, with a fresh database
// in the folder seeded by sqlite3, and prints PASS or FAIL per check. A
// scout opens the team page, the device takes its snapshot (S10), and the
// server goes away: the team page is then made by the service worker from the
// device's copy, and every other page is the offline shell. With a LAN
// address it also checks that plain http is left as it was (open decision 9).
import { spawn, execFileSync } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
const D = process.argv[2], LAN = process.argv[3], PORT = 18424, CDP = 9339;
const local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let server = null;
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
async function start() {
  server = spawn(`${D}/tt-web`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
  throw new Error("server did not start");
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }

await start();
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const today = new Date().toISOString().slice(0, 10);
let sql = `INSERT INTO events (tba_key, name, start_date, end_date, created_at, updated_at) VALUES ('2026demo', 'Demo Regional', '${today}', '${today}', 'x', 'x');\n`;
for (const [t, name] of [[254, "Poofs"], [1678, "Citrus"], [10101, "Teal"]]) {
  sql += `INSERT OR IGNORE INTO teams (team_number, name, created_at, updated_at) VALUES (${t}, '${name}', 'x', 'x');\n`;
  sql += `INSERT INTO event_teams (event_key, team_number, created_at) VALUES ('2026demo', ${t}, 'x');\n`;
}
sql += `INSERT INTO team_event_stats (team_number, event_key, opr, synced_at) VALUES (254, '2026demo', 61.25, '${new Date().toISOString()}');\n`;
sql += `INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, blue1, blue2, played, created_at, updated_at) VALUES ('2026demo_qm1', '2026demo', 'qm', 1, 254, 1678, 971, 10101, 1, 'x', 'x');\n`;
const payload = JSON.stringify({ auto_scored: 4, teleop_scored: 9, endgame: "full" });
sql += `INSERT INTO observations (client_record_id, match_key, team_number, event_key, alliance, payload, schema_version, review_state, observed_at, created_at, updated_at) VALUES ('r1', '2026demo_qm1', 254, '2026demo', 'red', '${payload}', 1, 'approved', 'x', 'x', 'x');\n`;
writeFileSync(`${D}/seed.sql`, sql);
execFileSync("sqlite3", [`${D}/t.db`, `.read ${D}/seed.sql`]);

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); const events = [];
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } else events.push(msg); });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;
const go = async (url, ms = 1500) => { await send("Page.navigate", { url }); await sleep(ms); };
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
const page = `({ url: location.pathname + location.search, device: !!document.getElementById('device-page'), shell: !!document.getElementById('offline-shell'),
  h1: document.querySelector('h1')?.textContent, text: document.querySelector('main')?.textContent.replace(/\\s+/g, ' '),
  signIn: !!document.querySelector('a[href="/sign-in"]'), bg: getComputedStyle(document.body).backgroundColor })`;
await send("Page.enable"); await send("Runtime.enable"); await send("Log.enable");
for (const url of [local, LAN && `http://${LAN}:${PORT}`].filter(Boolean)) await send("Network.setCookie", { name: "tt_session", value: session, url });

// 1. Online: the worker installs with the wasm module in its cache.
const team = `${local}/teams?event=2026demo&team=254`;
await go(team);
await evaluate(`navigator.serviceWorker.ready`);
await go(team);
const cached = await evaluate(`caches.keys().then(ks => caches.open(ks[0])).then(c => c.match('/static/client/tt_client_bg.wasm')).then(r => r && r.headers.get('content-type'))`);
check("the wasm module is precached", cached === "application/wasm", String(cached));
const online = await evaluate(page);
check("online, the page is the server's", !online.device && online.h1 === "254 · Poofs", JSON.stringify(online.h1));

// 2. No copy on the device yet: the shell, as before C5.
await stop();
await go(team);
const nocopy = await evaluate(page);
check("no copy yet: the offline shell", nocopy.shell && !nocopy.device, JSON.stringify({ shell: nocopy.shell, device: nocopy.device }));

// 3. The device takes its copy.
await start();
await go(team);
await evaluate(`new Promise(r => { const s = document.createElement("script"); s.src = "/static/js/snapshot.js"; s.onload = r; document.head.append(s); })`);
const snap = await evaluate(`ttSnapshot.bootstrap({ events: ["2026demo"] }).then(s => s.bytes.byteLength, e => 'refused: ' + e.reason)`);
check("the snapshot is on the device", typeof snap === "number" && snap > 0, String(snap));

// 4. Server gone: the team page is made on the device.
await stop();
events.length = 0;
const before = Date.now();
await go(team, 3000);
const made = await evaluate(page);
const took = await evaluate(`Math.round(performance.getEntriesByType('navigation')[0].responseEnd)`);
check("server gone: the team page is made on the device", made.device && made.h1 === "254 · Poofs" && made.url === "/teams?event=2026demo&team=254", JSON.stringify({ device: made.device, h1: made.h1, url: made.url }));
check("from the device's copy", /OPR\s*61\.25/.test(made.text) && /From 1 approved observation/.test(made.text) && /Citrus/.test(made.text), made.text.slice(0, 300));
check("and says so", /made on this device/.test(made.text));
check("styled, from the cache", made.bg === "rgb(3, 7, 18)", made.bg);
check("with no account links", !made.signIn);
console.log(`      (first page, wasm compiled: ${took} ms to the response; ${Date.now() - before} ms with the wait)`);
await sleep(1500);
const chip = await evaluate(`[...document.querySelectorAll('[data-link]')].filter(e => !e.hidden).map(e => e.textContent.trim())[0]`);
check("the chip says offline", /^Offline/.test(chip), chip);
// The refused /health checks are the browser's network log, not the page's.
const noise = events.filter((e) => e.method === "Runtime.exceptionThrown" || (e.method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(e.params.type))
  || (e.method === "Log.entryAdded" && e.params.entry.level === "error" && e.params.entry.source !== "network"));
check("no errors or warnings on the page", noise.length === 0, JSON.stringify(noise.map((e) => e.params?.entry?.text || e.params?.exceptionDetails?.text || e.params?.args?.[0]?.value)));

// 5. Another team, by the page's own form, and a team the copy lacks.
await evaluate(`(document.getElementById('team-number').value = '1678', document.querySelector('form.keypad').requestSubmit())`);
await sleep(1500);
const other = await evaluate(page);
check("the lookup form works offline", other.device && other.h1 === "1678 · Citrus", JSON.stringify({ url: other.url, h1: other.h1 }));
await go(`${local}/teams?event=2026demo&team=4`);
const missing = await evaluate(page);
check("a team the copy lacks says so", missing.device && /no team 4 on this server/.test(missing.text), missing.text.slice(0, 200));

// 6. A page the device cannot make is the shell.
await go(`${local}/lead-scout?event=2026demo`);
const lead = await evaluate(page);
check("any other page: the offline shell", lead.shell && !lead.device && lead.url === "/lead-scout?event=2026demo", JSON.stringify({ shell: lead.shell, url: lead.url }));

// 7. Server back: a device page reloads itself into the server's.
await go(team, 500);
await start();
await sleep(7000);
const back = await evaluate(page);
check("server back: the page reloaded from the server", !back.device && back.h1 === "254 · Poofs", JSON.stringify({ device: back.device, h1: back.h1 }));

// 8. Plain http from the LAN address: no worker, so nothing changes.
if (LAN) {
  await go(`http://${LAN}:${PORT}/teams?event=2026demo&team=254`);
  const lan = await evaluate(`({ secure: isSecureContext, sw: 'serviceWorker' in navigator, h1: document.querySelector('h1')?.textContent })`);
  check("LAN http: no service worker, the server's page", !lan.secure && !lan.sw && lan.h1 === "254 · Poofs", JSON.stringify(lan));
}

ws.close(); chrome.kill(); await stop(); process.exit(0);
