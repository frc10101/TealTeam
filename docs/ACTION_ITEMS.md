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
| U16 | Mobile pass: bottom nav, 44px touch targets, card layouts under 600px | RI-U10 · RS §7 | M |  |
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

### Coach and pick list

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U18 | **Coach panel reads the local `matches` table**, not the live FIRST schedule. It was non-functional offline, at exactly the event where it matters most | RS §12.6 | M || **Done** — `/drive-coach`, `tt_core::coach` |
| U19 | Match status classification (±15 min windows) as a pure function in `tt-core` | RS §5.7 | S | **Done** — `tt_core::matches::classify`, since the ingestion commit; see Phase 2 notes |
| U20 | Pick list read / upsert / delete | RS §5.8 | S | **Done** — `/pick-list`; edits, not positions, so two people at once both land |

### Platform

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| P3 | SQLite WAL, single writer, on **NVMe/USB SSD — not the SD card** | RI-N2 · RS §10 | M |  |
| P4 | Avahi → `http://tealteam.local`. Removes the most common event-day support question | RI-N3 · RS §10 | S |  |
| P5 | Asset resolution: walk up from both the exe and cwd, or embed assets in the binary | RS §10 | S |  |
| P6 | Wired Ethernet to clients + USB tethering as the uplink (`usb0`, route metric). **Build no Wi-Fi AP** — it violates E143 | RI-N4, RI-N5 · RS §10 | M |  |
| P7 | Buy per-client 25 ft flat Ethernet, gaff tape, and USB-C Ethernet adapters (~$15 each) | RI §1 | S |  |
| P8 | One-page laminated event-day setup runbook with a photo of the correct cabling | RI-N7 | S |  |
| P9 | Practice the full network setup and teardown twice at the shop, timed, by a student who did not design it | RI §1 | S |  |

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

**Still open in Phase 2:** U16, U17, and P3-P9.

---

## Phase 3 — Offline-first and client-centred

The architectural payoff. Phase 2 must be shipping before this starts.

| # | Action | Source | Effort |
| --- | --- | --- | --- |
| C1 | **Service Worker + app-shell precache + navigation fallback.** WASM alone makes nothing offline; this is the piece that does | RI-O1 | M |
| C2 | Web App Manifest, icons, installability, `navigator.storage.persist()` | RI-O2 | S |
| C3 | Debounced form-state persistence and restore — no more lost in-progress entries | RI-O3 | S |
| C4 | `tt-repo-sqlite` for the browser over SQLite-WASM/OPFS | RI-O7 | L |
| C5 | Service Worker fragment interception → wasm handler dispatch | RI-O8 | M |
| C6 | Migrate read-only `/hx/*` routes to wasm, one at a time | RI-O9 | L |
| C7 | Outbox + sync client in `tt-client` | RI-O10 | L |
| C8 | **Make assignments available offline.** An assignment a scout cannot see when the network drops is worse than no assignment | RI-A5 | M |
| C9 | Offline auth tokens (PASETO) layered onto device identity | RI-O12 | M |
| C10 | Conflict review screen for the lead scout | RI-O13 | M |
| C11 | Repo-trait round-trip tests run against **both** implementations, so server and browser cannot diverge | RS §11 | M |

### Sync architecture

| # | Action | Source | Effort |
| --- | --- | --- | --- |
| S1 | `upstream` append-only log fed by the FIRST/TBA clients | RI-S2 | M |
| S2 | `changes` append-only log + `/api/sync/pull` with a lag window. **Not** per-table watermarks — those cannot see deletions and have a commit-ordering race | RI-O15 | M |
| S3 | Scoped subscription filtering + a never-replicate allowlist, so other teams' notes never leak | RI-O16 | M |
| S4 | Compile the FIRST/TBA clients for `wasm32`; client-side conditional fetch with ETags. **Both APIs allow direct browser requests**, so no relay server is needed | RI-S3 | M |
| S5 | Bundle import on the Pi: role-gate the push, `ATTACH`, upsert, advance cursor, audit-log | RI-S4 | M |
| S6 | USB tether as the Pi's automatic uplink; pull bundles whenever `usb0` is up | RI-S6 | M |
| S7 | Opportunistic client fetch: detect signal, fetch upstream, queue bundle, push on reconnect | RI-S7 | M |
| S8 | **SSE fan-out endpoint** with `Last-Event-ID` resume and a polling fallback | RI-S9 | M |
| S9 | **Push assignment changes over SSE** instead of re-rendering the whole grid on every click | RI-A2 · RS §12.8 | M |
| S10 | SQLite snapshot bootstrap (`/api/sync/snapshot`, OPFS import) — ship a file, not a million rows | RI-O17 | M |
| S11 | Schema version handshake + blocking update banner. The mid-event deploy footgun | RI-O18 | S |
| S12 | Clients compute and record their clock offset against the server on each sync, so device skew is measurable rather than mysterious | RI §Time Sync | S |

---

## Phase 4 — Analysis and communication

| # | Action | Source | Effort |
| --- | --- | --- | --- |
| U21 | **Graph view**: uPlot + tap-to-toggle metric chips + team chips. Tap, not drag — drag is a desktop metaphor | RI-U5 | L |
| U22 | Notes panel as a separate, filterable, timestamped view | RI-U6 | M |
| L13 | Rotation fairness — track matches scouted per person and suggest rotation, instead of making the lead scout remember | RI-A4 | M |
| L14 | `yrs`-backed collaborative pick list. The one place in this app where a CRDT genuinely earns its keep — two leads reordering currently clobber each other silently | RI-O14 · RS §5.8 | M |
| X1 | `messages` table, `POST /api/messages`, history endpoint with cursor paging | RI-M1 | M |
| X2 | SSE message stream sharing the S8 event channel | RI-M2 | S |
| X3 | Side panel (desktop) + full-screen view (mobile) with unread badges | RI-M3 | M |
| X4 | Offline outbox integration and pending-message rendering | RI-M4 | S |
| X5 | Hybrid logical clock ordering; dual-timestamp display for delayed messages | RI-M5 | M |
| X6 | `#team` / `#match` autocomplete and context chips — the reason to build chat in-app rather than adopt Matrix | RI-M6 | M |
| X7 | **Moderation: mentor log view, retract-not-delete, rate limiting.** Non-negotiable; the users are minors | RI-M7 | M |
| S13 | QR transfer: Rust encoder, browser scanner with `BarcodeDetector` + zxing-wasm fallback | RI-N6, RI-S12 | L |

---

## Cross-cutting

| # | Action | Source | Effort |
| --- | --- | --- | --- |
| Q1 | `tt-core` unit tests: scoring, mode aggregation, match-status, connectivity classification, match-number normalization, TBA fallback extraction. **Every one of these had a bug** | RS §11 | M |
| Q2 | Deserialization tests against **recorded** FIRST/TBA payloads, including at least one from a prior season | RS §11 | M |
| Q3 | Load test before the season: 30 simulated clients, two hours, p95 latency and SSE stability — with the cable pulled, the power killed, and a client's storage filled, deliberately | RI §Load Testing · RS §11 | M |
| Q4 | Backups: timed dump to the SSD (10-minute interval, 24-hour retention), USB copy between match blocks, and **one deliberate restore test** before you need it | RI §Backups | M |
| Q5 | Store everything in UTC; render in the event's IANA zone per `TIMEZONE_HANDLING.md` | RI §Time Sync | S |

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
