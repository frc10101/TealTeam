// QR handoff (S13), end to end in a real Chromium over the DevTools
// protocol, with the codes read by the real readers. Not part of ./check.sh:
// it needs Chromium, Node 20+, and the sqlite3 shell.
//
//   cargo build -p tt-web
//   node --experimental-websocket handoff.mjs <empty folder> target/debug/tt-web
//   (TT_PORT and TT_CDP move it off 18423 and 9338.)
//
// Kim's tablet saves a form the server never answers, and shows it as QR
// codes. Each code is screenshotted to a PNG, as a lead's camera would see
// it. Sam, a lead, gives those photos to the scanner page, which reads them
// (zxing-wasm: headless Chromium on Linux has no BarcodeDetector) and posts
// them; the server records the form as Kim's. The receipt Sam's screen shows
// is screenshotted too, and Kim's tablet, now with no server at all, reads
// it and lets the form go. Prints PASS or FAIL per check; leaves the PNGs in
// the folder, with tablet.png and result.png of the two pages.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
const [D, BIN] = process.argv.slice(2).map((a) => resolve(a));
const PORT = Number(process.env.TT_PORT || 18423), CDP = Number(process.env.TT_CDP || 9338), local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(D, { recursive: true });
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });

const server = spawn(BIN, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
let chrome = null;
process.on("exit", () => { server.kill(); chrome?.kill(); });
for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); break; } catch { await sleep(100); } }
async function signUp(name) {
  const r = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `name=${name}&email=${name}%40handoff.test&team_number=10101&password=longenough1&confirm_password=longenough1` });
  return r.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];
}
const sam = await signUp("Sam"); // the first account: an admin, so a lead
const kim = await signUp("Kim");
const today = new Date().toISOString().slice(0, 10);
execFileSync("sqlite3", [`${D}/t.db`, `
INSERT INTO events (tba_key, name, start_date, end_date, created_at, updated_at) VALUES ('2026demo', 'Demo Regional', '${today}', '${today}', 'x', 'x');
INSERT INTO teams (team_number, name, created_at, updated_at) VALUES (254, 'The Cheesy Poofs', 'x', 'x');
INSERT INTO event_teams (event_key, team_number, created_at) VALUES ('2026demo', 254, 'x');
INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, red3, blue1, blue2, blue3, played, created_at, updated_at)
  VALUES ('2026demo_qm2', '2026demo', 'qm', 2, 254, 1, 2, 3, 4, 5, 0, 'x', 'x');`]);

chrome = spawn("chromium", ["--headless=new", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${D}/chr`, "about:blank"], { stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { await sleep(100); } }
const target = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0; const pending = new Map();
ws.addEventListener("message", (m) => { const msg = JSON.parse(m.data); if (msg.id) { pending.get(msg.id)?.(msg); pending.delete(msg.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.result?.value;
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
const until = async (expr, ms = 8000) => { for (let t = 0; t < ms; t += 100) { if (await evaluate(expr)) return true; await sleep(100); } return false; };
let failed = 0;
const check = (label, ok, detail = "") => { if (!ok) failed++; console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`); };
const as = (session) => send("Network.setCookie", { name: "tt_session", value: session, url: local });
// A screenshot of one element, as a camera would see the screen.
async function shoot(selector, file) {
  const box = await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); el.scrollIntoView({ block: "center" }); const r = el.getBoundingClientRect(); return { x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height }; })()`);
  const shot = await send("Page.captureScreenshot", { format: "png", clip: { ...box, scale: 1 } });
  writeFileSync(file, Buffer.from(shot.result.data, "base64"));
  return file;
}
// The whole page, for a person to look at.
async function page(file) {
  const shot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
  writeFileSync(file, Buffer.from(shot.result.data, "base64"));
}
async function setFiles(selector, files) {
  const doc = await send("DOM.getDocument", {});
  const node = await send("DOM.querySelector", { nodeId: doc.result.root.nodeId, selector });
  await send("DOM.setFileInputFiles", { nodeId: node.result.nodeId, files });
}

await send("Page.enable"); await send("Runtime.enable"); await send("Network.enable"); await send("DOM.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 360, height: 800, deviceScaleFactor: 2, mobile: true });

// ── Kim's tablet ──────────────────────────────────────────────────────────
await as(kim);
await go(`${local}/submission?event=2026demo&match=2026demo_qm2&team=254`);
check("Kim's tablet holds a token", await until(`!!(window.ttToken && ttToken.token())`));
// Kim fills the form and presses Save; no answer ever comes back. Since
// C13 the form saves by fetch, and holds what does not land, so the save
// and the outbox's push are both cut off, as when the Pi is out of reach.
await send("Network.setBlockedURLs", { urls: ["*/api/submission*", "*/api/sync/push*"] });
await evaluate(`(() => {
  const form = document.getElementById("scout-form");
  form.elements["f.starting_position"].value = "center";
  form.elements["f.teleop_scored"].value = "9";
  form.elements["f.broke_down"].checked = true;
  form.elements["f.notes"].value = "tippy on the ramp";
  form.querySelector("button[type=submit]").click();
})()`);
await until(`document.body.textContent.includes("Held on this tablet")`, 15000);
const recordId = await evaluate(`document.getElementById("scout-form").elements.record_id.value`);
await evaluate(`document.querySelector("#handoff summary").click()`);
await sleep(300);
check("the panel lists the form, ticked, since Save was pressed",
  await evaluate(`(() => { const boxes = document.querySelectorAll("[data-handoff-forms] input"); return boxes.length === 1 && boxes[0].checked; })()`),
  await evaluate(`document.querySelector("[data-handoff-forms]").textContent.trim()`));
check("it reads Q2 · Team 254", (await evaluate(`document.querySelector("[data-handoff-forms]").textContent`)).includes("Q2 · Team 254"));
await evaluate(`document.querySelector("[data-handoff-show]").click()`);
check("the code shows", await until(`document.querySelectorAll("[data-handoff-frames] svg.qr").length > 0`));
const texts = await evaluate(`[...document.querySelectorAll("[data-handoff-frames] svg.qr")].map((s) => s.dataset.text)`);
check("in our format", texts.every((t) => /^TT1:H:[0-9A-F]{8}:\d+\/\d+:/.test(t)), `${texts.length} part(s)`);
// T1: what the codes carry names Kim and holds a MAC, and no token.
const carried = await evaluate(`(async () => {
  const done = await (() => { const a = new ttQr.Assembler(); ${JSON.stringify(texts)}.forEach((t) => a.add(t)); return a.finish(); })();
  return new TextDecoder().decode(done.bytes);
})()`);
check("the codes carry no token, only who and a MAC", !!carried && !carried.includes("v4.public.") && /"mac":"[A-Za-z0-9_-]{43}"/.test(carried) && carried.includes('\\"scout\\":{'), (carried || "").slice(0, 120));
check("no horizontal scroll at 360px", await evaluate(`document.documentElement.scrollWidth <= 360`));
await page(`${D}/tablet.png`);
// Each part, photographed.
await evaluate(`document.querySelector("[data-handoff-pause]").click()`);
const photos = [];
for (let i = 0; i < texts.length; i++) {
  await evaluate(`(() => { const figs = [...document.querySelectorAll("[data-handoff-frames] figure")]; figs.forEach((f, j) => f.hidden = j !== ${i}); })()`);
  photos.push(await shoot(`[data-handoff-frames] figure:not([hidden]) svg`, `${D}/part-${i + 1}.png`));
}

// ── Sam, the lead, at the server ──────────────────────────────────────────
await as(sam);
await go(`${local}/lead-scout/scan?event=2026demo`);
check("the scanner shows", await evaluate(`!document.querySelector("[data-scanner]").hidden`));
await setFiles("[data-photo]", photos);
const recorded = await until(`document.body.textContent.includes("Recorded 1 of 1 form from Kim's tablet")`, 20000);
check("the photos are read, and the server records the form as Kim's", recorded,
  recorded ? "" : await evaluate(`(document.querySelector("[data-status]") || document.body).textContent.trim().slice(0, 300)`));
const db = execFileSync("sqlite3", [`${D}/t.db`, `SELECT u.name, o.team_number, json_extract(o.payload, '$.teleop_scored'), json_extract(o.payload, '$.notes') FROM observations o JOIN users u ON u.id = o.scouter_id WHERE o.client_record_id = '${recordId}'`]).toString().trim();
check("as Kim, with the answers typed", db === "Kim|254|9|tippy on the ramp", db);
await page(`${D}/result.png`);
const receipt = await shoot("#receipt svg.qr", `${D}/receipt.png`);

// ── Kim's tablet again, with no server at all ─────────────────────────────
await as(kim);
await go(`${local}/submission?event=2026demo&match=2026demo_qm2&team=254`);
await sleep(2500); // the reader is fetched two seconds after load
await send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
server.kill();
check("the form still waits on the tablet", await evaluate(`Object.keys(localStorage).some((k) => k.startsWith("tt-draft:v1:2:") && JSON.parse(localStorage[k]).recordId === ${JSON.stringify(recordId)})`));
await evaluate(`document.querySelector("#handoff summary").click()`);
// A receipt the server did not make (R1): it names Kim's form, and this
// tablet's nonce, but its MAC is not the server's. Nothing is deleted.
const nonce = await evaluate(`JSON.parse(localStorage.getItem("tt-handoff:v1:2")).nonce`);
const forgedTexts = await evaluate(`ttQr.frames("R", new TextEncoder().encode(JSON.stringify({
  body: JSON.stringify({ nonce: ${JSON.stringify(nonce)}, receipts: [{ client_record_id: ${JSON.stringify(recordId)}, outcome: "recorded" }] }),
  mac: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" })))`);
await evaluate(`(() => { const box = document.createElement("div"); box.id = "forged"; box.style.cssText = "background:#fff;padding:16px;width:320px";
  box.appendChild(ttQr.draw(${JSON.stringify(forgedTexts[0])})); document.body.prepend(box); })()`);
const forged = await shoot("#forged svg", `${D}/forged.png`);
await evaluate(`document.getElementById("forged").remove()`);
await setFiles("[data-handoff-photo]", [forged]);
const turnedAway = await until(`document.querySelector("[data-handoff-status]").textContent.includes("not made by the server")`, 20000);
check("a forged receipt is turned away", turnedAway, await evaluate(`document.querySelector("[data-handoff-status]").textContent`));
check("  and the form still waits", await evaluate(`Object.keys(localStorage).some((k) => k.startsWith("tt-draft:v1:2:") && JSON.parse(localStorage[k]).recordId === ${JSON.stringify(recordId)})`));
await setFiles("[data-handoff-photo]", [receipt]);
const settled = await until(`document.querySelector("[data-handoff-status]").textContent.includes("The server has 1 of your forms.")`, 20000);
check("the receipt is read with no server, and says so", settled, await evaluate(`document.querySelector("[data-handoff-status]").textContent`));
check("and the tablet let the form go", await evaluate(`!Object.keys(localStorage).some((k) => k.startsWith("tt-draft:v1:2:") && JSON.parse(localStorage[k]).recordId === ${JSON.stringify(recordId)})`));

ws.close(); chrome.kill();
console.log(failed ? `${failed} FAILED` : "all passed");
process.exit(failed ? 1 : 0);
