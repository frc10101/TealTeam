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
| A6 | Derive user "online" from **heartbeat**, not from an unexpired 24-hour session. Auto-distribute was assigning robots to people who had gone home | RS §12.13 | S | **Done** |

### Core interface

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U1 | Layout + role-gated nav, Askama compile-time-checked templates | RS §7 | M | **Done** |
| U2 | **Event selection as client/URL state, not `sessions.selected_event_id`** — bookmarkable, multi-tab, and a precondition for offline. Persistent header switcher; allow multi-event analysis | RI-U9 · RS §12.12 | M |  |
| U3 | Event summary: team count, match count, roster, "your team is not listed" warning | RS §5.1 | S |  |
| U4 | **Schema-driven submission form renderer** reading D4 — replaces per-season template branching | RI-U3 · RS §5.2 | L | Schema + validator done; renderer pending |
| U5 | Account page, change password, help page | RS §5 | S | **Done** |
| U6 | **Vendor Unpoly and the Tailwind build locally — never CDN.** Hard requirement for an event LAN, not an optimization | RS §7 | S | **Done** |
| U7 | Tailwind component layer (`.btn` / `.card` / `.form-*` / `.alert` / `.badge` / `.data-table` / `.nav-link`) + teal palette | RS §7 | M | **Done** |
| U8 | Re-create or deliberately design away the three Unpoly glue contracts: `tt:navigate` via `X-Up-Events`, `[tt-src]` polling regions, `[tt-change]` select-driven render | RS §7 | M |  |
| U9 | Dual-mode responses keyed on `X-Up-Version` — keeps the app usable when Unpoly fails to load | RS §7 | M |  |
| U10 | Error and success fragments **in templates**, not inline Rust format strings | RS §7, §12 | S |  |

### Phase 1 notes

**Done: the data layer and auth, end to end.** 116 tests. `cargo run -p tt-web` gives a working app: sign up, sign in, sign out, change password, account page with role badges, and device heartbeats — against a migrated WAL-mode SQLite database, with role guards enforced.

**D4 shipped as `crates/tt-core/seasons/2026.json`** — the 2026 game, Rebuilt, embedded at compile time. Embedding rather than reading from disk means a schema typo is a red build instead of an event-day surprise, and the deployed binary can never drift from the schema it was tested against. Editing it is a rebuild, which at Kickoff is the right trade.

**Access control is a type, not a convention.** `LeadScout` / `Coach` extractors mean a handler that lacks the guard fails to compile rather than shipping open. `/lead-scout` and `/drive-coach` exist and are guarded now, with placeholder content, so the nav never links a 404 and the access rule is settled before there is anything on the page worth protecting.

**One table for observations, not two.** The retired design had `scouting_submissions` and `scouting_data` with near-identical columns and copied rows between them, deleting on both approve and decline. This schema has one `observations` table with a `review_state`; approve and decline are updates, nothing is destroyed, and L10's retract-not-delete requirement is already satisfied by the schema.

**Tailwind is not in the build.** `static/css/site.css` is hand-written using the component class names REBUILD_SPEC §7 documents (`.btn`, `.card`, `.form-*`, `.alert`, `.badge`). That removes Node, npm, and a TypeScript compiler from a workflow maintained by students who graduate every four years. Swapping a Tailwind build back in later means replacing one file, not rewriting templates. **This is a deliberate deviation from the stated stack** — revisit at U7 if the utility classes are wanted.

**Still open in Phase 1:** U2 (event selection as client state), U3 (event summary), U4's form renderer, U8-U10 (Unpoly glue, dual-mode responses, error templates). These need events and matches in the database, which is the upstream-sync work in Phase 2.

---

## Phase 2 — Event-ready

Everything a competition weekend actually needs. This is the phase that must ship before kickoff.

### Assignment-driven scouting

This is the highest-leverage cluster in either source document. It removes the 50-team list problem at its root and eliminates wrong-robot entry.

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| L1 | Assignment grid: matches × six robot slots, `"TBD"` for teams not in the local roster | RS §5.6 | L |  |
| L2 | Set / auto-distribute / clear-all / clear-match / rename-device | RS §5.6 | M |  |
| L3 | **Assignment-driven team selection** replacing the team list, with a keypad escape hatch | RI-U4 · RS §5.2 | M |  |
| L4 | Prefill query — next unplayed match, matching `scouter_id` **OR** `device_uuid` | RS §5.2 | M |  |
| L5 | **Lock the scouting form to the assignment**, pre-filled and restricted, with a deliberate override | RI-A1 · RS §5.2, §12 | M |  |
| L6 | **Coverage view**: who is assigned, who has submitted, which robots are uncovered | RI-A3 | M |  |
| L7 | Resolve `submitting_team_id` at write time — it drives the notes privacy rule, and missing it once already required a backfill migration | RS §5.2 | S |  |

### Review pipeline

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| L8 | Pending queue ordered by `created_at`, missing-notes flag | RS §5.3 | S |  |
| L9 | Approve: one transaction, copy into canonical + retract from queue | RS §5.3 | M |  |
| L10 | **Decline → retract, not delete**, with an audit record and feedback to the scout. The old path destroyed data silently with no correction route | RS §12.5 | M |  |
| L11 | Ranking score: weighted sum per row, then **averaged, with `n=` shown**. Summing rewarded volume alone | RS §5.5, §12.9 | M |  |
| L12 | Weight editor: `weight_{metric}__{option}` fields, `[-100, 100]`, whole-form rejection on invalid input | RS §5.5 | S |  |

### Team analysis

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U11 | **Consolidate team stats into one `TeamProfile` view model** — synced stats plus scouting aggregates, empty strings for absent values rather than zeros | RI-U8 · RS §5.4 | M |  |
| U12 | **Pick one aggregation rule.** Mode for some fields and latest-row for others was an accident, not a design | RS §5.4, §12 | S |  |
| U13 | Notes filtered to the viewer's own `submitting_team_id`; no-team viewers see none | RS §5.4 | S |  |
| U14 | **Provenance badges** (`n=`, `scouted_at`, `synced ago`) on every aggregate | RI-U7 | S |  |
| U15 | **Remove synchronous upstream calls from page renders.** `/teams` and the team-select fallback both blocked a render on the network | RS §12.7 | M |  |
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
| I7 | Background loop: 2 min during active events, 3 hr otherwise, ±7-day fallback, 24-hr lookahead, 120s per-pass timeout | RS §6.2 | M | Cadence + sync_active done; loop task pending |
| I8 | **Pre-event bulk load** — full upstream snapshot, one command, verifiable row counts. An afternoon of work that covers most of the tedious data before you leave the shop | RI-S5 | S | **Done** |
| I9 | ETag / conditional requests on the TBA poller | RI-S10 | S |  |
| I10 | Connectivity tracker: TCP connect to `1.1.1.1:443`, 1500 ms, 3s cache, skip loopback/RFC1918/link-local | RS §6.4 | S | **Done** |
| I11 | **Four-state connection chip describing the client's link to the server**, not the server's internet — and remove all "offline mode" toggle language | RI-O11 · RS §6.4, §12 | S |  |
| I12 | **Upstream freshness badges**; amber past 20 minutes during quals. Stale rankings that look live cause bad picks | RI-S11 | S | `is_stale` + `synced_at` done; badges pending |
| I13 | `POST /api/frc/sync` manual sync, admin/lead only | RS §6.1 | S |  |
| I14 | **Manual rankings entry screen** — the true last resort. A lead scout can type 40 rows off the audience display in five minutes, and it has never once failed to work | RI-S13 | S |  |

### Coach and pick list

| # | Action | Source | Effort | Status |
| --- | --- | --- | --- | --- |
| U18 | **Coach panel reads the local `matches` table**, not the live FIRST schedule. It was non-functional offline, at exactly the event where it matters most | RS §12.6 | M |  |
| U19 | Match status classification (±15 min windows) as a pure function in `tt-core` | RS §5.7 | S |  |
| U20 | Pick list read / upsert / delete | RS §5.8 | S |  |

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

**Done so far: upstream ingestion.** The FIRST and TBA clients, the uplink probe, and the sync that lands events, rosters, matches, and statistics in the database. 12 integration tests run the whole path — stub HTTP server, real clients, real SQLite — against payloads shaped like the real thing.

**The two schema-drift bugs from `TBA_SCHEMA_FIX_SUMMARY.md` are fixed and pinned by tests.** Component OPRs are found by dynamic name (`totalAutoPoints`, not a fixed `auto_oprs` field), and ranking points fall back to `sort_orders` / `extra_stats` when the legacy primitives are null. Both have tests named after the symptom, so a future "simplification" to direct field access fails loudly.

**Partial success is the design, not an accident.** `SyncReport` carries counts and problems together: one event's roster failing does not abandon the other eleven, and a missing component-OPR endpoint does not discard the rankings that came with it. Only a total loss of connectivity aborts.

**Parsing is in `tt-core`, transport in `tt-upstream`.** That split keeps the deserializers wasm-clean for S4, where a client with signal fetches upstream itself and hands the Pi a bundle — the reason the refurbish plan needs no relay server.

**Still open in Phase 2:** the sync loop task (I7 has its cadence and `sync_active`, but nothing spawns it yet), I9, I11, I13, I14, all of L1-L12, U11-U20, and P3-P9.

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
| A client-side SPA / Leptos rewrite | Unnecessary. Askama compiles to wasm and Unpoly fetches fragments over HTTP, so the UI can be served by wasm handlers with no framework and no client router. |
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
