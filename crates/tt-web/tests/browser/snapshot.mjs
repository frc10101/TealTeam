// The snapshot bootstrap (S10), in a real Chromium, over the DevTools
// protocol. Not part of ./check.sh: it needs Chromium and Node 20+.
//
//   1. cargo build -p tt-web && mkdir -p /tmp/snap && cp target/debug/tt-web /tmp/snap/
//   2. node --experimental-websocket snapshot.mjs /tmp/snap [this machine's LAN IP]
//
// It starts and stops the server itself on port 18421, with a fresh database
// in the folder, and prints PASS or FAIL per check. With a LAN address it
// also checks that plain http is refused cleanly (open decision 9).
import { spawn } from "node:child_process";
import { rmSync } from "node:fs";
const D = process.argv[2], LAN = process.argv[3], PORT = 18421, CDP = 9336;
const local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
const server = spawn(`${D}/tt-web`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); break; } catch { await sleep(100); } }
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map();
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1200); };
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
const load = `new Promise(r => { const s = document.createElement("script"); s.src = "/static/js/snapshot.js"; s.onload = r; document.head.append(s); })`;
const reason = (call) => `${call}.then(() => "resolved", e => e.reason)`;
// The OPFS copy: its size, its first 16 bytes, and whether a string is in it.
const opfs = (needle) => `navigator.storage.getDirectory().then(d => d.getFileHandle("tealteam.sqlite3")).then(h => h.getFile()).then(async f => {
  const b = new Uint8Array(await f.arrayBuffer()); const text = new TextDecoder("latin1").decode(b);
  return { size: b.length, head: text.slice(0, 15), has: text.includes(${JSON.stringify(needle)}) }; })`;
await send("Page.enable"); await send("Runtime.enable");
for (const url of [local, LAN && `http://${LAN}:${PORT}`].filter(Boolean)) await send("Network.setCookie", { name: "tt_session", value: session, url });

await go(`${local}/teams`);
await evaluate(load);
const snap = await evaluate(`ttSnapshot.bootstrap({ events: ["2026mslr"] }).then(s => ({ changes: s.changes, upstream: s.upstream, size: s.bytes.byteLength, takenAt: s.takenAt }))`);
const direct = await fetch(`${local}/api/sync/snapshot?event=2026mslr`, { headers: { cookie: `tt_session=${session}` } });
check("downloaded, with the server's cursors", snap && direct.headers.get("x-sync-cursor") === `${snap.changes}-${snap.upstream}`, JSON.stringify(snap));
const copy = await evaluate(opfs("sam@demo.test"));
check("written to OPFS as a SQLite database", copy.size === snap.size && copy.head === "SQLite format 3", JSON.stringify(copy));
check("no account in it", copy.has === false);
const kept = await evaluate(`ttSnapshot.local()`);
check("cursors kept beside it", kept && kept.changes === snap.changes && kept.upstream === snap.upstream && kept.events[0] === "2026mslr", JSON.stringify(kept));
check("an existing copy is not replaced", (await evaluate(reason(`ttSnapshot.bootstrap()`))) === "exists");
check("another schema is refused, and says which way", (await evaluate(reason(`ttSnapshot.bootstrap({ replace: true, schema: 9999 })`))) === "server-behind");
check("and leaves the copy alone", (await evaluate(opfs(""))).size === snap.size);
const again = await evaluate(`ttSnapshot.bootstrap({ replace: true }).then(s => s.bytes.byteLength)`);
check("replace: true downloads again", again > 0 && (await evaluate(opfs(""))).size === again, String(again));

if (LAN) {
  await go(`http://${LAN}:${PORT}/teams`);
  await evaluate(load);
  check("plain http: refused as insecure", (await evaluate(reason(`ttSnapshot.bootstrap()`))) === "insecure");
}

ws.close(); chrome.kill(); server.kill();
