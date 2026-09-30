// The service worker (C1), end to end in a real Chromium, over the DevTools
// protocol. Not part of ./check.sh: it needs Chromium and Node 20+.
//
//   1. Build twice, differing in one static file, into one folder:
//        cargo build -p tt-web && cp target/debug/tt-web /tmp/sw/tt-web-a
//        echo '/* b */' >> crates/tt-web/static/css/site.css
//        cargo build -p tt-web && cp target/debug/tt-web /tmp/sw/tt-web-b
//        git checkout crates/tt-web/static/css/site.css
//      (the last check looks for "build B" in the new stylesheet: append
//       '/* build B */' exactly, or change that check)
//   2. node --experimental-websocket service-worker.mjs /tmp/sw <this machine's LAN IP>
//
// It starts and stops the server itself on port 18419, with a fresh database
// in the folder, and prints PASS or FAIL per check. The LAN address stands in
// for a tablet at the event: plain http, not a secure context.
import { spawn } from "node:child_process";
import { rmSync } from "node:fs";
const D = process.argv[2], LAN = process.argv[3], PORT = 18419, CDP = 9334;
const local = `http://127.0.0.1:${PORT}`, lan = `http://${LAN}:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let server = null;
// A browser with no worker yet, and an empty database.
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
async function start(bin) {
  server = spawn(`${D}/${bin}`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); return; } catch { await sleep(100); } }
  throw new Error("server did not start");
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }

await start("tt-web-a");
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map(); const events = [];
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } else events.push(msg); });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
await send("Page.enable"); await send("Runtime.enable"); await send("Log.enable");
for (const url of [local, lan]) await send("Network.setCookie", { name: "tt_session", value: session, url });

// 1. Registration and precache on a secure origin.
await go(`${local}/teams`);
const reg = await evaluate(`navigator.serviceWorker.ready.then(r => r.active.scriptURL)`);
await sleep(1000);
const cacheA = await evaluate(`caches.keys().then(async ks => ({ keys: ks, n: (await (await caches.open(ks[0])).keys()).length }))`);
check("worker registered at /sw.js", reg === `${local}/sw.js`, reg);
check("shell precached", cacheA.keys.length === 1 && cacheA.n > 10, JSON.stringify(cacheA));
const cachedPages = await evaluate(`caches.open(${JSON.stringify(cacheA.keys[0])}).then(c => c.keys()).then(ks => ks.map(k => new URL(k.url).pathname).filter(p => !p.startsWith('/static/')))`);
check("the only page cached is /offline", JSON.stringify(cachedPages) === '["/offline"]', JSON.stringify(cachedPages));

// 2. Server gone: a reload shows the shell, styled, at the same address.
await stop();
await go(`${local}/teams`);
const offline = await evaluate(`({ url: location.pathname, shell: !!document.getElementById('offline-shell'),
  heading: document.querySelector('h1').textContent, bg: getComputedStyle(document.body).backgroundColor })`);
check("reload with no server shows the offline page", offline.shell && offline.url === "/teams", JSON.stringify(offline));
check("its stylesheet came from the cache", offline.bg === "rgb(3, 7, 18)", offline.bg);
await sleep(1500);
const chip = await evaluate(`[...document.querySelectorAll('[data-link]')].filter(e => !e.hidden).map(e => e.textContent.trim())[0]`);
check("the chip says offline", /^Offline/.test(chip), chip);

// 3. A POST is never answered from the cache.
const post = await evaluate(`fetch('/api/pick-list', { method: 'POST' }).then(r => 'answered ' + r.status, e => 'refused: ' + e.name)`);
check("a POST while offline fails, not cached", post.startsWith("refused"), post);

// 4. Server back: the shell reloads itself into the real page.
await start("tt-web-a");
await sleep(7000);
const back = await evaluate(`({ url: location.pathname, shell: !!document.getElementById('offline-shell'), h1: document.querySelector('h1')?.textContent })`);
check("server back: the page reloaded itself", !back.shell && back.url === "/teams", JSON.stringify(back));

// 4b. /offline opened directly, with the server up, stays put (no reload loop).
await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 640, deviceScaleFactor: 1, mobile: true });
events.length = 0;
await go(`${local}/offline`);
await sleep(7000);
const navs = events.filter((e) => e.method === "Page.frameNavigated").length;
check("/offline opened directly does not reload itself", navs === 1, `${navs} navigation(s)`);
const shot = await send("Page.captureScreenshot", { format: "png" });
(await import("node:fs")).writeFileSync(`${D}/offline.png`, Buffer.from(shot.result.data, "base64"));
await send("Emulation.clearDeviceMetricsOverride");

// 5. A new binary with one changed file replaces the shell's cache.
await stop(); await start("tt-web-b");
await go(`${local}/teams`);
await sleep(3000);
const cacheB = await evaluate(`caches.keys()`);
check("a new build replaced the cache", cacheB.length === 1 && cacheB[0] !== cacheA.keys[0], `${cacheA.keys[0]} -> ${JSON.stringify(cacheB)}`);
const cssB = await evaluate(`caches.open(${JSON.stringify(cacheB[0])}).then(c => c.match('/static/css/site.css')).then(r => r.text()).then(t => t.includes('build B'))`);
check("and holds the new build's stylesheet", cssB === true);

// 6. Plain http from the LAN address: no worker, nothing in the console.
events.length = 0;
await go(`${lan}/teams`);
await sleep(2000);
const insecure = await evaluate(`({ secure: isSecureContext, sw: 'serviceWorker' in navigator, h1: document.querySelector('h1')?.textContent })`);
const noise = events.filter((e) => e.method === "Runtime.exceptionThrown" || e.method === "Log.entryAdded" || (e.method === "Runtime.consoleAPICalled"));
check("LAN http: not a secure context, no service worker API", !insecure.secure && !insecure.sw, JSON.stringify(insecure));
check("LAN http: the page works", insecure.h1 && !insecure.h1.includes("reach"), insecure.h1);
check("LAN http: no console errors or warnings", noise.length === 0, JSON.stringify(noise.map((e) => e.params?.entry?.text || e.params?.exceptionDetails?.text || e.params?.type)));

ws.close(); chrome.kill(); await stop(); process.exit(0);
