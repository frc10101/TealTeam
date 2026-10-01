// The graph view (U21), in a real Chromium at phone size, over the DevTools
// protocol. Not part of ./check.sh: it needs Chromium, sqlite3, and Node 20+.
//
//   1. cargo build -p tt-web && mkdir -p /tmp/graph && cp target/debug/tt-web /tmp/graph/
//   2. node --experimental-websocket graph.mjs /tmp/graph
//
// It starts and stops the server itself on port 18422, with a fresh database
// in the folder seeded by sqlite3: four teams, twelve matches, every robot
// scouted and approved. It taps chips as a finger would, prints PASS or FAIL
// per check, and leaves graph-360.png in the folder to look at.
import { spawn, execFileSync } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
const D = process.argv[2], PORT = 18422, CDP = 9337;
const local = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rmSync(`${D}/chr`, { recursive: true, force: true });
for (const f of ["t.db", "t.db-wal", "t.db-shm"]) rmSync(`${D}/${f}`, { force: true });
const server = spawn(`${D}/tt-web`, [], { cwd: D, env: { ...process.env, PORT: String(PORT), DATABASE_URL: `sqlite://${D}/t.db?mode=rwc` }, stdio: "ignore" });
for (let i = 0; i < 80; i++) { try { await fetch(`${local}/health`); break; } catch { await sleep(100); } }
const signup = await fetch(`${local}/api/auth/signup`, { method: "POST", redirect: "manual", headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "name=Sam&email=sam%40demo.test&team_number=10101&password=longenough1&confirm_password=longenough1" });
const session = signup.headers.get("set-cookie").match(/tt_session=([^;]+)/)[1];

const teams = [254, 1678, 971, 10101];
const today = new Date().toISOString().slice(0, 10);
let sql = `INSERT INTO events (tba_key, name, start_date, end_date, created_at, updated_at) VALUES ('2026demo', 'Demo Regional', '${today}', '${today}', 'x', 'x');\n`;
for (const t of teams) {
  sql += `INSERT OR IGNORE INTO teams (team_number, name, created_at, updated_at) VALUES (${t}, 'Team ${t}', 'x', 'x');\n`;
  sql += `INSERT INTO event_teams (event_key, team_number, created_at) VALUES ('2026demo', ${t}, 'x');\n`;
  sql += `INSERT INTO team_event_stats (team_number, event_key, opr, synced_at) VALUES (${t}, '2026demo', ${(t % 50) + 10}.5, '${new Date().toISOString()}');\n`;
}
for (let m = 1; m <= 12; m++) {
  sql += `INSERT INTO matches (tba_key, event_key, comp_level, match_number, red1, red2, blue1, blue2, played, created_at, updated_at) VALUES ('2026demo_qm${m}', '2026demo', 'qm', ${m}, 254, 1678, 971, 10101, 1, 'x', 'x');\n`;
  teams.forEach((t, i) => {
    const payload = JSON.stringify({ starting_position: "left", no_show: false, auto_scored: (m + i) % 5, teleop_scored: 5 + ((m * (i + 2)) % 13), endgame: m % 3 ? "full" : "partial", broke_down: m === 7 && i === 1, penalties: (m + i) % 3 });
    sql += `INSERT INTO observations (client_record_id, match_key, team_number, event_key, alliance, payload, schema_version, review_state, observed_at, created_at, updated_at) VALUES ('r${m}-${t}', '2026demo_qm${m}', ${t}, '2026demo', '${i < 2 ? "red" : "blue"}', '${payload}', 1, 'approved', 'x', 'x', 'x');\n`;
  });
}
writeFileSync(`${D}/seed.sql`, sql);
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
const go = async (url) => { await send("Page.navigate", { url }); await sleep(1500); };
const check = (label, ok, detail = "") => console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? "  — " + detail : ""}`);
// A finger on the middle of an element.
const tap = async (selector) => {
  const box = await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); el.closest("label, div").scrollIntoView({ block: "center" }); const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  await send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [box] });
  await send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await sleep(300);
};
const lines = `Number(document.getElementById("graph").dataset.lines)`;
const slotOf = (kind, value) => `document.querySelector('[data-kind="${kind}"] input[value="${value}"]').closest(".chip").dataset.slot`;

await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 360, height: 800, deviceScaleFactor: 2, mobile: true });
await send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
await send("Network.setCookie", { name: "tt_session", value: session, url: local });

await go(`${local}/graph?event=2026demo`);
check("uPlot draws the three best scouted, one metric", (await evaluate(lines)) === 3, String(await evaluate(lines)));
check("the Show button is hidden once the script runs", await evaluate(`document.querySelector("[data-graph-show]").hidden`));
check("no horizontal scroll at 360px", await evaluate(`document.documentElement.scrollWidth <= 360`), String(await evaluate(`document.documentElement.scrollWidth`)));

await tap(`[data-kind="metric"] input[value="f.teleop_scored"]`);
await sleep(200);
check("tapping a metric adds a line per team", (await evaluate(lines)) === 6);
check("and the URL keeps up", (await evaluate(`location.search`)).includes("metric=f.teleop_scored"), await evaluate(`location.search`));
check("it takes the dashed style", (await evaluate(slotOf("metric", "f.teleop_scored"))) === "2");

const first = await evaluate(`document.querySelector('[data-kind="team"] .chip-row input:checked').value`);
const kept = await evaluate(`[...document.querySelectorAll('[data-kind="team"] input:checked')].slice(1).map(i => i.value + ":" + i.closest(".chip").dataset.slot).join(",")`);
await tap(`[data-kind="team"] input[value="${first}"]`);
check("tapping a team off removes its lines", (await evaluate(lines)) === 4);
check("the others keep their colours", (await evaluate(`[...document.querySelectorAll('[data-kind="team"] input:checked')].map(i => i.value + ":" + i.closest(".chip").dataset.slot).join(",")`)) === kept, kept);

await evaluate(`document.querySelector(".more-chips").open = true`);
const other = await evaluate(`document.querySelector('.more-chips input').value`);
await tap(`.more-chips input[value="${other}"]`);
check("a team from More teams takes the freed colour", (await evaluate(slotOf("team", other))) === "1", await evaluate(slotOf("team", other)));

await tap(`[data-kind="metric"] input[value="f.auto_scored"]`);
await tap(`[data-kind="metric"] input[value="f.penalties"]`);
check("a fourth metric is refused, and says why", (await evaluate(`document.getElementById("graph-status").textContent`)).startsWith("Up to 3 at once"));
check("and stays off", (await evaluate(`document.querySelector('[data-kind="metric"] input[value="f.penalties"]').checked`)) === false);

await evaluate(`document.getElementById("graph").scrollIntoView()`);
await sleep(200);
await tap(`#graph .u-over`);
const read = await evaluate(`document.getElementById("graph-readout").hidden ? "" : document.getElementById("graph-readout").textContent`);
check("tapping the chart reads a match", /scouted match/.test(read) && /Q\d+/.test(read), read);
check("the tables follow the chips", (await evaluate(`document.querySelectorAll("#graph-tables table").length`)) === 3);

await go(`${local}${await evaluate(`location.pathname + location.search`)}`);
check("the URL reopens the same chart", (await evaluate(lines)) === 9, String(await evaluate(lines)));

await tap(`#graph .u-over`);
await sleep(200);
const shot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { x: 0, y: 0, width: 360, height: await evaluate(`document.documentElement.scrollHeight`), scale: 1 } });
writeFileSync(`${D}/graph-360.png`, Buffer.from(shot.result.data, "base64"));

ws.close(); chrome.kill(); server.kill();
