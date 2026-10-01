# TealTeam — Action Items

**One list.** Merges the work items from [RefurbishInstructions.md](../RefurbishInstructions.md) (what should change) with the rebuild work from [REBUILD_SPEC.md](REBUILD_SPEC.md) (what existed and how to recreate it). Those two documents remain the reference material; **this is the list you work from.**

Ordered by dependency and by value delivered per unit of work — not by source-document order.

## How to read this

| Column | Meaning |
| --- | --- |
| **#** | Stable ID. Area prefix + number. Use these in commits and issues. |
| **Source** | Provenance. `RI` = RefurbishInstructions work-item ID. `RS §n` = REBUILD_SPEC section. Items with both are merges. |
| **Effort** | S ≈ hours · M ≈ a day or two · L ≈ a week · XL ≈ multiple weeks |

Area prefixes: **F** foundation · **D** data · **A** auth/identity · **U** interface · **I** integrations · **L** lead scout · **C** client/offline · **P** platform/Pi · **X** communication · **Q** quality

**The merge changed some estimates.** Several refurbish items were sized against a codebase that no longer exists. `RI-O6` ("extract SQL out of handlers") was XL — the largest single item in the plan — and is now `F3` at M, because there are no 187 inline queries to extract. `RI-O19` (migrate Pi to SQLite) was L and is now free: the schema is SQLite from its first line. Conversely, everything the old code gave you for nothing now has to be built.

---

## Already done

| Action | Result |
| --- | --- |
| Retire the Go and .NET ports, and Render (`RI` Phase 0) | Done 2026-08-26. All three implementations, `render.yaml`, the shared `migrations/`, the Docker stack, the Pi scripts, and `web/` deleted. Knowledge captured in `REBUILD_SPEC.md`. |
| Resolve the port-retirement timeline (`RI` Open Question 3) | Resolved by the above. There is one implementation now — never mirror a change across ports again. |

---

## Phase 0 — Decisions and foundations

Nothing downstream is safe until these land.

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| F1 | Cargo workspace: `tt-core`, `tt-templates`, `tt-repo`, `tt-repo-sqlite`, `tt-web` (`tt-client` arrives in Phase 3) | RI-O4 · RS §9 | M | **Done** |
| F2 | **CI job building `tt-core` + `tt-templates` for `wasm32-unknown-unknown`, from the first commit.** The only thing that keeps the crate boundary honest | RI-O5 · RS §9 | S | **Done** |
| F3 | `Repo` trait via `trait-variant` — Send and non-Send variants, because wasm futures are not `Send` | RI-O6 · RS §9 | M | **Done** |
| F4 | Config loading (`.env` app-dir then repo root, existing env always wins) and tracing at `info,sqlx=warn` | RS §8 | S | **Done** |
| F5 | Startup sequence: validate env → lazy pool → `SELECT 1` probe → **boot anyway if the DB is down** and degrade DB-backed pages. At an event, half-working beats not booting | RS §8 | S | **Done** |
| P1 | **DS3231 RTC on the Pi** before anything else (~$5, two hours). Without it a power cycle gives a server whose clock is wrong by hours, silently corrupting every timestamp in the design | RI-N1 · RS §10 | S | Hardware — not started |
| P2 | Confirm the exact 2026 E143 wording; record the FTA conversation at the first event | RI-N8 | S | Blocked, see below |

### Phase 0 notes

**What F1–F5 produced.** A five-crate workspace that builds, tests, lints clean, and runs. `cargo run -p tt-web` serves `/` and `/health` against a WAL-mode SQLite file. `./check.sh` runs everything CI runs.

The wasm gate was verified to *fail* as well as pass: temporarily adding `sqlx` and `tokio` to `tt-core` breaks the wasm32 build with `This wasm target is unsupported by mio`. A gate that has only ever been seen passing proves nothing.

**`TEALTEAM_ENV=test` is now a fatal startup error**, not a silent dangerous default (RS §12.10). `dev` is the only value that permits schema resets, and it logs a warning when set.

**P2 is blocked on sources, not effort.** The 2026 Game Manual HTML at `firstfrc.blob.core.windows.net` truncates before section 14.3, where the wireless rules live. Get the full PDF, or read E143 off a printed manual. The second half of this item — what your FTA will actually tolerate — cannot be researched at all; it is a conversation to have at the first event and write down afterwards.

**P1 is a hardware task.** Buy the DS3231, install it on the GPIO header, enable the overlay, and verify the clock survives a cold power cycle. Nothing in software can substitute, and every timestamp-ordered design decision downstream assumes it is done.

---

## Phase 1 — A skeleton that runs

The goal is an app a scout can sign into and submit through. No sync, no offline, no analysis.

### Data layer

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| D1 | `users` + `sessions` (SQLite) | RS §2.1 | S | **Done** |
| D2 | `teams`, `events`, `event_teams` — **with UNIQUE on `teams.team_number` and `events.tba_key`**, so upserts are real `ON CONFLICT` instead of a select-then-insert race | RS §2.2, §12.11 | S | **Done** |
| D3 | `matches` — **drop all ~38 dead 2022 score-breakdown columns**; key on `tba_key` and retire the `set_number * 100 + match_number` hack | RS §2.3, §12.1 | M | **Done** |
| D4 | **Season schema format + `seasons/2026.json`** — the field definitions everything else renders from. The 2026 game is **Rebuilt** | RI-U1 · RS §2.4 | M | **Done** |  |
| D5 | Scouting tables as **`payload` JSON + `schema_version`**, not fixed 2022 columns. This is the fix for the January rewrite treadmill | RI-U2 · RS §2.4, §12.1 | L | **Done** |
| D6 | **Add `match_id` to scouting rows.** Without it you cannot tell two observations of one robot apart, detect duplicate or missing coverage, or correlate an observation with the result | RS §12.2 | S | **Done** |
| D7 | **Add `client_record_id` (UUIDv7) + unique constraint to every client-originated table.** Cheap now, painful to retrofit — and Phase 3 sync depends on it | RI-S8 | S | **Done** |
| D8 | `team_event_stats` with `REAL` columns — the `::float8` cast dance disappears under SQLite | RS §2.5, §12 | S | **Done** |
| D9 | `devices` + per-match `scout_assignments` (`UNIQUE(match_id, team_id)`, `CHECK(scouter_id OR device_id)`) | RS §2.6 | S | **Done** |
| D10 | `scouting_point_weights`, `pick_list_entries` | RS §2.7 | S | **Done** |
| D11 | Migration runner — **safe default with explicit opt-in to reset**, and no `DROP TABLE … CASCADE` preamble. The old default erased everything on one missing env var | RS §2.8, §12.10 | M | **Done** |
| D12 | Do **not** create `awards` or `zebra_data` until something writes them; drop `status`/`rejection_reason` unless F-phase builds the workflow | RS §12.3 | — | **Done** |

### Auth and identity

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| A1 | Argon2id password hashing — the bcrypt cross-port compatibility constraint died with the other ports | RS §3 | S | **Done** |
| A2 | Session create / read / expire-on-read / delete; cookie `HttpOnly`, `SameSite=Lax`, `Secure=false` (plain HTTP LAN) | RS §3 | S | **Done** |
| A3 | Generic "Invalid email or password" on both failure modes — preserve the anti-enumeration behavior | RS §3 | S | **Done** |
| A4 | **One typed role-guard extractor**, replacing the per-handler `if !user.is_admin && !user.is_lead_scout` repetition | RS §4 | M | **Done** |
| A5 | Device identity: `localStorage` UUID → ten-year cookie → 60s heartbeat; `COALESCE` on upsert so a borrowed device keeps its first team | RS §3 | M | **Done** |
| A6 | Derive user "online" from **heartbeat**, not from an unexpired 24-hour session. Auto-distribute was assigning robots to people who had gone home | RS §12.13 | S | **Done** — completed with L2; see Phase 2 notes |

### Core interface

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U1 | Layout + role-gated nav, Askama compile-time-checked templates | RS §7 | M | **Done** |
| U2 | **Event selection as client/URL state, not `sessions.selected_event_id`** — bookmarkable, multi-tab, and a precondition for offline. Persistent header switcher; allow multi-event analysis | RI-U9 · RS §12.12 | M | **Done** — multi-event analysis waits for U21 |
| U3 | Event summary: team count, match count, roster, "your team is not listed" warning | RS §5.1 | S | **Done** |
| U4 | **Schema-driven submission form renderer** reading D4 — replaces per-season template branching | RI-U3 · RS §5.2 | L | **Done** |
| U5 | Account page, change password, help page | RS §5 | S | **Done** |
| U6 | **Vendor Unpoly and the Tailwind build locally — never CDN.** Hard requirement for an event LAN, not an optimization | RS §7 | S | **Done** — nothing loads from a CDN; there is no Unpoly to vendor (U8) and no Tailwind build |
| U7 | Tailwind component layer (`.btn` / `.card` / `.form-*` / `.alert` / `.badge` / `.data-table` / `.nav-link`) + teal palette | RS §7 | M | **Done** |
| U8 | Re-create or deliberately design away the three Unpoly glue contracts: `tt:navigate` via `X-Up-Events`, `[tt-src]` polling regions, `[tt-change]` select-driven render | RS §7 | M | **Done** — designed away; live regions in `live.js` |
| U9 | Dual-mode responses keyed on `X-Up-Version` — keeps the app usable when Unpoly fails to load | RS §7 | M | **Not needed** — one response mode (U8) |
| U10 | Error and success fragments **in templates**, not inline Rust format strings | RS §7, §12 | S | **Done** — no Rust builds HTML; one layer turns every other error into a page |

### Phase 1 notes

**Done: the data layer and auth, end to end.** 116 tests. `cargo run -p tt-web` gives a working app: sign up, sign in, sign out, change password, account page with role badges, and device heartbeats — against a migrated WAL-mode SQLite database, with role guards enforced.

**D4 shipped as `crates/tt-core/seasons/2026.json`** — the 2026 game, Rebuilt, embedded at compile time. Embedding rather than reading from disk means a schema typo is a red build instead of an event-day surprise, and the deployed binary can never drift from the schema it was tested against. Editing it is a rebuild, which at Kickoff is the right trade.

**Access control is a type, not a convention.** `LeadScout` / `Coach` extractors mean a handler that lacks the guard fails to compile rather than shipping open. `/lead-scout` and `/drive-coach` exist and are guarded now, with placeholder content, so the nav never links a 404 and the access rule is settled before there is anything on the page worth protecting.

**One table for observations, not two.** The retired design had `scouting_submissions` and `scouting_data` with near-identical columns and copied rows between them, deleting on both approve and decline. This schema has one `observations` table with a `review_state`; approve and decline are updates, nothing is destroyed, and L10's retract-not-delete requirement is already satisfied by the schema.

**Tailwind is not in the build.** `static/css/site.css` is hand-written using the component class names REBUILD_SPEC §7 documents (`.btn`, `.card`, `.form-*`, `.alert`, `.badge`). That removes Node, npm, and a TypeScript compiler from a workflow maintained by students who graduate every four years. Swapping a Tailwind build back in later means replacing one file, not rewriting templates. **This is a deliberate deviation from the stated stack** — revisit at U7 if the utility classes are wanted.

**Event selection is URL state (U2, U3).** Pages take `?event=2026mabil`; the header switcher is a GET form with no action, so choosing an event reloads the page you are on, for that event, and every nav link carries the parameter onward. Nothing is stored anywhere — which is what makes a page bookmarkable, lets two tabs show two events, and means the choice needs no server memory offline. The retired `POST /api/events/select` has nothing left to do and was not rebuilt.

**With no `?event=`, the page shows the event running today**, else the next to start, else the most recent (`tt_core::records::default_event`). A viewer is offered their team's events, or every event when they have no team *or their team is on no roster yet*. An event named in the URL is shown even if it is not on that list, and an unknown key renders the default with an error naming the key rather than a 400. "Today" is UTC's date, so near midnight at a US event the default can pick a neighbouring event — correcting that needs the event's zone (Q5).

**Multi-event analysis is not part of this.** The refurbish plan's point was to scope *data entry* to one event and let *analysis* span several; there are no analysis screens yet, so that half lands with U21.

**A scout can now record a match (U4).** `/submission` is three steps, all URL state like U2: pick the match (`?match=`, defaulting to the first unplayed), pick one of its **six robots** (`&team=`), then fill in a form drawn entirely from `seasons/2026.json` — no template names a 2026 field. A robot is never chosen from the event's list of fifty, and the alliance is read off the match rather than asked, which removes the two wrong-robot paths the retired form had. Assignments (L3-L5) should preselect a robot in this picker, not replace it.

**Selects are rows of big buttons, counters are − / + around a number, and everything used mid-match is 56px** (RI §2F). It works without JavaScript: the match select has a Go button, and counters are plain number inputs until `static/js/counter.js` reveals their buttons — they are rendered `hidden`, so a page without the script never shows buttons that do nothing. `[hidden]` now beats component `display` rules in `site.css`. Checked by screenshot at 360 and 390px, not only by tests.

**Untouched is not the same as unrecorded.** A counter starts at its minimum, so leaving it alone records a real 0; an unticked box records `false`. Only an unchosen select, a cleared counter, or blank notes are left out of the payload. The reading lives in `tt_core::form`, so a service worker can run the same code offline (C5).

**Saving is idempotent (D7).** The form carries a UUIDv7 minted at render (`tt_core::record_id`, clock and randomness passed in, so wasm-clean); a double-tapped Save or a retried post stores one row. The id survives a failed save, so fixing a mistake and saving again cannot store two. A second, *different* observation of the same robot in the same match by the same scout is refused by the coverage index with a message saying so — and choosing that robot again says so before any typing. Correcting a saved observation waits on L10's decline.

**Rows are written pending, with provenance:** scout, tablet (from the device cookie), the scout's team resolved at write time (L7), the schema version, and `observed_at`. A failed save re-renders with every answer and a message per field; success redirects (303) to a confirmation that is shown only if the row is really there, not because the URL says `saved=`.

**A robot on no synced roster can still be scouted.** TBA's schedule routinely names teams before FIRST's roster sync creates them, and `observations.team_number` references `teams`. Recording inserts a placeholder `Team N` row if needed; the next roster sync renames it.

**A schema papercut this surfaced:** `no_show` says "Tick this and skip the rest", but `starting_position` is required, so a no-show still needs a position picked. Either drop `required` from `starting_position` or change the hint — a schema-owner call (Open decision 2), so it is left as is.

**No Unpoly (U8, U9).** The retired app's three glue contracts are designed away, not rebuilt:

| Retired contract | Now |
| --- | --- |
| `tt:navigate` via `X-Up-Events` — server-driven navigation after login, logout, approve | Every form is a plain POST answered with a 303, or with the page re-rendered around the error. Browsers follow redirects on their own. |
| `[tt-change]` — a select renders a fragment | `data-autosubmit` on a select in a GET form (`static/js/autosubmit.js`): a navigation to a URL holding the choice, so it is bookmarkable, with a `<noscript>` Go button. The event switcher and the scouting match picker use it. |
| `[tt-src]` — self-loading and polling regions | `data-live="<url>" data-live-every="<s>"` (`static/js/live.js`): the region re-fetches a **whole page** and swaps in the element with its own id. The page is the fragment — no fragment routes, no second template, nothing to fall out of step. |

Why: every page already worked as plain HTML. Unpoly's gain would be skipping a few KB of layout per click on a wired LAN; its cost is a second response mode on every handler (U9), a third-party attribute language to learn and upgrade, and target mismatches that fail silently (REBUILD_SPEC §7). Offline is unaffected: a service worker (C1, C5) intercepts page navigations and live-region fetches alike, and can render the same Askama pages. Reversible: Unpoly later is one vendored file plus `up-*` attributes, and nothing here fights it.

**Live regions keep what they have when a refresh fails.** `live.js` pauses while the tab is hidden, never swaps content from under the focus, skips unchanged content, and on any failure leaves the content dimmed under "Not updating — showing what the server last said." A refresh that lands on sign-in after the session expires finds no element with the region's id, so a sign-in form is never swapped into a card. Each refresh is a full page render — fine at 30 s on a Pi, but a page that grows expensive should get a cheaper URL of its own rather than a slower timer. S8's server push can later trigger the same refresh.

**The first live region is the lead scout's sync card**, because the background sync keeps working after **Sync now** returns. The card now also counts what is stored for the selected event (teams, matches, played), so matches can be watched landing. The outcome of the press sits outside the region, so a refresh cannot take it away mid-read. Tested: a flow test fetches every `data-live` URL on the page and requires exactly one element with the region's id, and another requires every `/static/` asset a page loads to be served. In headless Chrome against the binary, a tampered card was restored by the next poll, and after sign-out the card was kept and marked stale.

**U9 has nothing left to do.** There is one response mode, the one that works without JavaScript. `POST /api/frc/sync` still answers scripts with JSON by `Accept`, which is a different split and stays.

**Errors are templates, and a browser never gets a blank screen (U10).** The spec's defect — error fragments built as Rust format strings with hand-called escaping — has no counterpart here: no Rust code builds HTML, handlers pass plain-text messages, and Askama escapes them. Form errors were already in each page's template, beside the field. One alert was inconsistent: a failed **Sync now** was announced as a polite status; it is now `role="alert"`, like every other error.

What was left was everything that goes wrong *around* a handler. axum answers those with an empty body or a line of plain text, which on a phone is a dead end with no nav:

| A browser reaches | It got | It gets |
| --- | --- | --- |
| A mistyped link or old bookmark (404) | A blank page | **Page not found**, with a way home |
| A form's address opened as a page (405) — after a failed sign-in or save the address bar shows `/api/…`, so history or a restored tab lands there | A blank page | **Nothing to show here**, saying nothing was sent |
| A form body the extractor rejects, e.g. a page older than the server (422, 415) | ``Failed to deserialize form body: missing field `password` `` | **That did not work**, and reload the page |
| A fault inside a handler (5xx) | Plain text | **Something went wrong** — the internal text stays out of sight |

This is one layer, `crates/tt-web/src/errors.rs`, not a handler per case, so a route added later cannot forget it. It rewrites a response only when the request asked for `text/html` and the body is not already HTML, and it keeps the status and headers (a 405 keeps `Allow`). Scripts, `fetch`, and a page's own `<script>` and `<link>` loads do not ask for HTML and get the original response; a script can still read why its post was refused. The text it replaces is logged, since that is the part worth reading. The page has the full layout with the right nav for whoever is signed in, and shows the status and address so a scout can read them out. The wording lives in `pages/error.html`, chosen by status. Checked against the binary and at 390px, and each guard in the layer was removed once to confirm a test fails without it.

**Phase 1 is complete.**

---

## Phase 2 — Event-ready

Everything a competition weekend actually needs. This is the phase that must ship before kickoff.

### Assignment-driven scouting

This is the highest-leverage cluster in either source document. It removes the 50-team list problem at its root and eliminates wrong-robot entry.

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| L1 | Assignment grid: matches × six robot slots, `"TBD"` for teams not in the local roster | RS §5.6 | L | **Done** |
| L2 | Set / auto-distribute / clear-all / clear-match / rename-device | RS §5.6 | M | **Done** |
| L3 | **Assignment-driven team selection** replacing the team list, with a keypad escape hatch | RI-U4 · RS §5.2 | M | **Done** |
| L4 | Prefill query — next unplayed match, matching `scouter_id` **OR** `device_uuid` | RS §5.2 | M | **Done** — `tt_core::assignments::agenda` |
| L5 | **Lock the scouting form to the assignment**, pre-filled and restricted, with a deliberate override | RI-A1 · RS §5.2, §12 | M | **Done** |
| L6 | **Coverage view**: who is assigned, who has submitted, which robots are uncovered | RI-A3 | M | **Done** — on the assignment grid |
| L7 | Resolve `submitting_team_id` at write time — it drives the notes privacy rule, and missing it once already required a backfill migration | RS §5.2 | S | **Done** — with U4's save |

### Review pipeline

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| L8 | Pending queue ordered by `created_at`, missing-notes flag | RS §5.3 | S | **Done** |
| L9 | Approve: one transaction, copy into canonical + retract from queue | RS §5.3 | M | **Done** — an update, not a copy |
| L10 | **Decline → retract, not delete**, with an audit record and feedback to the scout. The old path destroyed data silently with no correction route | RS §12.5 | M | **Done** |
| L11 | Ranking score: weighted sum per row, then **averaged, with `n=` shown**. Summing rewarded volume alone | RS §5.5, §12.9 | M | **Done** |
| L12 | Weight editor: `weight_{metric}__{option}` fields, `[-100, 100]`, whole-form rejection on invalid input | RS §5.5 | S | **Done** |

### Team analysis

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U11 | **Consolidate team stats into one `TeamProfile` view model** — synced stats plus scouting aggregates, empty strings for absent values rather than zeros | RI-U8 · RS §5.4 | M || **Done** — `/teams?team=N` |
| U12 | **Pick one aggregation rule.** Mode for some fields and latest-row for others was an accident, not a design | RS §5.4, §12 | S || **Done** — `tt_core::profile` |
| U13 | Notes filtered to the viewer's own `submitting_team_id`; no-team viewers see none | RS §5.4 | S | **Done** — `tt_core::notes::Notes`, on the review page and the team profile |
| U14 | **Provenance badges** (`n=`, `scouted_at`, `synced ago`) on every aggregate | RI-U7 | S | **Done** — rankings, the team profile, and the drive coach panel |
| U15 | **Remove synchronous upstream calls from page renders.** `/teams` and the team-select fallback both blocked a render on the network | RS §12.7 | M | **Done** — no render calls upstream; `no_page_asks_upstream_even_when_storage_has_nothing` holds it |
| U16 | Mobile pass: bottom nav, 44px touch targets, card layouts under 600px | RI-U10 · RS §7 | M | **Done** — tab bar under 40rem, `.cards` tables, no target under 44px |
| U17 | DB viewer — **guard with `is_admin` and exclude `sessions`**, or do not rebuild it. The old one was completely unguarded and exposed every user's email and all session rows | RS §12.4 | S |  |

### Upstream data

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| I1 | FIRST client: basic auth, 3 attempts, retry only on 429/5xx, backoff 250/500/1000 ms, 4096-byte error truncation, rustls | RS §6.1, §6.3 | M | **Done** |
| I2 | TBA client: `X-TBA-Auth-Key`, same retry policy | RS §6.2, §6.3 | S | **Done** |
| I3 | **TBA field-fallback deserializers** (the `effective_*` family). Read `TBA_SCHEMA_FIX_SUMMARY.md` first — schema drift across seasons is the recurring failure mode | RS §6.2 | M | **Done** |
| I4 | FIRST sync: event/team/event_teams upserts, `tba_key = {year}{code}`, lenient three-format date parsing, country-filter precedence | RS §6.1 | M | **Done** |
| I5 | TBA stats sync → `team_event_stats`; component OPRs non-critical (log and continue with nulls) | RS §6.2 | M | **Done** |
| I6 | TBA match sync: `played` derivation, `winning_alliance`, `red1..blue3` from `frc` keys, unix `0` → `NULL` not epoch | RS §6.2 | M | **Done** |
| I7 | Background loop: 2 min during active events, 3 hr otherwise, ±7-day fallback, 24-hr lookahead, 120s per-pass timeout | RS §6.2 | M | **Done** |
| I8 | **Pre-event bulk load** — full upstream snapshot, one command, verifiable row counts. An afternoon of work that covers most of the tedious data before you leave the shop | RI-S5 | S | **Done** — `tt-web bulk-load` |
| I9 | ETag / conditional requests on the TBA poller | RI-S10 | S | **Done** — in-memory, inside `TbaClient` |
| I10 | Connectivity tracker: TCP connect to `1.1.1.1:443`, 1500 ms, 3s cache, skip loopback/RFC1918/link-local | RS §6.4 | S | **Done** |
| I11 | **Four-state connection chip describing the client's link to the server**, not the server's internet — and remove all "offline mode" toggle language | RI-O11 · RS §6.4, §12 | S | **Done** — the header chip, `tt_core::link`, `static/js/link.js` |
| I12 | **Upstream freshness badges**; amber past 20 minutes during quals. Stale rankings that look live cause bad picks | RI-S11 | S | **Done** — `connectivity::Freshness`, with U14 |
| I13 | `POST /api/frc/sync` manual sync, admin/lead only | RS §6.1 | S | **Done** |
| I14 | **Manual rankings entry screen** — the true last resort. A lead scout can type 40 rows off the audience display in five minutes, and it has never once failed to work | RI-S13 | S | **Done** — `/lead-scout/rankings/enter` |
| I15 | **Store each FIRST event under TBA's key.** FIRST and TBA disagree on the code for every Championship division and about fifty offseason events (`MILSTEIN` / `2026mil`), so a key built from FIRST's code 404s at TBA | Q2 | S | **Done** — `tt_core::upstream::tba_keys`, one `/events/{year}` request per sync |

### Coach and pick list

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U18 | **Coach panel reads the local `matches` table**, not the live FIRST schedule. It was non-functional offline, at exactly the event where it matters most | RS §12.6 | M || **Done** — `/drive-coach`, `tt_core::coach` |
| U19 | Match status classification (±15 min windows) as a pure function in `tt-core` | RS §5.7 | S | **Done** — `tt_core::matches::classify`, since the ingestion commit; see Phase 2 notes |
| U20 | Pick list read / upsert / delete | RS §5.8 | S | **Done** — `/pick-list`; edits, not positions, so two people at once both land |

### Platform

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| P3 | SQLite WAL, single writer, on **NVMe/USB SSD — not the SD card** | RI-N2 · RS §10 | M | **Done** (software) — `tt_repo_sqlite::storage`, [PI_STORAGE.md](PI_STORAGE.md); the hardware steps are untested |
| P4 | Avahi → `http://tealteam.local`. Removes the most common event-day support question | RI-N3 · RS §10 | S | **Done, untested on a Pi** — `deploy/pi/network/`, `docs/PI_NETWORK.md` |
| P5 | Asset resolution: walk up from both the exe and cwd, or embed assets in the binary | RS §10 | S | **Done** — embedded: `tt-web/build.rs`, `src/assets.rs` |
| P6 | Wired Ethernet to clients + USB tethering as the uplink (`usb0`, route metric). **Build no Wi-Fi AP** — it violates E143 | RI-N4, RI-N5 · RS §10 | M | **Done, untested on a Pi** — with P4; the rules check is still P2 |
| P7 | Buy per-client 25 ft flat Ethernet, gaff tape, and USB-C Ethernet adapters (~$15 each) | RI §1 | S |  |
| P8 | One-page laminated event-day setup runbook with a photo of the correct cabling | RI-N7 | S | **Done** — `docs/EVENT_DAY_RUNBOOK.md`; photo and server folder are fill-ins |
| P9 | Practice the full network setup and teardown twice at the shop, timed, by a student who did not design it | RI §1 | S |  |
| P10 | **Start the server at boot**: a systemd unit as a dedicated `tealteam` user, after the SSD is mounted (`RequiresMountsFor=/srv/tealteam`), restarting on failure | P8 | S | **Done, untested on a Pi** — `deploy/pi/service/`; `sudo ./install.sh path/to/tt-web` |

### Phase 2 notes

**Done so far: upstream ingestion, running on its own.** The FIRST and TBA clients, the uplink probe, and the sync that lands events, rosters, matches, and statistics in the database. 17 integration tests run the whole path — stub HTTP server, real clients, real SQLite — against payloads shaped like the real thing.

**The server now keeps itself current (I7).** At boot it pulls the FIRST event list in the background (60s cap, `FIRST_SYNC_ON_BOOT=false` to skip), then hands over to the TBA loop: every 2 minutes while an event is live, every 3 hours otherwise, with the ±7-day fallback so last weekend's final rankings still arrive. Serving never waits on either — a venue with no internet does not delay the first page. Neither starts when storage is down, and missing credentials switch the matching piece off with a log line saying so.

**Each pass has a 120s budget that bounds the network work only.** Choosing which events to sync is a local query and happens first, so a pass that overruns still knows whether an event is live and keeps the right cadence. What it stored before the cutoff stays stored.

**A lost uplink now ends a pass instead of probing once per event.** `SyncReport.offline` is set on the first offline error, and both the loop and the bulk load stop there. Before, a pass over thirty events with no signal paid thirty probe timeouts to learn the same thing thirty times.

**I4 and I8 were library code until this landed.** Nothing in the binary called `sync_events` or `bulk_load`, so "one command" did not exist. It does now: `tt-web bulk-load` applies migrations, loads everything, and prints per-event team, match, and stat counts **read back from the database** — what you check at the shop is what the Pi will serve. It exits non-zero if nothing landed. `tt-web help` lists the commands; a bare `tt-web` still serves.

**The two schema-drift bugs from `TBA_SCHEMA_FIX_SUMMARY.md` are fixed and pinned by tests.** Component OPRs are found by dynamic name (`totalAutoPoints`, not a fixed `auto_oprs` field), and ranking points fall back to `sort_orders` / `extra_stats` when the legacy primitives are null. Both have tests named after the symptom, so a future "simplification" to direct field access fails loudly.

**Partial success is the design, not an accident.** `SyncReport` carries counts and problems together: one event's roster failing does not abandon the other eleven, and a missing component-OPR endpoint does not discard the rankings that came with it. Only a total loss of connectivity aborts.

**Parsing is in `tt-core`, transport in `tt-upstream`.** That split keeps the deserializers wasm-clean for S4, where a client with signal fetches upstream itself and hands the Pi a bundle — the reason the refurbish plan needs no relay server.

**A lead scout can resync from the app (I13).** `POST /api/frc/sync` reruns the FIRST event sync (90s cap), then **wakes the background loop** so matches and statistics follow at once. The wake is the part that matters: a Pi that booted offline found an empty calendar and chose a three-hour pause, and without it the events a manual sync delivers would sit waiting out the rest. Only one FIRST sync runs at a time, boot or manual — a second press gets "already running" rather than doubling a hundred-request sync.

**One route, two callers.** A script gets JSON counts, per the spec. A browser posting the new **Sync now** button on `/lead-scout` gets the page back with the outcome, because without Unpoly (U8) a JSON body would be the whole screen. The page's "FIRST and TBA data" card shows the server's uplink, when it last synced (relative time, so no timezone question), and which feeds are configured. An uplink nobody has tested yet reads "Not checked yet", not "No internet" — otherwise a server with no credentials claims to be offline. `.badge-amber` was referenced by `UplinkState` but never defined in `site.css`; it is now, with `.badge-gray`.

**A lead scout can see the assignment grid (L1).** `/lead-scout/assignments` shows every match at the selected event down the side and the six driver stations across, each cell naming the robot, its team, and who is watching it — a scout by name, or a tablet by its label with "tablet" beside it. Upcoming matches come first; played ones fold away under a `<details>`, since nothing can be done about them. A count at the top says how many robots in upcoming matches have somebody on them. On a wide screen it is a table; under 48rem each match is a card of red over blue, the same shape as the scouting page's robot picker. Checked by screenshot at 1280 and 390px against the binary, with no sideways scroll on the phone.

Each row's id is its match key, so a change can land the lead back on the row it touched.

**"TBD" means an empty slot, not an unknown team.** The retired grid printed "TBD" as the *name* of a team missing from the roster. Beside a real team number that reads as "team to be decided", which it is not, so a known number with no roster entry shows the number and *not on roster*; "TBD" is kept for a slot the schedule has not filled, as in playoffs before alliance selection.

**An assignment the schedule moved away from is called out.** TBA revises schedules — replays, surrogates — and an assignment made against the old one points at a robot no longer in the match. Once assignments pre-fill the form (L4) that is a scout watching the wrong robot, the exact mistake assignments exist to prevent, so the grid lists them in a warning above the table (`tt_core::assignments::stale`). They are not counted as coverage.

**A failed read shows a message, not an empty grid.** If the schedule or the assignments cannot be read, the page says so instead of drawing a grid of "Unassigned", which would send a lead off to redo work that is already done. A failed roster read only costs the team names.

**The grid can be changed (L2).** Every change is a plain form post answered with a 303 back to the grid, scrolled to what changed, with a line saying what happened — so it works without JavaScript and a reload never posts twice. A refused change re-renders the grid with the reason.

| Retired | Now |
| --- | --- |
| `POST /hx/assignments/set`, one robot per request | **Tap a match** (`?edit=<match>`): its six robots, each a select of every scout and tablet, online ones marked. **Save**, or **Save and assign Q15**, because a lead assigns a match at a time. A robot left blank is unassigned. |
| Nothing stopped one person getting two robots in a match | Refused, naming them, with the picks kept so it can be fixed |
| An assignment to a robot the schedule moved out lingered | Saving the match drops it; the grid's warning says to |
| `POST /hx/assignments/auto`: `pool[i % n]` over every open slot in every unplayed match; pool defaulted to everyone with a live session plus every recently-seen device | **Auto-distribute**: tick who is scouting, optionally "the next N matches". Walks the pool round-robin across matches, and **never gives anyone two robots in one match** — the retired version did whenever there were fewer than six people. With fewer people than robots the rest stays open, visibly. Existing assignments are left alone. |
| clear-all, clear-match: one click | **Clear Q14** in the match's editor; **Clear all assignments** needs a ticked confirmation box, which is a confirmation that works without JavaScript |
| `POST /hx/devices/:id/rename` | A **Tablets** list: name, online or when last seen, who last used it, and a rename field. A blank name goes back to `Device 0191f7ac`. |

**Who starts ticked in auto-distribute.** Scouts online now, and no tablets. A scout and the tablet in their hands are one person; ticking both would hand them twice the robots. The hint on the form says so — tick a tablet when it is shared.

**A6 was not really done until now.** It was marked done in phase 1, but nothing recorded users' presence: only devices heartbeat. Migration `0002_presence.sql` adds `users.last_seen_at` and `devices.last_user_id`, and a heartbeat from a signed-in page stamps both. "Online" is a heartbeat in the last three minutes, for a person or a tablet. The migration was checked on the L1 demo database as well as a fresh one.

**The schema facts that shaped the writes.** `scout_assignments.team_number` references `teams`, so assigning a robot TBA scheduled before FIRST's roster sync created it inserts a placeholder team first, as recording an observation does. Setting writes the scout *or* the tablet and clears the other, so no row names both. A batch — a saved match, an auto-distribute — is one transaction.

Checked against the binary at 1280 and 390px: a match edit, save-and-next, auto-distribute over three matches (which handed out one robot, correctly: two matches were full and the third had one free person), and a stale assignment dropped by saving its match.

**Worth knowing:** every browser that opens the app becomes a device, so the Tablets list and the pool grow with every personal phone that visits. If that gets long at an event, hide devices not seen for a day or so.

**A scout is handed their robot (L3-L5).** Opening Scout with an assignment skips both picking steps: the page says **You are scouting 3310 · Q5 · Red 2** in large type, with the form already open and no robot picker to mis-tap. That is the lock. Leaving it is deliberate — **Not your robot? Choose another** opens the picker, where the assigned robot is marked *Yours* — and choosing a different robot says so: "You are assigned team 3310 in Q5. Make sure 971 is the robot you are watching." The save is still accepted. The scout is the one looking at the field, and a refused observation is worse than a flagged one.

**Which assignment (L4).** The first one in an unplayed match that the scout has not already recorded, naming either their account **or the tablet they are on** — the spec's `scouter_id OR device_uuid`, so "the tablet on the left" works whoever signs in on it. Assignments to a robot the schedule has since moved out are skipped. It is a pure function over the schedule, the event's assignments, and what the scout has recorded (`tt_core::assignments::agenda`), so C8 can run it offline unchanged. Opening a match from the picker's select or arrows also lands on the scout's robot in it, if they have one there.

**"Unplayed" is not the whole story.** TBA marks a match played minutes after it ends, often while a scout is still typing, and a scout who misses one should still record it. So an assignment in a played match the scout has not recorded is not dropped: it is listed under **Still to record**, one tap each. After a save the confirmation offers the next assignment ("Scout team 10101 in Q3") rather than simply the next match.

**The keypad (L3).** Under the robot picker: **Or type a team number**, a numeric field whose type-ahead comes from a `<datalist>` of the event's roster — `16` offers `166`, `1619`, `1678` — with no script. `?team=1678` with no match opens that team's next unplayed match, or its last one once all are played. It is the escape hatch for a robot that is not where the scout expected; the assignment and the six-robot picker remain the main paths.

Checked on a phone-width screenshot against the binary, signed in as a scout with assignments.

**The grid shows coverage (L6).** Not a separate page: the grid already shows who is assigned, so it now shows what happened too. A played robot is **Recorded** (×2 when two scouts saw it), **Missed** — assigned and nobody recorded it — or **Not scouted**, never assigned and never recorded; the last two are outlined in red, so the holes in the data are what stands out. A robot recorded by *anyone* counts as covered, whoever it was assigned to.

The top of the page counts both halves — "19 of 60 robots in upcoming matches have a scout", "22 of 24 robots in played matches were recorded" — and the played section's heading carries the gap ("Played matches (4) · 2 not scouted") so it can stay folded. A table then answers **who has been submitting**: per scout or tablet, assigned robots they recorded themselves, missed, and still to come, with who is online now. A tablet's robots count as recorded when saved from that tablet, whoever held it. The logic is pure (`tt_core::assignments::{slot_state, tallies}`).

**It keeps itself current.** The counts and both grids are live regions refreshing from the grid's own address every 30 seconds, so the lead watches submissions land without reloading — and live.js leaves a region alone while the focus is in it. Declined observations will not count once L10 exists; the query already excludes them.

**The review pipeline (L8-L10).** The lead-scout page has a **Waiting for review** card for the selected event: oldest first, so the top is always the next to look at, each row showing the robot, match, station, scout, and how long ago, with the retired queue's flag — **Missing notes** in amber when every free-text field is blank, **Clean** in teal. Each row can be approved where it is; tapping it opens the observation in full at `/lead-scout/submissions/{id}`, every answer labelled from the season schema in form order (options by their label, not their key), plus anything from an older form version under "Not on the current form" so nothing a scout recorded is hidden. The card refreshes itself while scouts keep submitting.

**Approve (L9) is an update, not a copy.** One table with a review state means approving sets the state, who, and when — nothing moves and nothing is deleted. It also fills in the scout's team where the row lacks it, as the spec's approve did. **Approve and see the next** goes straight to the oldest still waiting, then back to the queue when it is empty.

**Decline (L10) keeps everything and tells the scout.** A reason is required (500 characters at most): "declined" alone cannot help a scout do better. The row keeps its answers, the reason, who declined it, and when. The scout's scouting page then says "Kim declined your record of team 254 in Q2: “That was 1678”" with **Record it again**; the coverage index already ignores declined rows, so re-recording works, answers the notice, and the robot returns to the scout's agenda and to the grid's coverage as not yet recorded.

**One verdict per observation.** The update only applies to a pending row, in one statement, so two leads pressing at once cannot both record a verdict: the second is told it was already reviewed, and shown by whom.

The retired approve also started a background FIRST sync for the observed team. Not rebuilt: the background loop already keeps teams current, and a review should not reach for the internet.

Checked against the binary at phone width: the queue, the review page, a decline that moved on to the next, and the scout's notice.

**The TBA poller asks before it downloads (I9).** Every TBA request carries the `ETag` of the last body the client got for that path, as `If-None-Match`, and an unchanged resource comes back as an empty `304`. During quals the loop asks for four resources per live event every two minutes, and most of them have not moved since the last ask. On a phone tether, that is most of the data the Pi would otherwise spend.

**A 304 is still a sync.** The client parses the body it already holds and hands it over as if it had just arrived, so the upserts run and `synced_at` advances. Freshness (I12) means "when we last heard from TBA", and a 304 is TBA saying the stored copy is current. `sync.rs` did not change.

**Kept in memory, not in the database.** The cache lives in `TbaClient` and is shared by its clones, so the loop and a manual sync revalidate against each other. It holds the 64 most recently used paths, sixteen events' worth. A restart costs one full fetch per resource, which is cheaper than a table and a migration. Only a body that parsed is kept, and a `304` the client did not ask for is an error, not empty data. The tests use a stub that honours `If-None-Match` and counts the full bodies it sends; with the header switched off, four of the five fail.

**Not yet seen against live TBA.** That TBA exposes `ETag` is recorded in REFURBISH_PLAN's CORS notes, but no request in this change has gone to the real API. At the next shop session with a key, run a sync twice with `RUST_LOG=tt_upstream=debug` and look for `not modified` on the second pass.

**Rankings (L11).** `/lead-scout/rankings` lists every team at the event — the roster, plus anyone ranked or scouted who is not on it yet — with the event's own rank and a **scouting score: the average of its approved observations, with n beside it**, marked *thin* under three. The retired score was a sum, so a team scouted twelve times outranked a better one scouted four times (§12.9). Only approved observations count, and only those recorded on the current form version, since an older version's fields mean different things; pending ones are counted in a line saying they are not included yet. Column headers sort by rank (unranked last), score (highest first, unscored last), number, or name, ties broken by team number then name, with the order in the URL. The logic is pure (`tt_core::ranking`). The rank column reads `team_event_stats`, so manually entered rankings (I14) appear there with no change.

**Point values (L12).** `/lead-scout/weights` shows every scored answer — each option, a counter's value per piece, a toggle's value when ticked; free text is never scored — with its current points and the season default. Whole numbers from −100 to 100, and **one bad value saves nothing**, the form coming back as typed with the value marked. Only values that differ from the default are stored, so a stored row always means "changed on purpose", the page can say how many are changed, and **Put every value back** simply empties the table. A row naming a field the schema no longer has is ignored by scoring, as before. Weights are global rather than per event, as the table always was.

Checked against the binary at phone width, which caught the rankings table pushing the score off the edge of a 390px screen; its cells tighten and names wrap there now.

**Rankings can be typed in (I14).** `/lead-scout/rankings/enter` is one text box: one team per line, best first, **and the line is the rank**. So a rank is never typed, and can never disagree with the order. After the team number, a ranking score and a `W-L-T` record are optional, in either order: `254 3.42 11-1-0`. Spaces, tabs, and commas all separate, so a pasted spreadsheet column works too. The box opens holding the stored ranking, so correcting two rows is editing two lines, not retyping forty. It is linked from the Rankings page, and from the Lead Scout sync card beside **Sync now**, which is where a lead is when TBA cannot be reached.

**All of it or nothing.** Every bad line is named at once ("Line 7: team 254 is already ranked 2."), the text comes back exactly as typed, and nothing is stored until there are no errors: a half-applied ranking is worse than a stale one. A number not on the event's roster is refused, because it is almost always a typo or a rank typed where the team belongs, and the message says so. With no roster synced at all, any number is taken.

**A typed ranking replaces the stored one; it does not merge.** Listed teams get their rank, score, and record, with `synced_at` set to now. Their other ranking columns are cleared rather than left under a fresh timestamp that would vouch for them. OPRs are kept, since they are not on the display. Teams left out lose their rank, so a typed top twenty never leaves a stale "5th" beside a new one. It is one transaction (`Repo::record_standings`), and the next TBA sync overwrites it. It writes only to the event the form named, never to a default. The parsing is pure (`tt_core::standings`), so a client can run it offline.

Checked against the binary at 390 and 1280px on a seeded 40-team event: the prefilled box, a refused save listing three bad lines in one alert, and a good save showing on the Rankings page.

**U19 was already done.** `tt_core::matches::classify` implements the ±15-minute rule, with boundary tests, and has been there since the ingestion commit. **For U18:** it reads only the clock, so when an event runs 20 minutes behind, a match that has not been played reads "Completed". The coach panel should trust `matches.played` first and use the window only to pick out the current match.

**The team profile (U11, U12).** `/teams?team=N` (signed in; a **Teams** nav link, a team-number box with roster type-ahead) brings together who a team is, the statistics FIRST and TBA have published for it at the selected event — only those published, never a zero for "not yet", with when they were synced and a stale badge — what scouts saw, its matches with results and a Scout link each, and its other events. It never waits on the internet: a team this server does not know is said so (REBUILD_SPEC.md 12.7). **One aggregation rule (U12):** every field over all approved observations on the current form version — a choice is a tally, most common first, ties in form order; a counter its average and best; a yes/no how often. No field is "latest row" any more. Its **Notes** card is U13's: see below. Switching event in the header drops the team; the page's "Other events" links cover that for now.

**Notes are read only by the team that wrote them (U13).** One rule, in `tt_core::notes::Notes::for_viewer(viewer_team, submitting_team)`: notes show when both are set and equal, and never otherwise. A viewer with no team reads none, and notes saved with no team are read by nobody, since two teamless people are not a team. The numbers are shared; only the prose is held back.

**The one leak was the review page.** Any lead, of any team or none, could open `/lead-scout/submissions/{id}` and read every answer, notes included. It now shows the numbers and choices as before, and in place of held-back notes: "Only scouts on team 254 can read these notes." `review::answers` takes the viewer's `Notes` and marks those answers `hidden`, with no text in them. **Free text from an older form is held back too.** The page lists answers the current schema no longer declares, and a dropped field's text cannot be told apart from notes, so any undeclared text answer is treated as notes. Numbers and yes/no answers from old forms still show.

**What U13 did not change: who reviews what.** The queue still lists every team's pending observations, and any lead can approve or decline them. That is now a question of workflow, not of privacy. A lead can review another team's scouts without reading their notes. Whether a lead *should* is an open decision.

**The team profile's Notes card.** `/teams?team=N` lists the notes the viewer's own team wrote on that team's approved observations, in match order, each headed with its match and scout ("Q14 · Priya"). Other teams' notes are left out, not counted. A viewer with no team is told that notes belong to the team that wrote them, and sees none. The notes come from the same observations as "What scouts saw", and `tt_core::notes::written` picks out the filled-in text fields.

Checked against the binary at phone width: a lead on 10101 opening a 254 scout's observation sees the counts and choices, and the notes card says whose they are. The page's HTML carries none of the text. On team 254's profile, Sam (10101) reads their own note, and Kim (on 254) is told team 254 has written none there. Kim's HTML carries none of Sam's text.

**The drive coach panel reads the local schedule (U18).** `/drive-coach` shows the coach's team's matches at the selected event from the `matches` table the background sync fills — so it works with no internet, where the retired panel, which fetched FIRST live, showed nothing (§12.6). **The feed, not the clock, says what is played:** a played match shows its result; the first unplayed one is **Next**, however late the event is running; the clock only describes it — "in 12 min", "due now", "running 22 min late" — relative, so no timezone is needed (`tt_core::coach`). Each card shows our alliance and theirs with every team's OPR and DPR from the local stats, the alliance's OPR total (marked when some are not synced yet), and each team linked to its profile. Played matches are listed latest first. The schedule is a live region, refreshing every 30 seconds. A coach with no team number, or a team not on the schedule, is told so. The role-guarded placeholder page it replaces is no longer used by any route.

**Every aggregate says where it came from (U14, I12).** Rankings: the score line says when the latest counted observation was recorded, beside the `n` column L11 already had, and the rank line says when ranks were last synced or typed in, or that none have been. The team profile: "From 5 approved observations, the latest recorded 12 minutes ago", and a field fewer of them answered says so ("avg 6.0 · best 9 · 3 answered"), so an average of two is never read as an average of ten. Yes/no fields already read "1 of 2". The statistics card keeps its "synced … ago". The drive coach panel says when its OPR and DPR were synced, with the same badge.

**Stale is amber only while the event runs.** `tt_core::connectivity::Freshness::of(synced_at, now, event_running)` says the age once for every page, and marks it stale past 20 minutes (`STALE_AFTER`) only when the event is on today. Before and after an event nothing upstream moves, and a permanent amber badge would teach people to ignore it. The Lead Scout sync card keeps its own rule, since it is about the sync, not an event's numbers. **"Today" is the UTC date,** as the event picker's is, so on an event's last evening in the Americas the badge goes quiet a few hours early. The right fix is the event's own timezone (TIMEZONE_HANDLING.md), for the picker and the badge together.

Checked against the binary at phone width: ranks synced 35 minutes ago read "last updated 35 minutes ago" with the amber badge on the rankings page and the team profile, and the badge goes when the event is over.

**The pick list (U20).** `/pick-list` is the viewer's team's list for the selected event, best first, for lead scouts **and coaches** — the lead builds it, and at alliance selection the coach is the one crossing teams off; a scout is sent home. A **Pick List** link is in the nav for both roles. Each row shows the place, the team (linked to its profile), its name and event rank, an optional colour tag (green, yellow, red, blue — named in text, not only by colour; what each means is the team's call), and full-size ↑ / ↓ / **Cross off** buttons; **More** holds move-to-place, the colour, and removal. Crossed-off teams keep their place, struck through, so an undone pick reads the same. Teams are added by number (type-ahead from the roster) or from **Not on the list**, the rest of the roster best-ranked first, one tap each. After a change the page comes back scrolled to the team that moved.

**Two people editing at once both land.** The retired API took whatever `position` the client sent, so the second of two reorders silently undid the first (§5.8). Here the browser sends an *edit* — "254 up one", "cross off 1678" — and `tt_core::picklist::apply` performs it on the list as stored when it arrives. The write is a compare-and-swap (`Repo::replace_pick_list` stores the new list only if the stored one still matches what the edit was applied to); if someone changed it in between, the edit is redone on the new list, up to three times. That covers moves, crosses, and tags from several people without a CRDT. **L14 is still wanted** for what this cannot do: offline edits merged later, and live updates without a reload. Rows keep their `client_record_id` across edits, so L14 can adopt them.

**Also fixed here: the empty band under short pages.** `.page` is a grid that grows to fill the screen, and its rows stretched to share the spare height, so every card on a short page grew an empty band — the gap under the assignment grid noted with L6. `align-content: start` on `.page`.

**Not built:** the retired JSON endpoints (`GET /api/pick-list`, `POST`/`DELETE /api/pick-list/entry`). Nothing calls them; the Phase 3 client will want its own sync shape anyway.

**No page waits on the internet (U15).** Neither of §12.7's calls was ever ported: `/teams` has read storage only since U11, and the scouting page's roster is storage only, even when empty (the robots come from the match; the roster only adds names). U15 makes that a rule the tests hold. `no_page_asks_upstream_even_when_storage_has_nothing` points FIRST and TBA at a stub that counts requests and never answers, seeds an event with a schedule but no roster, and opens every page, including unknown teams and an event with nothing. It fails if any page waits on the stub or calls it. The one request that still waits on the network is `POST /api/frc/sync`, the sync button, where waiting is the point. A new page should be added to that test's list.

**The connection chip (I11).** Every page's header says whether *this device* can reach the server: **Synced**, **Syncing…** while a save is on its way, or **Offline · nothing unsent**. Offline is a state the page observes, never a mode, and nowhere does the app call it one — a test fails if a template or script says "offline mode". The words and the order (unreachable, then sending, then review, then synced) are `tt_core::link::Link`. The layout renders every state with one shown, and `static/js/link.js` picks which: it checks `/health` every 30 seconds while connected and every 5 while not, and believes the browser's `offline` event at once. Without the script the chip says Synced, true of a page that has just loaded. The lead scout page's uplink row is now **"Server's internet"**, so the two are not confused: a Pi with no internet still takes every save.

**Two of the four states are waiting on their data.** "Offline · 4 saved" needs the outbox (C5), so until then offline says *nothing unsent*, which is true: a save made offline fails in the browser and is not lost silently. "3 need review" needs a count on the client (L8, and conflicts from Phase 3). `Link` already has both, with tests, so they only need a number.

Checked in headless Chromium at 390px and 1280px, with `/health` dropping the connection: the chip switches from Synced to "Offline · nothing unsent" on the next check and fits beside the brand at both widths.

**The mobile pass (U16).** Under 40rem (640px) the sections are a **tab bar fixed along the bottom of the screen**, where a thumb reaches. Each tab is 56px, since it is tapped mid-match, and the section you are in is lit (`static/js/tabs.js`, by the first part of the path). Wider, the same links are a row of tabs under the header's first row, which holds the brand, the connection chip, the event, and the account. Signed out there is no bar: the sign-in page is the only place to be. The breakpoint is 40rem, not 600px, to match every other breakpoint in the stylesheet. It covers everything under 600px.

**No tap target under 44px.** `.btn-sm` keeps its smaller text but not a smaller height. Sign out, Sign in, and the no-script Go are full-size buttons. The brand, standalone "Back to …" links (`a.back`), rankings sort links, and the team-number links on the coach panel, the team profile, and the pick list all get a 44px box. Links inside a sentence stay text, as WCAG allows.

**Tables become cards under 40rem.** A `.data-table.cards` turns each row into a card. The row's header cell is the card's title, and every other cell labels itself from its `data-label`. Rankings keep their sort links as a row of buttons above the cards, and each card shows the rank down the left, then the team and its name, then score and n. The coverage table's cards read "Kim · Recorded 1 · Missed 1 · To come 1". Both tables carry `role="table"`, as the assignment grid does, so screen readers still treat them as tables once they are drawn as blocks. Those two are the only tables left; the assignment grid already had its own card layout (L1).

Checked by a script in headless Chromium that measures every link, button, input, and summary, and the page width. Across all fourteen pages, signed in as an admin with a seeded event, at 360px, 390px, and 1280px: nothing under 44px outside a sentence, and nothing wider than the screen. Screenshots at 390px, 700px, and 1280px show the bar, the lit tab, the cards, and a header that no longer wraps the account onto a line of its own.

**The binary carries its own assets (P5).** Of the two options, embedding. The walk-up from the exe and the working directory was already there, and it worked under `cargo run` and from `target/`. But a binary copied to the Pi without `static/` beside it served every page with no stylesheet and no scripts, and logged nothing. Migrations, the season, the templates, and the timezone database were already compiled in, so the assets now are too, and the one file is the whole deploy. `build.rs` lists every file under `crates/tt-web/static/` and includes its bytes, and `src/assets.rs` serves them at `/static/…`. Editing a file there rebuilds the binary, so `cargo run` still serves the current CSS. No new dependency; `tower-http`'s `fs` feature is dropped.

**Browsers revalidate rather than guess.** Each file has an ETag of its bytes and `Cache-Control: no-cache`, so a browser keeps its copy and asks each time. The answer is a bodiless 304 until a new binary changes the file, and then every tablet picks up the new CSS on its next page load, with no hard refresh at the event. A missing file is still a bare 404, and only the embedded paths exist, so `../` leads nowhere.

Checked by copying the debug binary alone into a directory outside the repo and running it from there. `site.css`, `link.js`, and `tabs.js` came back 200 with their types, the CSS byte-for-byte the source's; a request with its ETag got a 304 with no body; `js/gone.js` got a 404.

**Events are stored under TBA's key (I15).** FIRST and TBA agree on most event codes, but not all. In TBA's 2026 list, 91 events have a FIRST code different from their own. Among them are all eight Championship divisions (FIRST `MILSTEIN`, TBA `2026mil`) and some fifty offseason events that FIRST syncs: the Arizona League's qualifiers are `AZGLE` to `AZGLE3` at FIRST and `2026azrl1` to `2026azrl4` at TBA. Every one of those got a key built from FIRST's code, which 404s at TBA, so it would never get a schedule, rankings, or OPRs. Now `sync_events` fetches TBA's `/events/{year}` once and stores each FIRST event under the key TBA lists for its code, in any case. FIRST's own code stays in `event_code` for FIRST's calls. Twelve FIRST events in 2026 are unknown to TBA, and those keep the built key, as they do when TBA is not configured. When TBA is configured but its list fails, the sync goes ahead on built keys and reports it. An event stored under a built key before this change stays in the database next to the right one; nothing deletes it.

**The database on the SSD (P3): the software side.** The server already had most of it. `SqliteRepo::connect` opened SQLite in WAL mode with `synchronous=NORMAL` and a pool of **one** connection, so its writes queue up one at a time, each waiting up to 5 seconds rather than failing. Now there are tests on a real file: one that journal mode is `wal`, and one that a second write waits while a transaction is open and then lands.

**What is new: the server says where its database is.** At startup, and in `tt-web bulk-load`, it logs the absolute path and the device under it: "database is at /srv/tealteam/data/tealteam.db on nvme0n1p1". If that device is an `mmcblk` (the SD card, or eMMC) the line is a **warning** pointing at the doc. `tt_repo_sqlite::storage::locate` looks the device up through `/sys/dev/block`, and falls back to the deepest mount in `/proc/self/mountinfo` for filesystems like btrfs whose device number has no block device behind it. Anything it cannot place is "an unknown device", never an error. The path is still set with `DATABASE_URL`; a second setting for the same thing would only be a way for the two to disagree.

**[PI_STORAGE.md](PI_STORAGE.md)** covers mounting the SSD by UUID with `nofail`, and putting the database in a `data/` folder *inside* the mount. If the SSD is missing, the server then cannot open its database and says **Storage unavailable**, instead of quietly starting an empty one on the SD card. It also covers moving an existing database with `.backup`, so what is still in the `-wal` file is not lost. **None of the hardware steps have been run on the Pi**, and the doc says so. Blind spots: an SD card in a USB reader shows up as `sda`, and a LUKS volume on the SD card as `dm-0`, so neither is warned about.

Checked on a development machine: a database on the encrypted btrfs root was reported "on dm-0" (through the mountinfo fallback), one on tmpfs "on an unknown device", and the SD-card wording by unit test.

**The Pi's network (P4, P6) is written but has never run on a Pi.** `deploy/pi/network/setup.sh` is idempotent and has `--dry-run`. It sets `eth0` to a static `10.101.0.1` with no default route, and runs dnsmasq DHCP on `eth0` only with **no gateway**, so tablets keep their own cellular. dnsmasq also answers `tealteam.local` over plain DNS for clients that do not do mDNS. Avahi advertises it, and a tethered phone (`usb0` Android, `eth1` iPhone) is the uplink at route metric 50. An nftables redirect lets the URL drop the port. It deletes any Wi-Fi AP profile it finds. `status.sh` is the read-only event-day check. What was verified here: ShellCheck, the nftables ruleset loading twice in a scratch namespace, and a dry run. `docs/PI_NETWORK.md` lists the nine shop checks still owed. The first is whether Android routes to a wired network with no internet; the doc explains why that matters. P2's E143 question is still open. This follows the plan: design for the compliant path, and unplug the phone if the FTA objects.

**Still open in Phase 2:** U17, P7, and P9.

---

## Phase 3 — Offline-first and client-centred

The architectural payoff. Phase 2 must be shipping before this starts.

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| C1 | **Service Worker + app-shell precache + navigation fallback.** WASM alone makes nothing offline; this is the piece that does | RI-O1 | M | **Done** — `/sw.js` (`src/sw.js`, `src/shell.rs`), `/offline`; works on https and localhost only, see open decision 9 |
| C2 | Web App Manifest, icons, installability, `navigator.storage.persist()` | RI-O2 | S | **Done** — manifest, placeholder icons, `static/js/persist.js`; installs only over https, see open decision 9 |
| C3 | Debounced form-state persistence and restore — no more lost in-progress entries | RI-O3 | S | **Done** — `static/js/draft.js`; see Phase 3 notes |
| C4 | `tt-repo-sqlite` for the browser over SQLite-WASM/OPFS | RI-O7 | L | **Done** — new crate `tt-client` (`ClientRepo`, `tt_client::opfs::open`); OPFS needs https (open decision 9), see Phase 3 notes |
| C5 | Service Worker fragment interception → wasm handler dispatch | RI-O8 | M | **Done** — new crate `tt-pages`; `tt_client::pages`, `src/sw.js`, `deploy/build-client.sh`; the team page so far; https only (open decision 9), see Phase 3 notes |
| C6 | Migrate read-only `/hx/*` routes to wasm, one at a time | RI-O9 | L |  |
| C7 | Outbox + sync client in `tt-client` | RI-O10 | L | **Done** — `tt_client::{outbox, sync}`, `POST /api/sync/push` (`src/push.rs`), wire format `tt_core::outbox`; nothing loads it in a page yet (C5), see Phase 3 notes |
| C8 | **Make assignments available offline.** An assignment a scout cannot see when the network drops is worse than no assignment | RI-A5 | M | **Done** — the scouting page's **Your assignments** list, `static/js/agenda.js` (localStorage, works over plain http); see Phase 3 notes |
| C9 | Offline auth tokens (PASETO) layered onto device identity | RI-O12 | M |  |
| C10 | Conflict review screen for the lead scout | RI-O13 | M |  |
| C11 | Repo-trait round-trip tests run against **both** implementations, so server and browser cannot diverge | RS §11 | M | **Done** — `crates/tt-client/tests/round_trip.rs`: every method on both, answers and tables compared; see Phase 3 notes |

### Sync architecture

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| S1 | `upstream` append-only log fed by the FIRST/TBA clients | RI-S2 | M | **Done** — `upstream` table, `tt_upstream::journal`; see Phase 3 notes |
| S2 | `changes` append-only log + `/api/sync/pull` with a lag window. **Not** per-table watermarks — those cannot see deletions and have a commit-ordering race | RI-O15 | M | **Done** — `changes` table + triggers, `GET /api/sync/pull`; see Phase 3 notes |
| S3 | Scoped subscription filtering + a never-replicate allowlist, so other teams' notes never leak | RI-O16 | M | **Done** — `sync::Scope`, `tt_repo_sqlite::replication`; see Phase 3 notes |
| S4 | Compile the FIRST/TBA clients for `wasm32`; client-side conditional fetch with ETags. **Both APIs allow direct browser requests**, so no relay server is needed | RI-S3 | M | **Done** — `tt-upstream` builds for wasm32 (CI and `check.sh`), `TbaClient::remember`; see Phase 3 notes |
| S5 | Bundle import on the Pi: role-gate the push, `ATTACH`, upsert, advance cursor, audit-log | RI-S4 | M | **Done** — `POST /api/sync/bundle`, `tt_repo_sqlite::bundle`, `tt_upstream::project`; see Phase 3 notes |
| S6 | USB tether as the Pi's automatic uplink; pull bundles whenever `usb0` is up | RI-S6 | M |  |
| S7 | Opportunistic client fetch: detect signal, fetch upstream, queue bundle, push on reconnect | RI-S7 | M |  |
| S8 | **SSE fan-out endpoint** with `Last-Event-ID` resume and a polling fallback | RI-S9 | M | **Done** — `GET /api/sync/stream`; see Phase 3 notes |
| S9 | **Push assignment changes over SSE** instead of re-rendering the whole grid on every click | RI-A2 · RS §12.8 | M | **Done** — `grid-live.js`, `assignment-watch.js`; see Phase 3 notes |
| S10 | SQLite snapshot bootstrap (`/api/sync/snapshot`, OPFS import) — ship a file, not a million rows | RI-O17 | M | **Done** — `GET /api/sync/snapshot`, `tt_repo_sqlite::snapshot`, `static/js/snapshot.js`; OPFS needs https (open decision 9), see Phase 3 notes |
| S10b | Keep `users.id, name` in the S10 snapshot, so an offline grid names its scouts (follow-up from C4) | RI-O17 | S | **Done** — `tt_repo_sqlite::snapshot` keeps the users its rows name, by name only; see Phase 3 notes |
| S11 | Schema version handshake + blocking update banner. The mid-event deploy footgun | RI-O18 | S | **Done** — page `<meta>` vs `/health`, `?schema=` on `/api/sync/pull`; see Phase 3 notes |
| S12 | Clients compute and record their clock offset against the server on each sync, so device skew is measurable rather than mysterious | RI §Time Sync | S | **Done** — heartbeat measures it; shown in the Tablets list; see Phase 3 notes |

### Phase 3 notes

**The app is installable, where browsers allow it (C2).** Every page links `static/manifest.webmanifest`: "TealTeam Scouting", standalone, start and scope `/`, with the dark theme colour of the header. Its icons are an SVG, PNGs at 192 and 512, a maskable 512 for Android's crop, and a 180px `apple-touch-icon` for iOS's Home Screen. **The icons are placeholders**: two white Ts on the team teal, drawn from rectangles so they do not depend on a font. `static/icons/icon.svg` and `icon-maskable.svg` are the sources, and the PNGs were rendered from them with `rsvg-convert -w <size> -h <size>`. A real mark replaces those two files and the four PNGs. They reach the binary through P5's `build.rs` like every other static file, and `.webmanifest` is served as `application/manifest+json`.

**Asking to keep the data.** On every signed-in page, `static/js/persist.js` asks `navigator.storage.persist()` once in a browser tab, and once more after the app is installed, when Chrome is far likelier to agree. It never asks again after that, because Firefox asks the person and must not do it on every page. The account page's new **This device** card says what the browser decided: kept, best effort, or not available. It also shows how much space is used out of the quota, which the plan wanted checked before it gets tight.

**iOS (open decision 1, both supported).** Safari never shows a prompt for this. It keeps the data of a site added to the Home Screen, and clears a site's data after 7 days without a visit otherwise. That is harmless within an event and matters between events. iOS installs through Share → Add to Home Screen, using the `apple-touch-icon` and the manifest's `display`, and shows no install prompt of its own. The card says this in plain words. **Not tested on an iPhone or iPad.**

**What blocks most of Phase 3: the event LAN is plain http.** A browser offers installation, `navigator.storage`, service workers (C1), OPFS (C4), and `crypto.subtle` (C9) only in a *secure context*: https, or the device itself. REBUILD_SPEC.md §3 (session cookies) says the event LAN is plain http with no TLS on the Pi. Checked over the DevTools protocol against the real server:

- From `http://127.0.0.1`, Chrome parsed the manifest with no errors and listed **no** installability errors, even with no service worker. `persist()` ran, and headless Chrome answered "best effort".
- From the same server at its LAN address, `http://192.168.68.60`, the same manifest parsed. But Chrome's only installability error was **`not-from-secure-origin`**, and `navigator.storage` did not exist, so the card said "nothing is kept here".

So on the event LAN today, C2 does what it can: an Android "Add to Home screen" shortcut, the iOS Home Screen icon, and an honest card. C1 cannot work there at all. This is **open decision 9**.

**Unsaved answers survive a reload, a crash, or a flat battery (C3).** `static/js/draft.js` writes the scouting form to `localStorage` 400 ms after the last change, and at once when the page is hidden or left. On the next load, a draft that differs from what the server rendered is put back. A note above the form says "Restored your unsaved answers from 10:42 AM", with a button to discard them. The draft keeps the form's `record_id`, so a retry after a save that did land but whose reply was lost is stored once (D7). The server builds the key, `tt-draft:v1:{user}:{event}:{match}:{team}:{form version}` (`tt_templates::draft_key`), so a draft can only return to its own scout, robot, match, and form. The script also checks the match and team before restoring. A confirmed save puts that key on the success message for the script to delete. When the server shows a form it just rejected, the posted answers are newer and they replace the draft. Drafts older than three days are deleted. Storage failures (full, private browsing) are ignored, and the form still works. The format is deliberately simple JSON, for the outbox (C7) to replace. Checked: the page carries the right keys (a Rust test), and the script, in headless Chromium, against a copy of the form. That covered restore, a draft for another match, just-posted answers, debounced saving, clearing, and expiry. Not checked: on a real tablet, and the discard button.

**The upstream log (S1).** Every FIRST or TBA response with new content is appended, as received, to the `upstream` table: `(seq, api, path, etag, body, fetched_at, via)`. A 304 carries nothing new. FIRST sends no ETags, so the log drops a body identical to its path's newest. The sync still writes the events, matches, rankings, and stats tables exactly as before, and the pages read those. The log is the record they were derived from, and the stream to pass on. The clients stay storage-free, because S4 compiles them to wasm32: each takes an optional `Recorder`, and on the server `tt_upstream::journal` drains it into SQLite from a task. A failing log costs a warning, never a sync. `tt-web bulk-load` waits for the log to catch up before exiting.

- **How the later items use it.** S2 and S8 serve `upstream_since(cursor)`. The stream is `sync_state`'s `'upstream'` source, separate from `changes`, as RI's "two logs" requires. S5 appends a pushed bundle's responses with `via = <device>`, then derives the tables with the same tt-core parsers. The one ingest path is then "append to the log, then project". Today the projection is inline in `sync.rs`; S5 is where it moves behind the log.
- **Bounded by pruning.** Each append keeps only the newest `UPSTREAM_KEEP_PER_PATH` (5) bodies per path, in the same transaction. Upstream is last-write-wins, so the newest response per path is the whole current state, and a client whose cursor predates a pruned row loses nothing. `AUTOINCREMENT` means a pruned `seq` is never reused. The size is about five bodies per request the sync makes: a few MB per event, the largest being a playoff event's matches with score breakdowns.
- **Not built:** a path-to-event column for S3's scope filtering. Upstream data is public, so S3 only needs it for bandwidth, and the TBA path already contains the event key.

**The venue stream and the pull (S2).** A `changes` table logs every insert, update, and delete of observations, assignments, and pick list entries. That includes reviews and declines, which are observation updates. **SQLite triggers write it** in the same transaction as the change, so no code path can forget, and a deletion is an ordinary `delete` row with no body. Each row carries a key that is the same on every device: `client_record_id`, or `match_key:team`. Rows written before S2 were logged once by the migration, so the log alone rebuilds the current state.

- **The pull.** `GET /api/sync/pull?changes=<cursor>&upstream=<cursor>` returns both streams, each with its next cursor and a `more` flag. At most 500 changes and 20 upstream bodies come per request. Changes are served only once they are two seconds old: the lag window for a change whose `seq` is taken but not yet committed. The changes cursor moves past rows the viewer may not see, so nobody is sent back for them.
- **Visibility is decided in one place,** `tt_web::sync::visible`. A pick list goes only to its team, since `team_scope` is the owning team. Another team's observation arrives with its notes removed by `tt_core::notes::redact`. Unreadable answers are sent as none, never passed through. **S3's subscription scope** applies there too: see below. Only these three tables have triggers. users, sessions, and devices have none, and the migration says they must never get one.
- **Not built:** compaction. An event writes a few thousand change rows, mostly pick-list reorders, a few MB at most. When it matters, superseded upserts can be pruned per `entity_pk` as S1 prunes per path, but tombstones must stay longer than any client stays offline. Nothing consumes the pull yet; C7's sync client is the first.

**A reload with no server shows a page, not the browser's error (C1).** This is where a secure context allows it: https, or localhost (open decision 9). `static/js/shell.js` registers `/sw.js`, and does nothing at all when `!isSecureContext`. The worker's source is `src/sw.js`; `src/shell.rs` serves it from the root, so its scope is the whole site, and fills in its precache list from the embedded asset table. The worker does four things:

- **On install,** it precaches `/offline` and every static file into `tealteam-shell-<build>`.
- **Pages** always come from the network. Only when that fails does the worker show the cached `/offline` page, at the address that was asked for.
- **Static files** also come from the network first, so a page and its stylesheet are always from the same build, with the cache only as the fallback.
- **Everything else passes through untouched:** POSTs, `/health`, `/api`, and the live regions' fetches. Nothing is written to the cache after install, so no one's page is ever kept.

**The shell is the same for everyone.** `/offline` (`OfflinePage`) renders with an anonymous nav, with no account links at all, and as if storage were fine, so no device's copy says anything about who cached it. It says the server cannot be reached, that a half-typed scouting form is kept on the device (C3), and that it will reload by itself. `static/js/link.js` sees `#offline-shell`, checks `/health` every 5 seconds, and reloads into the real page when the server answers. It does not do this at `/offline` itself, where it would reload forever, a loop the browser test caught.

**Versioned by the build.** `build.rs` hashes every static file, every template, and the worker's source into `BUILD_VERSION`. A binary that changes any of them serves a different `/sw.js`. Browsers look for a changed worker on every navigation, and `/sw.js` is `no-cache`. The new worker installs its own cache, takes over at once (`skipWaiting` and `clients.claim`), and deletes the old one. Because pages and files come from the network first, taking over at once cannot pair a new page with an old stylesheet.

**For S11 (the version handshake).** `/health` now reports `{"storage":…,"build":"<BUILD_VERSION>"}`, and `link.js` already polls it. What S11 still needs:
- Render the page's own build into it, e.g. a `<meta>` from `Nav`, and compare it with `/health`'s on each check. A difference is the "Update required — tap to reload" banner.
- Flush or export the outbox (C7) before that reload.
- Compare the schema version as well as the build. The build changes whenever a template does, which a sync does not care about, so the migration count (`_sqlx_migrations`) is the number the sync protocol should refuse on.

A new worker is already fetched by the next navigation. S11 decides when the page reloads into it.

**Checked in Chromium over the DevTools protocol:** `crates/tt-web/tests/browser/service-worker.mjs` runs 14 checks, and all pass.
- On `127.0.0.1`, the worker registers and precaches 18 files, and the only page among them is `/offline`.
- With the server stopped, reloading `/teams` shows the offline page at `/teams`, styled from the cache, with the chip saying offline. A POST fails rather than being answered from the cache. With the server back, the page reloads itself into Teams.
- `/offline` opened directly stays put.
- A second binary, differing only in a CSS comment, replaced the cache with one holding the new stylesheet.
- From the LAN address over plain http, there is no service worker API, the page works, and the console is empty.

Not checked: Safari, Firefox, or a real tablet.

**The live stream (S8).** `GET /api/sync/stream` pushes S2's two streams as server-sent events, with the same cursors and the same `sync::visible`. So a stream can no more leak another team's notes or pick list than a pull can. Every event's id is both cursors, `"<changes>-<upstream>"`.
- **Resuming.** A browser reconnects on its own with that id as `Last-Event-ID` and carries on from there. A client with saved cursors passes `?changes=&upstream=` instead.
- **Event types.** `change`, `upstream`, and `cursor`: rows went by that this viewer may not see, so the id moves on without them. Later types join on the same channel: assignment pushes (S9), chat (X2). `EventSource` ignores types a client has not registered for.
- **Keeping it open.** A comment line every 15 s keeps phones and proxies from dropping an idle stream.
- **The cap.** At most 64 streams are open at once. Past that the answer is a 503 with `Retry-After`, naming `/api/sync/pull` as the polling fallback.
- **Cost.** Each open stream checks the logs once a second, which is nothing to SQLite at that count. Changes wait out the two-second lag anyway, so push latency is 2-3 s. A shared notifier would cut the queries if the cap ever needs to rise. Nothing in the browser listens yet; C7's sync client is the first.

**A deploy mid-event blocks old pages instead of letting them post (S11).** Every page carries `<meta name="tt-build">`, `tt-schema`, and `tt-form`, which are the versions it was rendered under (`Nav.version`, `shell::page_version`). `/health` reports the same three. `static/js/link.js` already checks `/health` every 30 seconds, and at once when the network comes back, so it compares them.

- **Any difference** is a deploy: the build, the schema, or the season form. The page shows a **blocking banner**, "TealTeam has been updated", over an `inert` page, with one button, **Reload now**. It adds that the scouting form changed too only when the form version differs. A rollback is also a difference, and reloading is right for it too.
- **Before reloading,** the page fires `tt:before-update`. Listeners may add promises to `detail.waitFor`, and the reload waits for them, for up to 5 seconds. `draft.js` (C3) saves the form there at once, without the debounce, so the last words typed before the deploy are kept. They come back after the reload when the form is unchanged. When the form changed, the draft's key names the old form version, so it stays on the device but does not fill the new form, which the banner says.
- **The schema number is the newest embedded migration** (`migrate::latest()`). `PRAGMA user_version` is never set by a migration, so `Repo::schema_version` does not give it. The build alone would be the wrong number for sync, since it changes with any template.

**Sync refuses a client on another schema.** `/api/sync/pull?schema=<n>` and S8's `/api/sync/stream?schema=<n>` answer **409** when `n` is not the server's; the stream checks before it takes one of its 64 slots. The answer carries `"action": "reload"` for an older client, or `"server-behind"` for a client newer than the Pi, which is the lead scout's problem. Every pull also reports the server's `schema`, `build`, and `form`, so a client that did not send `schema` is told anyway. An older client that sends no schema is still served, because nothing sends one yet.

**What the outbox (C7) must do with this:**
- Send `?schema=` on every pull, and on its push, which should refuse the same way.
- Listen for `tt:before-update` and put its flush in `detail.waitFor`, so queued submissions are pushed before the reload. A push refused for schema must **not** drop the queue: keep it, and offer it as a file to hand to the lead scout. The 5-second cap is for drafts; C7 may need a longer one while the push is making progress.
- Treat a 409 with `server-behind` as "stop syncing and tell someone", never "reload".
- Open its `EventSource` with `?schema=` too. After a deploy the stream drops, and the browser reconnects to the same URL on its own. The new server's 409 then stops the `EventSource` for good, where a stream that simply ended would have it reconnect over and over. An `EventSource` cannot read the 409's body, so on an error that leaves it `CLOSED`, the client pulls once with the same `?schema=` to learn which way it is.

Observations already carry their form version, so one saved from an old form lands flagged on the review page, not lost.

**Checked in Chromium over the DevTools protocol** with three builds (`crates/tt-web/tests/browser/update-banner.mjs`, 12 checks, all pass). The builds were as is, with one CSS comment added, and with the season's `version` bumped.
- A scout had the Q5 form open. Words were typed but not yet saved by the debounce when the server was swapped for the second build.
- The page blocked, with the button focused and the page behind it inert. **Reload now** landed on the new build with those words back in the notes box.
- Swapped again for the third build, the banner also said the form had changed. After the reload the new form version was on the page, and the old form's draft was still on the device.
- With the same build again, there was no banner.
- The 409s, and the stamps matching `/health`, are Rust tests.

Not checked: a real tablet, or an iPad's `pagehide`.

**Assignments over the live stream (S9).** The grid no longer re-renders whole for a one-cell change (REBUILD_SPEC.md 12.8). Every cell's id is `match_key:team`, the same as S2's `entity_pk` for an assignment. The page opens the S8 stream from the current log heads (`Repo::log_heads`), so it gets what happens next, not the history.
- **The lead's grid.** On the grid, `grid-live.js` marks the cell named in each `assignment` change. 300 ms after the last one, it fetches the grid once and swaps just those cells and the coverage table, outlining each briefly. The markup still lives only in the page template. A **Save in the match editor** is posted with `fetch`. From the page the 303 leads to, it takes the saved match's cells, the notice, and the next editor, then updates the URL. Anything other than a redirect, such as a refused save and its reasons, is posted again the ordinary way so the page shows it. Without JavaScript nothing changes: plain posts, 303s, and live.js's 30 s polling, which also stays as the fallback when the stream is refused.
- **The scout's page.** It carries who the scout is (user and tablet), the robot on screen if it came from an assignment, and the event's match labels. `assignment-watch.js` says so in words when the lead takes that robot away ("took Q2 · Team 254 off your list; if you are already watching it, finish and save") or gives them one. The two together read as a move. It links to their next assignment and never touches the form.
- **Checked:** a router test that both pages carry the hooks and start from the log heads. Both scripts ran in headless Chromium with a fake `EventSource` and `fetch`: a move, an unrelated change staying quiet, the one-cell swap, the in-place save, and a refused save falling back to a real post. Not checked: against the live server in a browser.


**Tablet clocks (S12).** `device.js` measures its clock against the server's on every heartbeat, NTP-style. The reply carries `server_ms`, the server's time taken to be halfway through the round trip. The next heartbeat reports `offset_ms` (server minus tablet) and `rtt_ms`. The server keeps the latest per device, in `devices.clock_offset_ms` (migration 0005), and ignores a measurement taken over a round trip longer than 10 s. The Tablets list on the assignments page reads it as "clock 3 s behind" or "clock 4 min ahead". Past a minute (`connectivity::CLOCK_TOLERANCE`) it becomes an amber badge saying the tablet's timestamps will be wrong, and to set its clock to automatic. Within a second reads as "in step", since the measurement is only good to half a round trip. Everything is UTC milliseconds, so a tablet set to the wrong zone shows as hours out only if its clock itself is wrong, not merely its display. Nothing corrects timestamps by the offset yet; C7's outbox can, when it stamps offline records.

**Scoped subscriptions and the replication allowlist (S3).** A client subscribes with `?event=` on the pull or the stream, repeated or comma-separated; none is every event. A change for another event is not sent. Of the upstream log, TBA's per-event paths (`/event/<key>/...`) are scoped the same way. Season lists and FIRST's rosters, which name events by FIRST's code, are small and go to everyone. The cursor moves past whatever the scope leaves out. **The team is never the client's to choose:** it is the signed-in viewer's, and `sync::visible` applies the pick-list and notes rules whatever scope was asked for. The grid and the scouting page subscribe to their own event.

**Which tables replicate is decided by name, in `tt_repo_sqlite::replication`.** There are four lists, and every table is in exactly one:
- `REPLICATED`, through `changes`: observations, assignments, pick lists.
- `NEVER_REPLICATED`: users, sessions, devices.
- `FROM_UPSTREAM`, which clients derive from the upstream log.
- `SERVER_ONLY`: the logs themselves, and the point weights for now.

A test fails if a table is in no list or in two, if a listed table does not exist, or if any table outside `REPLICATED` has a trigger writing to `changes`.

**A phone that found signal can hand the Pi what it fetched (S5).** `POST /api/sync/bundle` takes a bundle as its body: a SQLite file with a `meta` table (`format` = `1`, `log` = the id of the client's upstream log) and S1's `upstream` table (`seq, api, path, etag, body, fetched_at`). The format is written down in `tt_repo_sqlite::bundle`. S4 and S7 build bundles; nothing does yet.
- **Only a lead scout may push.** Refusals are status codes with a JSON reason, never the guards' redirect, so a client knows to keep its bundle: 401, 403, 409 for another schema (S11's `?schema=`, as for the pull), 413 past 32 MB, 422 for a file that is not a bundle (with why), and 503.
- **The import.** The file is `ATTACH`ed, and one transaction does the rest. Rows past this log's cursor go through the same append as the Pi's own fetches. A row is skipped when its body is already the newest for its path, and **when it was fetched no later than the newest**, so an old phone cannot roll a ranking back. A time in the future is taken as now. A row that is not a FIRST or TBA JSON response is refused and counted. The cursor (`sync_state`, source `bundle:<log>`) moves to the bundle's newest row, so the same bundle twice imports nothing. A wiped phone starts a new log, which is read from the start.
- **The audit trail.** Every push is a `bundle_imports` row: when, who, which tablet, the seq range, and the counts. That includes empty and refused-row pushes, but not files refused whole. Appended rows carry `via = 'bundle:<id>'`, never the device id, which is the tablet's cookie. The lead scout page's status list says who pushed last, from which tablet, when, and how many responses were new.
- **Then the tables.** `tt_upstream::project` applies the log's newest response for each path the bundle touched, using the Pi's own storing code, split out of the sync as `store_matches` and `store_stats`. So rankings alone combine with the OPRs already logged. Stats wait until the log has both rankings and OPRs. Rows are stamped with the phone's fetch time, and stats with the older of the two. Only TBA's per-event matches, rankings, and OPRs are applied. Season lists and FIRST's rosters stay in the log, which clients still get, and the bulk load (I8) is what brings those. An event the Pi does not have is said, not created.
- **Not done:** the Pi's own sync still writes the tables inline, as well as logging. Only bundles go "append, then project", which S1's notes had hoped for both.
- **Checked:** repo tests (once only, stale, refused rows, cursors per log, files that are not bundles, and the one connection being left usable), projection tests, and a router test of every refusal and a push applied end to end. **`ATTACH` on an in-memory database silently attaches an empty one**, so those tests use a file. Not checked: a real phone's bundle, since none builds one yet.
**A fresh device downloads one file instead of replaying the log (S10).** `GET /api/sync/snapshot?event=` returns a SQLite database (`application/vnd.sqlite3`) of what the signed-in viewer may see of the events asked for. The cursors to pull from next are inside it, in `sync_state` as `server:changes` and `server:upstream`, and in the header `X-Sync-Cursor: <changes>-<upstream>`, S8's event id shape. With `?schema=` not the server's, it is a 409, as for the pull.
- **How it is made** (`tt_repo_sqlite::snapshot`). `VACUUM INTO` a scratch file, as a backup is. That is one read transaction, and SQLite has one writer, so the log heads are exact cursors with no lag window. The copy is then cut by `replication`'s lists. `NEVER_REPLICATED` and `SERVER_ONLY` tables are emptied, except `upstream`, which keeps the newest response per path in scope. Rows for events outside the scope go, from every table with an `event_key`, but `events` and `teams` stay whole. Other teams' pick lists go, and other teams' notes are removed with the pull's rule. A last `VACUUM` means nothing removed survives in a free page: a test looks for a password hash, a session id, and another team's notes in the raw bytes. The file is in rollback-journal mode, since OPFS has no `-wal` beside it. It is this build's whole schema, `_sqlx_migrations` and triggers included, so C4 opens it as is.
- **Shared for a minute.** A snapshot is kept for 60 s (`snapshot::FRESH`) and given to anyone asking with the same team and events. Only one is made at a time, so a room of tablets starting together costs one copy. One a minute old is still right, because its cursors are its own. That is instead of RI's "regenerate on a timer": a timer would make snapshots for scopes nobody asks for.
- **OPFS import** (`static/js/snapshot.js`). `ttSnapshot.bootstrap({ events })` downloads it and writes `tealteam.sqlite3` to OPFS with `createWritable`, which swaps the file in only when complete. The cursors go beside it in `tealteam-sync.json` (`ttSnapshot.local()`). It refuses to replace a copy already there unless given `replace: true`, because that copy may hold what C7's outbox has not pushed. **Nothing loads the script yet**; C4's browser repo is its first user. OPFS exists only in a secure context, so on the event LAN's plain http it refuses with `insecure` (open decision 9).
- **An empty event database is about 250 KB**, mostly one page per table and index.
- **Checked:** repo tests for every cut, the cursors, the journal mode, and the server's own database left untouched. A router test for the headers, the notes rule both ways, the shared copy, the 409, and signed-out. `crates/tt-web/tests/browser/snapshot.mjs`, 9 checks in headless Chromium against the real binary, all pass: download, the OPFS file and cursors, no account in it, an existing copy kept, a schema refusal leaving it alone, `replace`, and plain http refused. Not checked: Safari, whose `createWritable` support is recent, or a real tablet.

**The FIRST and TBA clients compile for the browser (S4).** `tt-upstream` builds for `wasm32-unknown-unknown`, and CI and `check.sh` build it there on every run. That is a separate build from tt-core's, because cargo unifies features within a build, and this crate's chrono `clock` would hide a leak into tt-core. The same `FirstClient` and `TbaClient` run on the Pi over rustls and in a browser over `fetch`. Four things differ on wasm, each behind a `cfg`:
- **Timeouts are per request.** A browser's reqwest client has no timeout of its own.
- **Retry backoff waits on `setTimeout`**, looked up on the global object so it works in a Service Worker as well as a page.
- **The probe asks `navigator.onLine`.** A browser cannot open the raw TCP connection the Pi probes with. A "no" from it is trustworthy and skips the request; a "yes" proves nothing, so it records nothing, and the request finds out.
- **`sync`, `project`, and `journal()` are server-only.** They need tokio and a `Repo`. A browser's recorder is whatever builds its bundle for S5 (S7).

**Conditional fetch works from a browser too.** Checked against the live APIs on 2026-10-01: TBA's preflight answers `access-control-allow-headers: if-none-match, x-tba-auth-key` and exposes `ETag`; FIRST allows `authorization` from any origin. A request carrying its own `If-None-Match` skips the browser's HTTP cache, so TBA's 304 reaches the client, which answers it from the body it holds. A browser's client lives only as long as its page or worker, so **`TbaClient::remember(path, etag, body)` seeds that cache from the client's own upstream log**, newest row per path. A phone that fetched the schedule yesterday spends a 304 on it today. The Pi could seed its cache the same way at boot, from `upstream`; it does not yet. Tests cover a seeded tag answered by a 304, a stale one replaced, and one that is not a valid header ignored. **Not checked: the wasm build running in a real browser.** Nothing loads it until S7 and C4; the wasm code compiles and passes clippy, and the native tests cover the shared logic.

**A scout's assignments survive the network dropping (C8).** Before, the scouting page showed one assignment and a link to the next, which is useless with no server. Now it lists every robot the scout or their tablet still has to watch, in playing order, as **Your assignments**: "Q14 · Red 2", the time on the event's clock, the team and its name, each row opening that robot, the one on screen outlined. `tt_core::assignments::Agenda` gained `upcoming`, so this is still L4's one pure function. Played ones stay under **Still to record**.
- **Kept on the device without https.** The page also carries the list as `data-agenda`, and `static/js/agenda.js` keeps the newest copy in `localStorage` as `tt-agenda:v1`. That works over the event LAN's plain http, which service workers and OPFS do not, so open decision 9 does not block it. One copy per device: the last scout to open Scout there. Signing out deletes it, and a copy over three days old is dropped.
- **Shown on the offline page.** Where a service worker runs (C1), `/offline` has an empty `#offline-agenda` card that agenda.js fills from the copy, so the cached shell itself still names nobody. It says whose list it is, when it was kept, and that the lead may have changed it since. Over plain http there is no offline page; the list on the page already open is what the scout has.
- **Changes pushed while the page is open** (S9) reach agenda.js as `tt:assignment-change` from assignment-watch.js. One that touches the kept list, or gives this scout a robot, marks the copy, and the offline card then says "The lead scout changed your assignments at 10:50 AM, after this list was kept". The copy is not patched, because a pushed change carries no station or time. The next scouting page replaces it.
- **Not built:** opening the form for the next robot with no server. That needs the wasm pages (C5/C6) and the outbox (C7). Until then an offline scout knows which robot to watch, and their typing is kept by C3.
- **Checked:** a core test for `upcoming`, and a router test for the list's order, links, current row, and kept JSON (and an empty one replacing an old copy). `crates/tt-web/tests/browser/agenda.mjs` runs 16 checks in headless Chromium against the binary at 390px, all passing: the list and its copy, a change marking it and an unrelated one not, the server stopped and the offline page showing the same rows, a fresh copy clearing the mark, the copy kept over the LAN address's plain http, and sign-out deleting it. Screenshots of both views were read. Not checked: a real tablet, or Safari.

**An offline grid names its scouts (S10b).** A snapshot now keeps the `users` its rows name: the scouts and the leads in `scout_assignments` and `observations`, no one else. Of each it keeps the id and the name. The email is `#<id>` (the column is unique), the password hash is empty, `team_number` and both timestamps of activity are null, and every role is off, so nobody can sign in to the copy. The device's grid now says "Sam" where C4 said "Scout 7", and an approved observation names its scout. A scout who signed up after the snapshot is still missing, and still reads as "Scout 7". Devices are left out as before ("Device 3"): their names are mostly the UUID, which the snapshot keeps secret. **Checked:** the snapshot test now seeds a user no row names and checks that neither that user nor Sam's email is anywhere in the file, and that Sam's row reads `1 Sam #1 []` with no team and no roles. `tt-client`'s grid test expects the name; the offline-write test uses a scout the file lacks.

**The server and the browser cannot drift apart unnoticed (C11).** `crates/tt-client/tests/round_trip.rs` migrates an empty database and opens the file twice: once with `SqliteRepo`, once as bytes with `ClientRepo`. Each test makes the same calls on both, in six areas: accounts, sessions and devices; the competition graph; assignments; observations and review; pick lists; and the upstream log. Every answer must match, errors included. Each test then reads everything back from both sides: every read over every key it wrote, the change log, and every table row for row. Comparing tables catches a write that no read shows, such as `record_login`'s `last_login_at`. The only difference allowed is foreign keys, which are off on the device. One test pins that: the server refuses a row naming a user it lacks, and the device takes it. Another test fails when `LocalRepo` gains a method that this file does not call on both. C4's snapshot tests stay, and share their helpers through `tests/common`. **Found and fixed:** `WeightOverrides` held a `HashMap`, so two equal sets of overrides could print in different orders. It is now a `BTreeMap`. **Checked:** breaking one statement on the device (`record_login`, then `rename_device`) failed the tests each time, and so did dropping a method's call. **Not compared:** the pick-list document bytes (`pick_list_docs`) after a document is made from existing rows. Each side mints its own random yrs client id, so the bytes differ even when the lists are the same. The tests compare documents built from merged updates, which match.

**The browser has the whole `Repo` (C4).** A new crate, `tt-client`, which REBUILD_SPEC.md §9 names as the browser side. `ClientRepo` implements `LocalRepo` with every method, over rusqlite. On wasm32, rusqlite runs on sqlite-wasm-rs, which compiles SQLite's C with clang. Each module's SQL is `tt_repo_sqlite`'s, statement for statement, in modules of the same names. rusqlite is pinned at 0.39 because it shares sqlx's `libsqlite3-sys` 0.37, and two versions of that cannot link into one workspace.
- **It opens a snapshot (S10) as is.** `tt_client::opfs::open()` reads `tealteam.sqlite3` from OPFS, where `snapshot.js` writes it. There is no schema to create and no migration to run. `ClientRepo::schema()` reads `_sqlx_migrations`, for the S11 comparison C7 will make.
- **The database is held in memory and written back whole.** `sqlite3_deserialize` loads it. After any method that changed something, `createWritable` swaps the whole file in, and a read writes nothing. SQLite's own OPFS file systems need a synchronous access handle, which only a dedicated worker has, and C5's handlers run in the service worker. One context must own the file: two copies writing back means the last one wins. A write that fails to save is returned as that method's error, and the next change saves both.
- **Foreign keys are off on the device.** A snapshot carries no `users` or `devices`, so a scout's own offline observation names a `scouter_id` the file has no row for. With keys on, every offline write would be refused. The server checks the keys when the row arrives (C7).
- **The assignment grid reads with ids in place of names.** The same missing rows mean a snapshot's assignments name scouts and tablets it has no names for. The server's code would panic reading one; the device shows "Scout 7" and "Device 3". **Follow-up for S10/C6:** if offline grids should show names, the snapshot could keep `users.id, name` with no email or hash.
- **Checked:** `crates/tt-client/tests/snapshot.rs`, 8 tests. A Pi is seeded through the server's own `Repo` and cut for one team and event. Both repos then open that snapshot, and the test checks that every read answers the same. So do the writes both sides can make, with the trigger-written change log equal too. It also covers: an offline observation and assignment naming a user the file lacks; a pick list built from rows, then merged; a save after every change and none after a read, with a failed save retried; junk refused; and a WAL-mode file opened. In headless Chromium, with a throwaway harness (not committed), the wasm build ran from a page and from a service worker. In both it opened a server snapshot from OPFS, read it, recorded an observation, saved, and read the observation back after reopening. Over the LAN address's plain http it refused with the decision-9 message. The wasm is about 1.7 MB in release before `wasm-opt`. Not checked: Safari, or a real tablet. A committed browser test belongs with C5, which is the first thing to load the wasm.

**A scout's offline saves reach the Pi, and count as saved only when they come back (C7).** Three parts:
- **The outbox** (`tt_client::outbox`). `ClientRepo::queue_observation` records the observation in the device's tables and queues what the Pi needs (`tt_core::outbox::QueuedObservation`: record id, match, team, answers, form version, time watched) in an `outbox` table of the device's own, created on first use; a snapshot never has one. The file is written whole, so a save holds both or neither. A duplicate record id is not queued twice.
- **The push** (`POST /api/sync/push?schema=`, `crates/tt-web/src/push.rs`). JSON, up to 100 at a time. Each entry gets the form post's rules: a UUID record id, a match on the schedule, a team in that match, and answers that fit the form when it is the Pi's form version (another version is kept as is, and flagged on the review page as S11 says). The signed-in scout is the scouter, the tablet's cookie is the device, and the scout's team is the submitting team. `observed_at` is corrected by the tablet's measured clock offset (S12) and never later than now. Each entry gets a receipt: recorded, or refused with a sentence. The whole push is refused only by status, never a redirect: 401, 409 (schema, as for the pull), 413, 422, 503.
- **The sync client** (`tt_client::sync::SyncClient`). One `sync` pushes the outbox and then pulls until caught up, from the cursors the snapshot left in `sync_state`, with `?schema=` and the subscribed events on every request. Pulled changes are applied by the log's own keys; the Pi's row wins, including over a local row that breaks a unique rule with it. **An entry leaves the outbox only when its observation comes back through the pull**, the plan's step 9; a receipt is not enough, so a lost reply just means a harmless resend. A refused entry stays with its reason, is not sent again, and its observation is taken off the device's tables. `export_outbox` writes the whole queue as a JSON file that is itself a valid push body; `discard_refused` drops one once dealt with. A 409 stops the sync as `Stop::Reload` or `Stop::ServerBehind`, a lost session (a redirect, or HTML where JSON should be) as `SignedOut`, no answer or a 5xx as `Offline`; none touches the queue. The network is a `Transport` trait: `Fetch` on wasm (same-origin, `redirect: "manual"`), the real router in a test.
- **Not built, for C5/C6 to wire up:** nothing loads the wasm in a page yet, so nothing calls `sync`. Whatever owns the database should call it on load, when the network returns, and on each event of an `EventSource` opened at `SyncClient::stream_url` (which carries `?schema=`). On `tt:before-update` it should put a `sync` in `detail.waitFor`, and on `Stop::Reload` with entries still queued, offer `export_outbox` as a download before reloading. The pages do not yet show a pending or refused state; C10 is where the lead sees refusals. Pulled upstream responses go into the device's log but are not projected into its matches and rankings; `tt_upstream::project` is server-only. Pushing someone else's exported file records it as whoever pushes, so a lead uploading a scout's file is not built either.
- **Checked:** `crates/tt-client/tests/sync.rs`, 4 tests against a scripted Pi: the echo rule with a resend, a refusal kept and exported and not resent, every stop leaving the queue alone, and a pull applying observations, assignments, and pick list rows (with a clash and deletions), skipping an unknown entity, and appending upstream. `push.rs` runs the real client against the real router: a scout signs up, the tablet downloads its snapshot, two observations are queued (one of a robot not in the match), one sync records one and refuses the other with its reason, and after the lag a second sync clears the first and stores nothing twice; then 401, 409, 422, 413, and answers that do not fit the form. The wasm build compiles and passes clippy. Not checked: in a browser.

**With no server, the device makes the team page itself (C5).** When a page cannot be fetched, the service worker now asks `tt-client`'s wasm module for it before falling back to the offline shell. The module runs the page's own code over the device's snapshot in OPFS. The offline shell (C1) is still what every other page gets, and what every page gets on a device with no snapshot yet.
- **The same function on both sides.** A new wasm-clean crate, `tt-pages`, holds pages built from a `LocalRepo`. tt-web's handler calls it over the Pi's database, and `tt_client::pages::render` calls it over the device's copy. So far it holds the team page (`/teams`, the plan's first route) and the event choice it needs (`EventContext`, `resolve`). Moving a page there is C6. A generic `LocalRepo` function still gives axum a `Send` future, because the server's repo implements `Repo`.
- **Pages, not fragments.** U8 dropped Unpoly, so there are no `/hx/*` routes to intercept. Only navigations go to the wasm. A live region's fetch still passes straight through and is marked stale when it fails: what it shows came from the server, and the device's copy is no newer until C7 keeps it current.
- **A page made on the device says so.** `Nav::from_device` puts a notice at the top: "The server can't be reached, so this page was made on this device from its own copy of the data, which may be behind." Like the shell, it has no account links. `link.js` treats it like the shell: it checks `/health` at once and reloads into the server's page when the server answers.
- **The device cannot know who is holding it.** The session is the server's to check, and offline sign-in is C9. So a device page shows no one's notes and offers every event in the copy. One difference is accepted: a snapshot keeps no roster outside its events (S10), so the team page made on the device has no "Other events" card. **Follow-up for S10/C9:** if the snapshot recorded the team it was cut for, device pages could show that team's notes and its events.
- **How the module gets built.** `deploy/build-client.sh` builds `tt-client` for wasm32 and runs wasm-bindgen (`--target no-modules`, for `importScripts`) into `crates/tt-web/static/client/`, which is gitignored. The next `cargo build -p tt-web` embeds it like any static file, so P5's one-binary deploy holds, `BUILD_VERSION` changes with it, and the worker precaches it. `/sw.js` imports it only when the binary has it, so a binary built without the script works exactly as before. wasm-bindgen's CLI must match `Cargo.lock` exactly (0.2.127 today), and the script says so. The release module is 2.8 MB (0.9 MB gzipped), with no `wasm-opt` run on it. The worker compiles it once per worker start; the first page took about 120 ms in headless Chromium, debug build. `check.sh` and CI still build `tt-client` for wasm32 but do not bundle it.
- **Checked:** `crates/tt-client/tests/pages.rs`. A Pi is seeded through the server's `Repo` and cut for team 10101. The team page from the device's snapshot must equal the Pi's byte for byte, apart from that "Other events" card, for a known team, a team at the event with no data, an unknown team, a non-number, and no team. A second test checks which addresses the device answers and which it leaves to the shell, and that its page has no account and no one's notes. `crates/tt-web/tests/browser/device-pages.mjs` runs 17 checks in headless Chromium against the binary, all passing. They cover: the module precached; the shell when there is no snapshot; the snapshot taken; the server stopped and the team page made on the device, styled and saying so, with the chip offline and no console errors; the page's lookup form going to another team; an unknown team; `/lead-scout` getting the shell; the page reloading into the server's when it returns; and the LAN address's plain http unchanged. C1's `service-worker.mjs` still passes with the module built in. Not checked: Safari, a real tablet, or a snapshot of a real event's size.

---

## Phase 4 — Analysis and communication

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U21 | **Graph view**: uPlot + tap-to-toggle metric chips + team chips. Tap, not drag — drag is a desktop metaphor | RI-U5 | L | **Done** — `/graph`, a **Graph** tab for everyone signed in; `tt_core::graph`, `static/js/graph.js`; see notes |
| U22 | Notes panel as a separate, filterable, timestamped view | RI-U6 | M | **Done** — `/notes`, a **Notes** tab for everyone signed in; filters in `tt_core::notes::Filter` |
| L13 | Rotation fairness — track matches scouted per person and suggest rotation, instead of making the lead scout remember | RI-A4 | M | **Done** — `tt_core::assignments::{distribute, rotation}`; see notes |
| L14 | `yrs`-backed collaborative pick list. The one place in this app where a CRDT genuinely earns its keep — two leads reordering currently clobber each other silently | RI-O14 · RS §5.8 | M || **Done** — `tt_core::picklist::PickDoc`, `/api/pick-list/doc`, `pick-live.js`; see notes |
| X1 | `messages` table, `POST /api/messages`, history endpoint with cursor paging | RI-M1 | M |  |
| X2 | SSE message stream sharing the S8 event channel | RI-M2 | S |  |
| X3 | Side panel (desktop) + full-screen view (mobile) with unread badges | RI-M3 | M |  |
| X4 | Offline outbox integration and pending-message rendering | RI-M4 | S |  |
| X5 | Hybrid logical clock ordering; dual-timestamp display for delayed messages | RI-M5 | M |  |
| X6 | `#team` / `#match` autocomplete and context chips — the reason to build chat in-app rather than adopt Matrix | RI-M6 | M |  |
| X7 | **Moderation: mentor log view, retract-not-delete, rate limiting.** Non-negotiable; the users are minors | RI-M7 | M |  |
| S13 | QR transfer: Rust encoder, browser scanner with `BarcodeDetector` + zxing-wasm fallback | RI-N6, RI-S12 | L |  |

### Phase 4 notes

**The graph view (U21).** `/graph` draws each chosen team's scouted matches as lines, one per metric, with uPlot 1.6.32 vendored in `static/vendor/uplot/` (MIT, its licence beside it). Teams and metrics are chips: tap one to put it on the chart or take it off. There is no drag; RI's own reasoning was that drag is dead on a phone.
- **What a line is.** Across is each team's own matches in order (1st, 2nd, 3rd scouted), not the event's match numbers, so two teams' trends line up and one team's events run into each other. Two scouts on one robot in one match make one point, their average. Only approved observations on the current form version count, as on the profile. The numbers are shared, as on the profile, and notes are not here (U22's view).
- **The metrics** come from the season file (`tt_core::graph::metrics`): the scouting score with the lead's weights, every counter, every yes/no as 1 or 0, and every choice whose options carry points, as those points. Where a robot started has no points, so it is not offered, and neither is free text. OPR, DPR, and CCWM are offered once anything has been synced from TBA. They are one number per event, so their line is flat across that event.
- **The limits.** 8 teams, the validated categorical palette's eight colours (checked for colour-blind separation and 3:1 on the card's `--gray-900`), and 3 metrics, drawn solid, dashed, and dotted. A chip turned on takes the first free colour, and turning another off never repaints it. All metrics share one y axis, so the chip legend says "on one scale". A URL asking for more is cut, and the page says how many it left off. A tap past the limit is refused with a status line.
- **The chips are the legend.** A team chip's dot is its colour, and a metric chip's line is its style. uPlot's own legend is off. A tap on the chart snaps to a match and fills a readout table below: each team's match label and value per metric, since the nth match is a different match for each team.
- **Before anyone chooses**, the three teams with the most scouting points at the event are drawn, with the score. After that (`chosen=1` in the URL), taking every chip off draws nothing rather than bringing the defaults back.
- **Multi-event analysis (the half of U2 left for here).** An **Every event this season** chip (`span=season`) draws the selected event's teams across every event this server has in the same year, oldest first. Points are then labelled with the event code, like `MABIL Q14`. Events nobody scouted are skipped without reading their schedules. That one chip is a navigation, since other events' numbers are not on the page. Every other tap redraws in place.
- **It is a URL.** The chips are checkboxes in a GET form. `graph.js` keeps the address current with `replaceState`, teams and metrics in colour order, so a bookmark reopens the same chart in the same colours. Without the script, a Show button submits the form, and the numbers are tables under **The numbers**. The script rebuilds those tables as chips change. The page also says where its numbers come from and how old TBA's are.
- **The tab bar now has eight tabs** for an admin who is also a lead and a coach. At 360 px that is 45 px each, still just over 44. The team profile links to its team's graph.
- **Checked:** core tests for the metric list, values (weights applied, a missing answer is not zero), the order and averaging of a line, and the limits. Router tests for the defaults, the source line, the URL's choices, both limits, an empty chart, and two events. `crates/tt-web/tests/browser/graph.mjs` runs 14 checks in headless Chromium at 360×800 with touch, all passing: the first draw, tapping a metric and a team on and off, colours kept, a freed colour reused, the limit, a tap on the chart filling the readout, no sideways scroll, and the URL reopening the same chart. Screenshots were read too, and they found two bugs. A tap left a selection box on screen, and uPlot's tap markers took the points' dark fill and blotted out the lines. Not checked: a real phone, or Safari.

**Rotation fairness (L13).** A scout's load is every match they are assigned at the event, played or to come, whether or not they recorded it, since that is the time they were asked to give. Two constants in `tt_core::assignments` set the rules: `LONGEST_RUN` = 6 matches in a row, about 45 minutes of qualifications, and `UNEVEN` = 3 matches between two scouts.
- **Auto-distribute keeps the rota fair by itself.** It now takes the whole schedule, so played matches count as history, and fills the next *n* unplayed ones. Each open robot goes to whoever has had the fewest matches so far, ties in pool order. Nobody is handed a match that would put them past six in a row while somebody else is free. A latecomer catches up without going an hour without a break. With no more people than robots, nobody can rest, and the robots are handed out anyway. Starting from nothing, it reads like the old round-robin: those who sat one out start the next.
- **The grid suggests swaps for hand-made assignments.** A Rotation list in the live coverage area gives at most one suggestion per scout. The first kind is a run past six, broken at the seventh match by the lightest scout who is free and would not pass six themselves. The second is a scout with three or more matches beyond the lightest, who takes the heavy scout's first upcoming robot. Suggestions are worked out as though each earlier one were taken, so two never lean on the same person in one match. Each has a button that posts that one robot to the ordinary match save, which `grid-live.js` performs in place. When nobody is free, the list says so and links to the match.
- **Who can take over:** scouts online now, and scouts with a match still to come, so nobody who went home is suggested. **Tablets are not rotated.** A tablet does not tire, and who will be holding it is not known in advance.
- **A fix found on the way:** the match save checked "two robots in one match" only among the robots the post named. A post naming one robot, which is what a suggestion sends, could put someone on a second robot. It now counts the robots the post leaves alone.
- **Checked:** core tests for the history, the limit, a latecomer, a past run, nobody free, the margin, and two suggestions sharing a reliever. Router tests for the suggestion's words and its one-tap move, and for the doubling refusal. Not checked: in a browser.

**The notes view (U22).** `/notes` lists every note the viewer's team wrote on approved observations at the selected event, in one place. A **Notes** tab is in the nav for everyone signed in. Each note shows the robot (tap it to see only that team's notes), the match, the scout, and when it was recorded: "Sat 2:14 PM CDT · 12 minutes ago", on the event's clock and calendar (`tt_core::timezone::day_and_time`), since an event runs several days. The list is newest first, or in match order. It narrows by team, by scout, and by words, with every word having to appear, ignoring case, so "defen" finds "Defended". The filters are a GET form, and the selects apply as they change, so a narrowed view is a URL. The team profile's Notes card links to its team's. Which notes show is still U13's rule and nothing else: other teams' notes are neither listed nor counted, and a viewer with no team is told why there are none. Pending observations are not listed, the same as on the profile, but the page says how many with notes are waiting for review.
- **Times are the tablet's.** A note's time is `observed_at`, which the tablet stamps, so a tablet with a wrong clock (S12 flags it) misorders its notes. The server's `created_at` is used only when `observed_at` is missing.
- **Notes from older forms show** when their text sits under a key the current form still has as a text field. Anything else is left out. (The profile reads only the current form version.)
- **The tab bar now has seven tabs** for an admin who is also a lead and a coach. At 360 px that is about 51 px each, and the longer labels take two lines. Still not under 44 px.
- **Checked:** router tests for the order, each filter, the waiting count, another team's view, a teamless view, and the profile link. Not checked: in a browser.

**The pick list is a CRDT (L14).** Each team's list at an event is a yrs document, `tt_core::picklist::PickDoc`, stored whole in `pick_list_docs` (migration 0006). The rows in `pick_list_entries` are what it reads as, rewritten in the same transaction, and still replicate through `changes`.
- **How the list is held.** Each team is a map of its own under its number: record id, tag, crossed, and an order key. The order is by key, then by team number. Keys are base-36 fractions, so there is always room between two. A move gives one team a key between its new neighbours and touches nobody else. Two moves of different teams both land, and so do a move and a cross of the same team. Two keys chosen at once for the same gap get two digits from each copy's client id, so they do not tie.
- **What cannot both land is settled the same way everywhere.** Two moves of one team: one wins. Two adds of one team: one row, with one of the record ids, and `merge_pick_list` replaces the row when the id changes. A removal beats a change to that team made without seeing it.
- **A change is a merge.** `POST /api/pick-list` applies the edit to the stored document and merges the update it makes (`Repo::merge_pick_list`). U20's compare-and-swap and its three retries are gone: if another change lands in between, both stand. A list stored before 0006 gets its document from its rows the first time it is read, made once and stored.
- **A copy kept on a device.** `GET /api/pick-list/doc?event=` returns the team's list as one yrs v1 update. `POST` the same path with an update in the body merges it and returns the whole list, so a tablet that edited offline sends what it did and catches up in one round trip. Sending it twice changes nothing. A body that is not an update gets a 400. It is the viewer's own team's list only, for leads and coaches, as with the page. A merged update is not checked against the roster, since the copy that made it checked against what it had. Nothing in the browser keeps a copy yet; that waits on Phase 3's wasm client (decision 3). `PickDoc` builds for wasm32.
- **Live without a reload.** `pick-live.js` watches the S8 stream. When someone else changes the list, it fetches the page once and swaps the list, the "Not on the list" teams, and the add box's suggestions. It waits while a field in the list has focus, and open panels stay open. The lead's own changes still post and come back as a fresh page.
- **Checked:** core tests that two copies merge to one list for moves, crosses, tags, two moves into one gap, one team moved two ways, a double add, a removal against a cross, and an offline batch. A test that a hundred keys squeezed into one gap all fit. Repo tests for merging, idempotence, a garbled update, a pre-0006 list, and a changed record id. Router tests for the exchange, team scoping, and the page's hooks. In headless Chromium against the real binary: with lead A's list open, lead B's move and cross appeared on A's page without a reload. A's open panel stayed open, nothing moved while A was typing a place, and the list redrew when A left the field. Not checked: a real tablet.

---

## Cross-cutting

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| Q1 | `tt-core` unit tests: scoring, mode aggregation, match-status, connectivity classification, match-number normalization, TBA fallback extraction. **Every one of these had a bug** | RS §11 | M | **Done** — gaps filled in `tt-core`; two bugs fixed, see notes |
| Q2 | Deserialization tests against **recorded** FIRST/TBA payloads, including at least one from a prior season | RS §11 | M | **Done** — `tt-upstream/tests/recorded.rs` over `tests/fixtures/`; six fixes, see notes |
| Q3 | Load test before the season: 30 simulated clients, two hours, p95 latency and SSE stability — with the cable pulled, the power killed, and a client's storage filled, deliberately | RI §Load Testing · RS §11 | M | **Done** — `crates/tt-load`, `tests/browser/storage-full.mjs`, [LOAD_TEST.md](LOAD_TEST.md); one bug fixed; Pi untested |
| Q3b | Run SQLite with `synchronous=FULL`, so a power cut cannot lose a save the scout was told was saved (found in Q3) | Q3 notes | S | **Done** — `SqliteRepo::connect`; about 2 ms a save, [LOAD_TEST.md](LOAD_TEST.md#synchronousfull-q3b); Pi untested |
| Q4 | Backups: timed dump to the SSD (10-minute interval, 24-hour retention), USB copy between match blocks, and **one deliberate restore test** before you need it | RI §Backups | M | **Done** — `tt_repo_sqlite::backup`, `tt-web backup` / `check-backup`, [PI_STORAGE.md](PI_STORAGE.md#backups-q4); USB and Pi steps untested |
| Q5 | Store everything in UTC; render in the event's IANA zone per `TIMEZONE_HANDLING.md` | RI §Time Sync | S | **Done** — `tt_core::timezone`; see notes |

### Cross-cutting notes

**Q1: what was already covered, and what was added.** Most of the six areas had tests from when they were written. Match status (`matches::classify`) had every boundary. Connectivity had the three-state order and the staleness edge. The TBA fallbacks had modern, legacy, and empty rankings. Scoring had each field kind and saturation. Added: a tied tally keeps form order; averages keep their fraction; a box never ticked is "0 of n", not unanswered; a team seen only on an old form has no score, not zero, and a breakdown can take an average negative; the success window's exact edge against a failed probe; half-way rounding in the fallbacks. **Two of the new tests found bugs, fixed here:**

- **Every double-elimination playoff match was labelled `SF1`.** TBA keys them `sf1m1` to `sf13m1`: the set is the match, and `match_number` is always 1. The label used `match_number`, so the scouting page, the assignment grid, and the coach panel showed thirteen `SF1`s. The sync test asserted `sf2m1` read `SF1`. `CompLevel::label` now takes the set: `SF5`, `SF5-2` for a replay or a pre-2023 best-of-three, and finals by match, `F1` to `F3`. Storage and ordering were already right (D3 keys on `tba_key`); only the label was wrong.
- **One `null` failed a whole event's sync.** TBA sends `"actual_time": null` for every unplayed match, and serde's `default` covers a missing field, not a null one. Every non-`Option` field in `tt_core::upstream` now reads `null` as missing, through one `or_default` helper (`set_number` stays 1, a score stays TBA's `-1`).

**Left for Q2:** the rankings `null` and `qf`/`ef`, both done in Q2 below.

**Q2: recorded payloads.** `crates/tt-upstream/tests/fixtures/` holds real FIRST and TBA responses, recorded with the shop's keys on 2026-09-30. Only bodies were kept: no headers and no keys. The match lists keep Q1-Q3 and every playoff match. The events: 2026 Magnolia (10101's own), 2019 Bayou for TBA's prior season, and 2025 Bayou for FIRST's, since FIRST's 2019 events endpoint returned a 500. Also 2026 Arizona League before it happened, and the 2026 Milstein division. `tests/recorded.rs` serves them to the real clients and syncs them into SQLite. Running them turned up six faults, all fixed:

- **Championship division rosters were cut at 65.** FIRST pages its teams endpoint. `event_teams` read page 1 only, so Milstein's roster lost 9 of 74. It now follows `pageTotal`.
- **Auto OPR changed from sync to sync.** 2026 has `autoTowerPoints` and `totalAutoPoints`, and `phase_opr` took the first points component a `HashMap` handed it. It now prefers points, then "total", then name order. For 10101 at Magnolia that is 2.50 rather than 0.0.
- **Average match points were whatever sat in `sort_orders[1]`.** In 2019 that column is Cargo. `tt_core::upstream::Rankings` keeps `sort_order_info` and reads the column named "Avg Match". A season without one stores none. Without names, it falls back to the position.
- **`qf` and `ef` were dropped.** `CompLevel` has `QuarterFinal` and `EighthFinal`, labelled `QF4-3` like a best-of-three semifinal, and matches order qm, ef, qf, sf, f in SQL as in Rust.
- **A `null` body failed the fetch.** Every TBA event endpoint now reads `null` as empty. No event returned it on the day, so that test serves `null` by hand.
- **FIRST's event type was always empty.** FIRST sends `type`, and `rename_all` looked for `eventType`.

**Found, not fixed:**

- **FIRST and TBA name divisions differently.** FIRST calls Milstein `MILSTEIN`, TBA `2026mil`, so `FirstEvent::tba_key()` makes `2026milstein`, and TBA sync at a Championship would 404. Fixed as I15, see the Phase 2 notes.
- **FIRST's `timezone` is a Windows zone name** (`"Central Standard Time"`), stored as is. Mapped to IANA in Q5.

**Q5: the event's clock.** Timestamps were already stored as UTC `DateTime<Utc>`, and every page but one showed times relative to now ("in 12 min", "5 minutes ago"), which need no zone. What Q5 changed:

- **Zones are IANA.** `tt_core::timezone::event_zone` maps FIRST's Windows names at sync, reading the country and state first. FIRST's 2026 list gave Perth, Sanya, and Trabzon "Eastern Standard Time", Arizona plain "Mountain" (it keeps no daylight time), and Torreón "Mountain" (it is on central time). The tests hold every pairing FIRST sent for 2026. A name it cannot place is stored as none and reported as a sync problem. An event stored earlier with a Windows name reads as having no zone until its next sync replaces it; no migration. The zone data is compiled in (`chrono-tz`), so it works on a Pi without `/usr/share/zoneinfo` and in wasm.
- **"Today" is the event's.** The event picker's default, the amber stale badge (U14), and the drive coach's badge use `Event::is_running(now)` on the event's own calendar. A US event's last evening, past midnight UTC, is still its last day, and the picker no longer switches to the next event during the finals. The sync loop's live check uses the earliest date anywhere (UTC minus 12 hours), so a US event's finals are still fetched every two minutes. An event ahead of UTC is covered by the one-day lookahead.
- **The drive coach shows the time on the event's clock**: "8:20 AM EDT · in 20 min". It is the one page where a clock time helps, since the coach compares it to the field's schedule. Without a zone it reads "12:20 UTC", never the Pi's local time.


**Q4: backups.** Three layers, all in [PI_STORAGE.md](PI_STORAGE.md#backups-q4).

- **Every 10 minutes, onto the SSD.** While it serves, the server snapshots the database into `BACKUP_DIR`: `/srv/tealteam/backups` on the Pi, and a `backups` folder beside the database when unset. It keeps 24 hours. A folder named in `BACKUP_DIR` is never created, so with the SSD missing a snapshot fails with a warning rather than landing on the SD card. The startup line says where the backups go, and warns if that is the SD card.
- **Between match blocks, one command:** `tt-web backup /media/…/STICK`. It works while the server runs.
- **The restore test:** `tt-web check-backup [file]` restores a backup (by default the newest timed one) into a fresh database in a temporary folder. It checks integrity, applies this build's migrations, and prints row counts. `tt-web backup` runs that same check on the copy it just made, so every USB copy has been restored once before anyone relies on it.

**Snapshots are `VACUUM INTO`, never a file copy.** A copy of `tealteam.db` misses whatever is still in `-wal`. Each snapshot uses a connection of its own that only reads, so the single writer carries on. Snapshots are written as `.partial` and renamed when complete. Pruning goes by the time in the file name, and never leaves fewer than six, so a Pi booted without its RTC (P1) and years out cannot prune away its last good copies.

**Off site is open decision 6, left as a setting.** `BACKUP_COPY_TO` is where a bare `tt-web backup` copies to. Unset, the command asks for a folder and names the decision. Whose laptop, and who checks it ran, is still the team's call.

Tested:

- A unit test restores a snapshot into a fresh database and reads the users back through `Repo`. One of them had been written only to the `-wal` file; its address was checked not to be in `tealteam.db`. The snapshot was taken with the writer's transaction open, did not wait for it, and did not include its uncommitted row.
- Pruning: a day kept, non-snapshots untouched, and six survive a clock set to 2030.
- By hand, with the real binary running against a copy of a test database: `tt-web backup` into a folder printed the same counts as the database (7 observations, 5 picks, 12 assignments), `check-backup` agreed, and the first timed snapshot landed 10 minutes after start.

Not tested: a real USB stick or anything on the Pi.

**Q3: the load test.** `tt-load run --server target/release/tt-web` runs the whole test on one machine. It starts the server on a fresh database, seeds an event, and runs 30 scouts for two hours. Every 15 minutes it causes a fault, alternating between the two kinds. The cable is pulled for 45 seconds: a relay stops passing bytes, so in-flight saves arrive late. The power is cut for 20 seconds: SIGKILL, with the cable also out, and a restart on the same database. `tt-load run --url` runs the same scouts against the Pi, with the faults done by hand. [LOAD_TEST.md](LOAD_TEST.md) covers both.

A scout's cycle is: open the scouting page, save, load the page the save redirects to, and pull. The redirect reopens the stream from `Last-Event-ID`, as a browser would. A failed save is retried with the same record id. This runs at about a hundred times event load. The report gives steady and overall p95 per request type, and every stream drop with its reason. It has seven verdicts. The two that matter most: every save a scout was told was saved is on the server exactly once, and every save reached every scout's stream exactly once.

**Results, on a desktop rather than the Pi.** Two hours with seven faults: 14,228 saves, all on the server once, 118 of them retried. Every save reached all 30 streams, with a p95 of 3.0 s: the two-second lag plus the poll. All 210 stream drops and every failed request fell inside a fault. The slowest page was the assignment grid, at 85 ms p95; everything else was 3 ms or under. The timed backups did not run, because the tool had not created their folder and the server never does (Q4). A 25-minute rerun with backups on also passed every verdict, and a backup landed mid-load. Full numbers are in [LOAD_TEST.md](LOAD_TEST.md#results). The Pi is the run that matters.

**A full phone (`storage-full.mjs`, 12 checks, all pass).** Chromium with localStorage full and the origin's quota turned down to 4 MB and filled. The scouting page works and its draft fails quietly. The save reaches the server. The offline shell does not install, and nothing breaks. With space freed, the shell installs.

**One bug, fixed: a full phone could never download its snapshot (S10).** `getFileHandle(…, { create: true })` creates an empty `tealteam.sqlite3` before the write. When the write failed for lack of space, the empty file stayed. Every later `bootstrap()` then answered "exists" and the device never got its copy. An empty file no longer counts as a copy, and running out of space is refused with reason `full`, not a bare `QuotaExceededError`.

**Found, not fixed:**

- **A power cut can lose a save the scout was told was made.** Fixed in Q3b, below. The database runs WAL with `synchronous=NORMAL`. SQLite documents that a commit under that setting can roll back on power loss, though not when the process is killed, so this test cannot catch it. The setting was chosen to spare SD and USB storage an fsync per commit, but the database now lives on the SSD (P3). At an event that is about one commit a minute. `synchronous=FULL` is a one-line change in `SqliteRepo::connect`. Measure it on the Pi with `tt-load` before deciding.
- **A full phone has no offline shell and is not told.** The site works online, and the account page shows how full the device is, but a reload with no server shows the browser's error page.

**Q3b: `synchronous=FULL`.** `SqliteRepo::connect` now asks for `FULL`, and the WAL test checks for it. Each commit is flushed to the disk before the scout is told "saved". On the desktop's NVMe a commit alone went from 0.01 ms to 0.56 ms. Under `tt-load` (10 minutes, 30 scouts, no faults) a save's p95 went from 0 to 2 ms, and so did the heartbeat's, since it writes too. Both runs passed every verdict. The plan said to measure on the Pi before deciding; two milliseconds against a 500 ms bar did not need the Pi to decide. Read the save row when the Pi run happens. A real pull of the plug on the Pi is still untested.

---

## Open decisions

These need a human, and several block Phase 2 or 3.

1. **Devices.** Team tablets or personal phones? Personal phones puts iOS in scope, which affects `BarcodeDetector` (S13), `navigator.vibrate`, and storage eviction (C2).
2. **Season schema ownership.** Who writes `seasons/YYYY.json` each January (D4), and by what date relative to Kickoff?
3. **Wasm scope.** Every route browser-side, or just enough for a scout to enter data and search team info offline? **The second is far cheaper and probably sufficient** — C6 lets you stop at any point.
4. **Multi-team.** Team 10101 only, or shared with alliance partners at events? Changes the scoping model (S3), the auth model (C9), and the chat design (X1–X7) substantially.
5. **Chat moderation.** Which mentor owns the log review (X7), and what is the retention policy?
6. **Off-site backup.** With Render retired the Pi holds the only authoritative copy. Whose laptop receives the between-blocks copy (Q4), and who verifies it ran?
7. **DB viewer.** Rebuild it guarded (U17), or drop it entirely?
8. **Rules.** Pending P2 — the E143 answer determines whether the network topology in P6 is legal as planned.
9. **HTTPS on the event LAN.** Service workers (C1), installing the app and keeping its storage (C2), OPFS (C4), and `crypto.subtle` (C9) all need https; the Pi serves plain http (found in C2, see Phase 3 notes). The options:
   - **A local certificate authority** (e.g. `mkcert`) whose root is installed on every client. It is practical on team tablets and painful on personal phones, so it ties to decision 1.
   - **A real domain with a Let's Encrypt certificate** (DNS-01) whose name points at the Pi's LAN address. It needs internet to renew every 90 days, and a DNS answer at a venue with no internet: the Pi would serve DNS on the wired LAN (P6).
   - **Staying on http**, and accepting that Phase 3's offline work cannot run in a browser.

   Phase 3 beyond C2 and C3 should wait on this.

---

## Explicitly dropped

| Item | Why |
| --- | --- |
| Wi-Fi access point on the Pi | Violates FRC rule E143. Not built at all — the old code's AP path is gone with the rest. |
| Wi-Fi HaLow | No client device supports it. Revisit only as a pit-to-stands Pi-to-Pi bridge, and only if Ethernet and QR both fail. |
| A client-side SPA / Leptos rewrite | Unnecessary. Askama compiles to wasm, so a service worker can render the same pages with no framework and no client router. |
| Unpoly | Designed away at U8. Plain navigations, 303s, and a small `live.js` cover what its glue layer did, with one response mode instead of two (U9). |
| IndexedDB as the browser store | Key-value only; you would hand-write every join. SQLite-WASM on OPFS lets the SQL mostly port. |
| ElectricSQL / PowerSync | Neither has a Rust/wasm client story that fits, and these conflict rules are simpler than what they solve. |
| Web Push notifications | Structurally impossible without internet. |
| `awards`, `zebra_data`, `scouting_submissions.status` | Created and never written by any of the three retired ports. Do not recreate without a writer (D12). |
| Postgres | Retired with Render. SQLite from the first line of schema. |

---

## Where the old IDs went

For anyone holding a printout of either source list.

| Source | Now |
| --- | --- |
| RI-N1 → P1 · N2 → P3 · N3 → P4 · N4/N5 → P6 · N6 → S13 · N7 → P8 · N8 → P2 | Platform |
| RI-U1 → D4 · U2 → D5 · U3 → U4 · U4 → L3 · U5 → U21 · U6 → U22 · U7 → U14 · U8 → U11 · U9 → U2 · U10 → U16 | Interface |
| RI-S2 → S1 · S3 → S4 · S4 → S5 · S5 → I8 · S6 → S6 · S7 → S7 · S8 → D7 · S9 → S8 · S10 → I9 · S11 → I12 · S12 → S13 · S13 → I14 | Sync |
| RI-O1 → C1 · O2 → C2 · O3 → C3 · O4 → F1 · O5 → F2 · O6 → F3 · O7 → C4 · O8 → C5 · O9 → C6 · O10 → C7 · O11 → I11 · O12 → C9 · O13 → C10 · O14 → L14 · O15 → S2 · O16 → S3 · O17 → S10 · O18 → S11 · **O19 → absorbed into D1–D11** | Offline |
| RI-M1…M7 → X1…X7 | Chat |
| RI-A1 → L5 · A2 → S9 · A3 → L6 · A4 → L13 · A5 → C8 | Assignments |
| RS §12.1 → D3/D5 · §12.2 → D6 · §12.3 → D12 · §12.4 → U17 · §12.5 → L10 · §12.6 → U18 · §12.7 → U15 · §12.8 → S9 · §12.9 → L11 · §12.10 → D11 · §12.11 → D2 · §12.12 → U2 · §12.13 → A6 | Defect fixes |

The four numbering errors in the original refurbish tables are corrected here: `client_record_id` was labelled S3 but is S8 (now D7); QR transfer was labelled S5 in Phase 3 but is S12 (now S13); the N2/O19 Postgres contradiction is resolved by dropping Postgres outright; and the "few hundred lines of Go" note in RI §3 describes work that is now Rust.
