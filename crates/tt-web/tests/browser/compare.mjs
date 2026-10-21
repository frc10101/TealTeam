// Compare teams (DR1), in a real Chromium at laptop width and at phone width,
// over the DevTools protocol. Not part of ./check.sh: it needs Chromium,
// sqlite3, and Node 20+.
//
//   1. cargo build -p tt-web && mkdir -p /tmp/compare && cp target/debug/tt-web /tmp/compare/
//   2. node --experimental-websocket compare.mjs /tmp/compare
//
// It starts and stops the server itself on port 18423, with the same seeded
// event as graph.mjs: four teams, twelve matches, every robot scouted and
// approved. It prints PASS or FAIL per check, and leaves compare-1280.png and
// compare-390.png in the folder to look at.
import { spawn, execFileSync } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
const D = process.argv[2], PORT = 18423, CDP = 9338;
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
    const payload = JSON.stringify({ robot_status: m === 7 && i === 1 ? "disabled" : "played", auto_leave: true, auto_launched: (m + i) % 5, launched: 5 + ((m * (i + 2)) % 13), garden_delivered: (m * (i + 1)) % 9, flower_role: m % 3 ? "placed_bottom_nectar" : "attempted", teleop_park: m % 2 === 0, fouls_observed: (m + i) % 3 });
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
const cells = `document.querySelectorAll("#compare-table thead th[data-slot]").length`;
const shoot = async (name, width) => {
  const shot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { x: 0, y: 0, width, height: await evaluate(`document.documentElement.scrollHeight`), scale: 1 } });
  writeFileSync(`${D}/${name}`, Buffer.from(shot.result.data, "base64"));
};

await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
await send("Network.setCookie", { name: "tt_session", value: session, url: local });

await go(`${local}/compare?event=2026demo`);
check("it opens on the best four, a column each", (await evaluate(cells)) === 4, String(await evaluate(cells)));
check("the Show button is hidden once the script runs", await evaluate(`document.querySelector("[data-compare-show]").hidden`));
check("a counter row has box plots", (await evaluate(`document.querySelectorAll("#compare-table svg.cmp-box").length`)) > 8);
check("each row's best is marked", (await evaluate(`document.querySelectorAll("#compare-table td.cmp-best").length`)) > 3);
check("fouls say lower is better", (await evaluate(`[...document.querySelectorAll("#compare-table th[scope=row]")].some(th => th.textContent.includes("Fouls") && th.textContent.includes("lower is better"))`)));
check("robot status is a tally", (await evaluate(`[...document.querySelectorAll("#compare-table td")].some(td => /Played 11 · Disabled 1/.test(td.textContent))`)));
check("OPR is in the official group", (await evaluate(`document.getElementById("compare-table").textContent.includes("Official, from the event")`)));
check("no horizontal scroll at 1280px", await evaluate(`document.documentElement.scrollWidth <= 1280`), String(await evaluate(`document.documentElement.scrollWidth`)));

const first = await evaluate(`document.querySelector('#compare-table thead th[data-slot="1"] a:not(.cmp-remove)').textContent`);
await evaluate(`document.querySelector('#compare-table thead th[data-slot="1"] .cmp-remove').click()`);
await sleep(1500);
check("Remove takes a team out", (await evaluate(cells)) === 3, String(await evaluate(cells)));
check("and the URL keeps the choice", !(await evaluate(`location.search`)).includes(`team=${first}`), await evaluate(`location.search`));

const ref = await evaluate(`document.querySelector("#compare-ref option:nth-child(2)").value`);
await evaluate(`(() => { const s = document.getElementById("compare-ref"); s.value = "${ref}"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
await sleep(1500);
check("a reference team is marked", (await evaluate(`document.querySelector("#compare-table thead th.reference a:not(.cmp-remove)").textContent`)) === ref);
check("the others say how far ahead or behind", (await evaluate(`document.querySelectorAll("#compare-table .cmp-delta.ahead, #compare-table .cmp-delta.behind").length`)) > 3);

await evaluate(`(() => { const s = document.getElementById("compare-last"); s.value = "3"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
await sleep(1500);
check("the last three matches only", (await evaluate(`[...document.querySelectorAll(".cmp-matches")].every(s => s.textContent.trim().startsWith("3 matches"))`)), await evaluate(`[...document.querySelectorAll(".cmp-matches")].map(s => s.textContent.trim()).join("|")`));
await shoot("compare-1280.png", 1280);

// Moving teams in and out: Remove, and + Add team's search.
await go(`${local}/compare?event=2026demo&chosen=1&team=254&team=1678&team=971`);
await evaluate(`document.querySelector('#compare-table thead th[data-slot="2"] .cmp-remove').click()`);
await sleep(1500);
check("Remove on the middle column takes that team out", (await evaluate(cells)) === 2 && !(await evaluate(`location.search`)).includes("team=1678"), await evaluate(`location.search`));
await evaluate(`document.querySelector("#compare-add summary").click()`);
await sleep(200);
check("+ Add team opens a search, with the cursor in it", await evaluate(`document.activeElement.id === "compare-search"`));
const type = async (text) => evaluate(`(() => { const i = document.getElementById("compare-search"); i.value = ${JSON.stringify(text)}; i.dispatchEvent(new Event("input")); return [...document.querySelectorAll("#compare-addable li:not([hidden])")].map(li => li.querySelector("strong").textContent).join(","); })()`);
check("the search lists the teams not compared", (await type("")) === "1678,10101" || (await type("")) === "10101,1678", await type(""));
check("it finds a team by its name", (await type("team 1678")) === "1678");
check("it finds a team by part of its number", (await type("67")) === "1678");
check("and says when nothing matches", (await type("99999")) === "" && !(await evaluate(`document.getElementById("compare-none").hidden`)));
await type("1678");
await evaluate(`document.getElementById("compare-search").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }))`);
await sleep(1500);
check("Enter adds the match, as the last column", (await evaluate(`[...document.querySelectorAll("#compare-table thead th[data-slot] a:not(.cmp-remove)")].map(a => a.textContent).join(",")`)) === "254,971,1678");


await go(`${local}/lead-scout/rankings?event=2026demo`);
await evaluate(`[...document.querySelectorAll('input[form="rank-compare"]')].slice(0, 2).forEach(i => i.click())`);
await evaluate(`document.querySelector("#rank-compare button").click()`);
await sleep(1500);
check("ticking two on Rankings compares those two", (await evaluate(cells)) === 2 && (await evaluate(`location.pathname`)) === "/compare", String(await evaluate(cells)));

await go(`${local}/teams?event=2026demo&team=10101`);
const link = await evaluate(`[...document.querySelectorAll("a")].find(a => a.textContent.includes("Compare with the best"))?.getAttribute("href") || ""`);
await go(`${local}${link}`);
check("the team page's link puts that team first, as the reference", (await evaluate(`document.querySelector("#compare-table thead th.reference a:not(.cmp-remove)")?.textContent`)) === "10101", link);

await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
await go(`${local}/compare?event=2026demo`);
check("no page-wide horizontal scroll at 390px; the table scrolls in its card", await evaluate(`document.documentElement.scrollWidth <= 390`), String(await evaluate(`document.documentElement.scrollWidth`)));
await shoot("compare-390.png", 390);

ws.close(); chrome.kill(); server.kill();
