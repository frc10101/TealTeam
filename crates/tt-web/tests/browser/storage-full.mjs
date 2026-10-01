// A phone whose storage is full (Q3), in a real Chromium, over the DevTools
// protocol. Not part of ./check.sh: it needs Chromium and Node 20+.
//
//   1. cargo build -p tt-web -p tt-load && mkdir -p /tmp/full
//      cp target/debug/tt-web target/debug/tt-load /tmp/full/
//   2. node --experimental-websocket storage-full.mjs /tmp/full
//
// It starts and stops the server itself on port 18423, with a fresh database
// in the folder seeded by `tt-load seed`, and prints PASS or FAIL per check.
//
// Two kinds of full. localStorage, where drafts live (C3), has its own limit
// and is filled until it refuses. Everything else -- OPFS, where the snapshot
// goes (S10), and the service worker's cache (C1) -- shares the origin's
// quota, which is turned down to 4 MB and filled with one file.
import { spawn, execFileSync } from "node:child_process";
import { rmSync } from "node:fs";
const D = process.argv[2], PORT = 18423, CDP = 9338, QUOTA = 4 * 1024 * 1024;
const local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
const db = `sqlite://${D}/t.db?mode=rwc`;
execFileSync(`${D}/tt-load`, ["seed", db, "20"]);
const server = spawn(`${D}/tt-web`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: db }, stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); break; } catch { await sleep(100); } }
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); const thrown = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); }
  else if (msg.method === "Runtime.exceptionThrown") thrown.push(msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text);
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
const load = `new Promise(r => { const s = document.createElement("script"); s.src = "/static/js/snapshot.js"; s.onload = r; document.head.append(s); })`;
const reason = (call) => `${call}.then(() => "resolved", e => e.reason || e.name)`;
await send("Page.enable"); await send("Runtime.enable");
await send("Network.setCookie", { name: "tt_session", value: session, url: local });
await send("Storage.overrideQuotaForOrigin", { origin: local, quotaSize: QUOTA });

// ── Fill it ──
// From /health: a page on the origin that does not register the service
// worker, so the shell has no chance to install before the storage is full.
await go(`${local}/health`);
const filled = await evaluate(`(() => {
  let n = 0;
  for (let size = 1 << 20; size >= 16; size >>= 1) {
    for (;;) { try { localStorage.setItem("filler-" + n, "x".repeat(size)); n++; } catch (e) { break; } }
  }
  try { localStorage.setItem("one-more", "x"); return "not full"; } catch (e) { return e.name + " after " + n + " items"; }
})()`);
check("localStorage filled until it refused", /QuotaExceeded/.test(filled), filled);
// One file per chunk, smaller and smaller: a write that fails is thrown away
// whole, so only completed files take up the quota. (Chromium's estimate()
// reports a padded quota whatever the override; the writes are what tell.)
const opfs = await evaluate(`navigator.storage.getDirectory().then(async d => {
  let n = 0, bytes = 0;
  for (let size = 256 * 1024; size >= 1024; size >>= 1) {
    for (;;) {
      try {
        const out = await (await d.getFileHandle("filler-" + n, { create: true })).createWritable();
        await out.write(new Uint8Array(size)); await out.close(); n++; bytes += size;
      } catch (e) { await d.removeEntry("filler-" + n).catch(() => {}); break; }
    }
  }
  return { files: n, bytes };
})`);
const full = await evaluate(`navigator.storage.getDirectory().then(d => d.getFileHandle("probe", { create: true })).then(h => h.createWritable())
  .then(async out => { await out.write(new Uint8Array(64 * 1024)); await out.close(); return "fits"; }).catch(e => e.name)`);
check("the origin's quota is used up", full === "QuotaExceededError", `${JSON.stringify(opfs)}, a 64 KB file: ${full}`);
await evaluate(`navigator.storage.getDirectory().then(d => d.removeEntry("probe")).then(() => true, () => true)`);

// ── A scout can still scout ──
thrown.length = 0;
const team = 9007; // robots(1)[0]: see crates/tt-load/src/seed.rs
await go(`${local}/submission?event=2026load&match=2026load_qm1&team=${team}`);
const form = await evaluate(`!!document.getElementById("scout-form")`);
check("the scouting page opens", form);
await evaluate(`(() => {
  const f = document.getElementById("scout-form");
  f.querySelector('[name="f.starting_position"][value="left"]').checked = true;
  f.querySelector('[name="f.notes"]').value = "storage full";
  f.querySelectorAll("input, textarea").forEach(el => el.dispatchEvent(new Event("input", { bubbles: true })));
  return true;
})()`);
await sleep(800); // past draft.js's debounce, so it tries to write
const draftKept = await evaluate(`Object.keys(localStorage).some(k => k.startsWith("tt-draft:"))`);
check("no draft could be kept, and nothing threw", !draftKept && thrown.length === 0, thrown.join(" | "));
const recordId = await evaluate(`document.querySelector('#scout-form [name="record_id"]').value`);
await evaluate(`document.getElementById("scout-form").requestSubmit() || true`);
await sleep(1500);
const after = await evaluate(`({ url: location.search, saved: !!document.querySelector(".alert-success") })`);
check("the save goes through", after.saved && after.url.includes(`saved=${team}`), JSON.stringify(after));
await sleep(2500); // past the change log's two-second lag
const pulled = await (await fetch(`${local}/api/sync/pull?event=2026load`, { headers: { cookie: `tt_session=${session}` } })).json();
check("and is on the server", pulled.changes.some((c) => c.entity_pk === recordId), recordId);

// ── The offline shell ──
const worker = await evaluate(`navigator.serviceWorker.getRegistration().then(r => r ? (r.active ? "active" : r.installing ? "installing" : "waiting") : "none")`);
const shell = await evaluate(`caches.keys().then(ks => ks.length)`);
check("the offline shell does not install while full, and the site works without it", worker === "none", `worker ${worker}, caches ${shell}`);

// ── The snapshot ──
await go(`${local}/teams`);
await evaluate(load);
const refused = await evaluate(reason(`ttSnapshot.bootstrap({ events: ["2026load"] })`));
check("a snapshot that does not fit is refused, as full", refused === "full", refused);
check("and leaves no cursors behind", (await evaluate(`ttSnapshot.local()`)) === null);
// A first write that fails leaves an empty file; it must not pass for a copy.
const again = await evaluate(reason(`ttSnapshot.bootstrap({ events: ["2026load"] })`));
check("a second try is not told the device already has a copy", again === "full", again);

// ── Room again ──
await evaluate(`(async () => { for (const k of Object.keys(localStorage)) if (k.startsWith("filler") || k === "one-more") localStorage.removeItem(k);
  const d = await navigator.storage.getDirectory();
  for await (const name of d.keys()) if (name.startsWith("filler")) await d.removeEntry(name);
  return true; })()`);
const retry = await evaluate(reason(`ttSnapshot.bootstrap({ events: ["2026load"] })`));
check("with room again, the snapshot downloads", retry === "resolved", retry);
await go(`${local}/teams`);
await sleep(1500);
const shellBack = await evaluate(`navigator.serviceWorker.ready.then(() => caches.keys()).then(ks => ks.length)`);
check("and the offline shell installs", shellBack === 1, String(shellBack));

ws.close(); chrome.kill(); server.kill();
