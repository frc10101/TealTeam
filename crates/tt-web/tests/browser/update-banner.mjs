// The update banner (S11), end to end in a real Chromium over the DevTools
// protocol. Not part of ./check.sh: it needs Chromium and Node 20+.
//
//   Three builds in one folder, plus a database with an event and matches:
//     tt-web-a  as is
//     tt-web-b  one static file changed (a new build, the same form)
//     tt-web-c  seasons/2026.json "version" bumped (the form changed)
//     seed.db   copied with: sqlite3 <db> ".backup <folder>/seed.db"
//   node --experimental-websocket update-banner.mjs <folder> <form path>
//     e.g. /submission?event=2026demo&match=2026demo_qm5&team=10101
//
// A deploy happens under an open scouting form, twice: the page must block,
// keep what was typed, and reload into the new version.
import { spawn } from "node:child_process";
import { copyFileSync, rmSync, writeFileSync } from "node:fs";
const [D, FORM] = process.argv.slice(2);
const PORT = 18420, CDP = 9335, origin = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
copyFileSync(`${D}/seed.db`, `${D}/t.db`);

let server = null;
async function start(bin) {
  server = spawn(`${D}/${bin}`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db` }, stdio: "ignore" });
  for (let i = 0; i < 80; i++) { try { await fetch(`${origin}/health`); return; } catch { await sleep(100); } }
  throw new Error(`${bin} did not start`);
}
async function stop() { server.kill(); await new Promise((r) => server.once("exit", r)); }
const health = async () => (await fetch(`${origin}/health`)).json();

await start("tt-web-a");
const signup = await fetch(`${origin}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Kim&email=kim%40update.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map();
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => {
  const r = (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result;
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 760, deviceScaleFactor: 1, mobile: true });
await send("Network.setCookie", { name: "tt_session", value: session, url: origin });

// The notes box: free text, so what is typed is easy to find again.
const FIELD = `document.querySelector('form[data-draft] textarea')`;
const type = (text, announce) => evaluate(`(() => { const f = ${FIELD}; f.value = ${JSON.stringify(text)};
  ${announce ? "f.dispatchEvent(new Event('input', { bubbles: true }));" : ""} return !!f; })()`);
const banner = () => evaluate(`(() => { const b = document.getElementById('update-banner');
  return { shown: !b.hidden, formNote: !b.querySelector('[data-update-form]').hidden,
    mainInert: document.querySelector('main').inert, focused: document.activeElement?.dataset?.updateReload !== undefined }; })()`);
const meta = (n) => evaluate(`document.querySelector('meta[name="${n}"]').content`);
const drafts = () => evaluate(`Object.keys(localStorage).filter(k => k.startsWith('tt-draft:')).map(k => JSON.parse(localStorage.getItem(k)))`);
async function deploy(bin) {
  await stop(); await start(bin);
  // As the browser does when it gets its network back: check at once.
  await evaluate(`window.dispatchEvent(new Event('online'))`);
  await sleep(1500);
}
async function reloadViaBanner() {
  await evaluate(`document.querySelector('[data-update-reload]').click()`);
  await sleep(2500);
}

await send("Page.navigate", { url: origin + FORM }); await sleep(2000);
check("the scouting form is open", await evaluate(`!!(${FIELD})`), await evaluate(`document.querySelector('h1')?.textContent`));
const a = await health();
check("the page is stamped with the running build", (await meta("tt-build")) === a.build, a.build);

// 1. A new build, the same form. The newest words are typed but not yet
// saved by the debounce when the deploy lands.
await type("seen at A", true); await sleep(700);
await type("typed just before the deploy", false);
await deploy("tt-web-b");
let b1 = await banner();
check("a new build blocks the page", b1.shown && b1.mainInert && b1.focused, JSON.stringify(b1));
check("  and says nothing about the form", !b1.formNote);
await reloadViaBanner();
const b = await health();
check("Reload now lands on the new build", (await meta("tt-build")) === b.build && b.build !== a.build, `${a.build} -> ${b.build}`);
check("  without the banner", !(await banner()).shown);
check("  with the words typed just before the deploy put back", (await evaluate(`${FIELD}.value`)) === "typed just before the deploy",
  await evaluate(`${FIELD}.value`));

// 2. The form itself changed.
await type("typed into form 1", true); await sleep(700);
await deploy("tt-web-c");
const b2 = await banner();
check("a changed form blocks the page", b2.shown && b2.mainInert, JSON.stringify(b2));
check("  and says the form changed", b2.formNote);
const shot = await send("Page.captureScreenshot", { format: "png" });
writeFileSync(`${D}/banner.png`, Buffer.from(shot.result.data, "base64"));
await reloadViaBanner();
const c = await health();
check("Reload now lands on the new form", (await meta("tt-form")) === String(c.form) && c.form !== b.form, `form ${b.form} -> ${c.form}`);
const kept = await drafts();
check("  and the old form's answers are still kept on the device", kept.some((d) => JSON.stringify(d).includes("typed into form 1")),
  `${kept.length} draft(s)`);

// 3. Nothing changed: no banner.
await evaluate(`window.dispatchEvent(new Event('online'))`); await sleep(1500);
check("the same build again: no banner", !(await banner()).shown);

ws.close(); chrome.kill(); await stop(); process.exit(0);
