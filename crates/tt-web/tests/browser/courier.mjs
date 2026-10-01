// A lead scout's tablet fetching from TBA with its own signal (S7), end to
// end in a real Chromium, over the DevTools protocol. Not part of ./check.sh:
// it needs Chromium, sqlite3, Node 20+, and wasm-bindgen's CLI (see
// deploy/build-client.sh).
//
//   1. deploy/build-client.sh --debug && cargo build -p tt-web
//      mkdir -p /tmp/cour && cp target/debug/tt-web /tmp/cour/
//   2. node --experimental-websocket courier.mjs /tmp/cour
//
// It starts and stops the server itself on port 18426, with a fresh database
// in the folder seeded by sqlite3, and a stand-in for TBA on 18427 that
// answers a browser the way TBA does (CORS, ETags). The server is pointed at
// it with TBA_BASE_URL and hands its key to the lead scout's tablet. Then:
// the venue's wifi with no internet (the tablet gets the key, fetches
// nothing), the lobby (no server, but signal: the tablet fetches and keeps
// it), and back in the venue (the server gets it and applies it). Prints PASS
// or FAIL per check.
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { rmSync, writeFileSync } from "node:fs";
const D = process.argv[2], PORT = 18426, TBA = 18427, CDP = 9340;
const local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
let server = null;
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });

// ── TBA, on localhost ───────────────────────────────────────────────────────
const MATCHES = JSON.stringify([{ key: "2026demo_qm1", comp_level: "qm", set_number: 1, match_number: 1,
  alliances: { red: { score: 88, team_keys: ["frc254", "frc1678", "frc10101"] }, blue: { score: 74, team_keys: ["frc971", "frc4", "frc5"] } },
  winning_alliance: "red" }]);
const BODIES = { matches: MATCHES, oprs: JSON.stringify({ oprs: { frc254: 61.25 }, dprs: {}, ccwms: {} }),
  rankings: JSON.stringify({ rankings: [], sort_order_info: [] }), coprs: "{}" };
const tba = { asked: [], keys: new Set() };
const stub = createServer((req, res) => {
  res.setHeader("access-control-allow-origin", req.headers.origin || "*");
  res.setHeader("access-control-allow-headers", "x-tba-auth-key, if-none-match, accept");
  res.setHeader("access-control-expose-headers", "ETag");
  if (req.method === "OPTIONS") return res.writeHead(204).end();
  tba.asked.push(req.url);
  tba.keys.add(req.headers["x-tba-auth-key"]);
  const what = (req.url.match(/^\/event\/2026demo\/(\w+)$/) || [])[1];
  if (!BODIES[what]) return res.writeHead(404).end("null");
  res.writeHead(200, { "content-type": "application/json", etag: `W/"${what}-1"` }).end(BODIES[what]);
});
const signal = (on) => new Promise((r) => {
  if (on) return stub.listen(TBA, "127.0.0.1", r);
  stub.closeAllConnections();
  stub.close(r);
});

async function start() {
  server = spawn(`${D}/tt-web`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc`,
    TBA_AUTH_KEY: "the-key", TBA_BASE_URL: `http://127.0.0.1:${TBA}`, FIRST_API_USERNAME: "", FIRST_API_KEY: "" }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
  throw new Error("server did not start");
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }
const sql = (query) => execFileSync("sqlite3", [`${D}/t.db`, query]).toString().trim();

// The server with no event yet, so its own TBA loop asks for nothing.
await start();
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];
const today = new Date().toISOString().slice(0, 10);
let seed = `INSERT INTO events (tba_key, name, start_date, end_date, created_at, updated_at) VALUES ('2026demo', 'Demo Regional', '${today}', '${today}', 'x', 'x');\n`;
for (const [t, name] of [[254, "Poofs"], [1678, "Citrus"], [10101, "Teal"]]) {
  seed += `INSERT OR IGNORE INTO teams (team_number, name, created_at, updated_at) VALUES (${t}, '${name}', 'x', 'x');\n`;
  seed += `INSERT INTO event_teams (event_key, team_number, created_at) VALUES ('2026demo', ${t}, 'x');\n`;
}
seed += `INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, red3, blue1, played, created_at, updated_at) VALUES ('2026demo_qm1', '2026demo', 'qm', 1, 254, 1678, 10101, 971, 0, 'x', 'x');\n`;
writeFileSync(`${D}/seed.sql`, seed);
execFileSync("sqlite3", [`${D}/t.db`, `.read ${D}/seed.sql`]);

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map();
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;
const go = async (url, ms = 1500) => { await send("Page.navigate", { url }); await sleep(ms); };
await send("Page.enable"); await send("Runtime.enable");
await send("Network.setCookie", { name: "tt_session", value: session, url: local });
// What courier.js asks the worker, asked now rather than on its timer.
const tick = () => evaluate(`new Promise((resolve) => {
  const channel = new MessageChannel();
  channel.port1.onmessage = (e) => resolve(e.data);
  navigator.serviceWorker.controller.postMessage({ type: "tt-courier", bearer: window.ttToken && ttToken.token() }, [channel.port2]);
})`);

// 1. Signed in, with the worker and the device's copy.
const team = `${local}/teams?event=2026demo&team=254`;
await go(team);
await evaluate(`navigator.serviceWorker.ready`);
await go(team);
await evaluate(`new Promise(r => { const s = document.createElement("script"); s.src = "/static/js/snapshot.js"; s.onload = r; document.head.append(s); })`);
const snap = await evaluate(`ttSnapshot.bootstrap({ events: ["2026demo"] }).then(s => s.bytes.byteLength, e => 'refused: ' + e.reason)`);
check("the device has its copy", typeof snap === "number" && snap > 0, String(snap));
const lead = await evaluate(`document.querySelector('script[src="/static/js/courier.js"]')?.dataset.lead`);
check("the page says the scout leads", lead === "true", String(lead));

// 2. The venue's wifi, no internet: the key, and nothing fetched. courier.js
//    asks on its own a few seconds after the page opens.
await go(team, 500);
const first = await evaluate(`new Promise((resolve) => {
  document.addEventListener("tt:courier", (e) => resolve(e.detail), { once: true });
  setTimeout(() => resolve(null), 9000);
})`);
check("courier.js asks on its own, and reports", first && first.pi === true && first.key === true, JSON.stringify(first));
check("no internet: nothing fetched, nothing waits", first && first.skipped === "no signal" && first.fetched === 0 && first.waiting === 0, JSON.stringify(first));

// 3. The lobby: no server, but signal.
await stop();
await signal(true);
const lobby = await tick();
check("in the lobby the tablet fetches from TBA", lobby && lobby.pi === false && lobby.fetched === 4 && lobby.waiting === 4, JSON.stringify(lobby));
check("with the server's key, through TBA's CORS", tba.keys.size === 1 && tba.keys.has("the-key") && tba.asked.length === 4, JSON.stringify(tba));
const unplayed = sql(`SELECT COALESCE(red_score, 'none') FROM matches WHERE tba_key = '2026demo_qm1'`);
check("the server does not have it yet", unplayed === "none", unplayed);

// 4. Back in the venue, where the wifi has no internet again.
await signal(false);
await start();
await go(team, 500);
const back = await tick();
check("back in reach, the tablet hands it over", back && back.pi === true && back.pushed === 4 && back.waiting === 0 && !back.stopped, JSON.stringify(back));
check("not fetched again so soon", back && back.skipped === "fetched less than two minutes ago", JSON.stringify(back && back.skipped));
const played = sql(`SELECT red_score || '-' || blue_score FROM matches WHERE tba_key = '2026demo_qm1'`);
check("the server applied it: Q1 was 88-74", played === "88-74", played);
const pushed = sql(`SELECT u.name || ' ' || b.appended FROM bundle_imports b JOIN users u ON u.id = b.user_id`);
check("and logged who brought it", pushed === "Sam 4", pushed);
const again = await tick();
check("nothing waits, so nothing is pushed twice", again && again.pushed === 0 && again.waiting === 0, JSON.stringify(again));
const once = sql(`SELECT COUNT(*) FROM bundle_imports`);
check("one import", once === "1", once);

ws.close(); chrome.kill(); await stop(); process.exit(0);
