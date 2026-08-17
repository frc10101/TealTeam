# Rust Port Architecture (`rust/tealteam-web`)

The Rust/axum implementation of TealTeam. This document covers how it is put
together and how to work in it; per-item reference documentation lives in the
code itself and is browsable with `cargo doc` (see
[Generated API docs](#generated-api-docs)).

## What it is

`rust/tealteam-web` is one of three interchangeable implementations of the same
application:

| Implementation | Location | Stack |
|---|---|---|
| Go (original) | `cmd/`, `internal/` | Gin + `html/template` |
| .NET | `dotnet/TealTeam.Web` | ASP.NET Core MVC + Razor |
| Rust | `rust/tealteam-web` | axum + Askama |

All three serve the **same routes**, run the **same SQL migrations** against the
**same PostgreSQL schema**, and issue the **same `session_id` cookie** with
bcrypt password hashes. They can run side by side against one database, and an
account created in any of them signs into the others.

The target deployment is a LAN server at a competition — often a Raspberry Pi —
with scouting tablets on wired ethernet. That shapes most of the design
decisions below: the venue may have no usable uplink, so the app has to keep
working when the FIRST API, The Blue Alliance, or even the database is
unreachable.

## Stack decisions

- **axum + Askama, not a heavier framework.** The app is server-rendered HTML
  plus Unpoly fragments. Handlers returning rendered Askama templates map 1:1
  onto the Gin handlers and the C# controllers. Askama is compile-time checked
  (the closest analog to Razor), so a malformed template or a field a template
  references but the struct does not have is a build error, not a runtime 500.
- **sqlx with runtime queries (`query`/`query_as`), not the compile-time
  macros.** The schema is owned by the shared `migrations/` directory and
  created at boot, so there is no compile-time database to check against.
  Hand-written SQL also keeps the queries readable next to the Go/C# ports.
- **bcrypt** at cost 12, matching the other ports.
- **rustls** (no OpenSSL) for `reqwest` and `sqlx`, so the release binary has no
  system TLS dependency and cross-compiles cleanly to `aarch64` for the Pi.
- **Unpoly is vendored locally** (`static/js/unpoly.min.js` + `.css`) so the UI
  works on an offline event LAN. `static/js/tt-unpoly.js` is a small glue layer
  covering the two places Unpoly differs from the HTMX fragments the UI was
  ported from: server-driven navigation (an `X-Up-Events: tt:navigate` response
  header, the analog of HTMX's `HX-Redirect`) and the `[tt-src]` / `[tt-change]`
  self-load helpers for regions whose responses are bare inner fragments.

## Layout

The crate is organised as MVC, mirroring the C# port's Controllers/Models/Views
split.

```text
src/main.rs              startup: config, migrations, boot sync, bind
src/config.rs            env/.env config, migrations//static/ path lookup
src/routes.rs            URL table — every route maps to one controller action
src/state.rs             AppState (the sqlx pool) shared with every handler
src/web.rs               HTTP plumbing: error type, forms, Unpoly helpers, current user
src/db.rs                SQL migration runner (schema_migrations, shared with Go/C#)

src/models/*             MODEL — entities (sqlx::FromRow) + every SQL statement
  user, session            accounts, bcrypt, cookie + DB sessions
  event, team, stats       events, rosters, synced TBA/FIRST statistics
  scouting, scouting_points  submissions, approved data, ranking point weights
  assignment, device       matches, per-match slot assignments, scouting tablets
  pick_list, schema        pick list entries, DB-viewer introspection

src/views/*              VIEW — Askama template structs + view models/formatting
  home, auth, events, submission, lead_scout, assignments,
  teams, matches, coach, db_viewer, network

src/controllers/*        CONTROLLER — request handling, no SQL and no markup
  home, auth, submission, lead_scout, assignments,
  teams, matches, coach, db_viewer, api

src/services/*           external world: FIRST/TBA clients, sync jobs, connectivity

templates/*              Askama templates (layout + pages + partials)
static/*                 vendored Unpoly, tt-unpoly.js glue, site JS/CSS, device.js
```

### Dependency direction

```text
routes ──▶ controllers ──┬──▶ models ──▶ PostgreSQL
                         ├──▶ services ──▶ FIRST API / The Blue Alliance
                         └──▶ views ──▶ Askama templates ──▶ HTML
                                 │
                                 └──▶ models (types only, never the database)
```

One way, no cycles:

- **Controllers** may use models, views, services and `web`.
- **Views** may read model types, but never touch the database.
- **Models** and **services** know nothing about HTTP.

The rules that keep it honest:

1. **All SQL lives in `src/models/`.** If a controller needs data that no model
   function returns, add the function — do not inline a query.
2. **All markup lives in `templates/` or `src/views/`.** A handful of small
   fragments are built with `format!` in `views` (auth banners, the team
   `<select>`); those escape their inputs explicitly.
3. **All outbound network calls live in `src/services/`.**

## Runtime startup flow

1. `config::Config::from_environment` loads `.env` (crate dir, then repo root),
   validates `TEALTEAM_ENV`, and resolves the database URL.
2. Tracing is initialised (`RUST_LOG`, default `info,sqlx=warn`).
3. A **lazy** sqlx pool is opened (25 connections, 10s acquire timeout) and
   probed with `SELECT 1`.
   - If the probe fails the server **still starts**, logs a warning, and serves
     degraded pages. This matches the Go app and matters when the box powers on
     before the database container does.
4. In `test` mode, migration history is dropped; then `migrations/*.sql` is
   applied through the shared `schema_migrations` table.
5. `services::first_sync::sync_on_boot` runs unless `FIRST_SYNC_ON_BOOT=false`
   (60s timeout, non-fatal).
6. `services::stats_syncer::run` is spawned as a background task; it exits
   immediately if `TBA_AUTH_KEY` is unset.
7. `routes::router` is mounted, `/static` is served from the resolved static
   directory, and the server binds `0.0.0.0:$PORT`.

`migrations/` and `static/` are located by walking up from both the executable's
directory and the current working directory, so the same binary works under
`cargo run`, from `target/release/`, and in a publish layout where those
directories sit next to the binary.

## Request model

Three route shapes, distinguished by prefix:

| Prefix | Returns | Example |
|---|---|---|
| bare | full HTML page | `/teams`, `/lead-scout` |
| `/hx/*` | HTML fragment for Unpoly | `/hx/matches/schedule` |
| `/api/*` | JSON, or an action + redirect | `/api/pick-list` |

The `/hx/` prefix is a leftover from the HTMX version the UI was ported from;
the fragments are now served to Unpoly.

A page and its fragments are usually the same controller. `web::is_unpoly`
(presence of the `X-Up-Version` header) decides which to return, and mutating
actions generally answer with the **re-rendered fragment** rather than a
redirect, so the page updates in place. Where a full navigation is needed after
a mutation — sign-in, sign-out, submission review — `web::up_navigate` emits the
`X-Up-Events: tt:navigate` header that `tt-unpoly.js` turns into a location
change.

Fragments that are both embedded in a page and served on their own (event
picker, submission panel, assignment table, device list) are rendered to a
string with `views::render_html` and passed into the page struct as a `_html`
field, so first paint and later swaps produce identical markup.

### Route table

```text
Pages
  GET  /                                  home: event picker, summary, live schedule
  GET  /help                              usage guide
  GET  /sign-in  /sign-up  /account       auth pages
  GET  /submission                        scouting form
  GET  /teams                             team lookup
  GET  /lead-scout                        review queue, rankings, pick list
  GET  /lead-scout/submissions/:id        one submission in full
  GET  /lead-scout/weights                point weight settings
  GET  /lead-scout/assignments            per-match assignment grid
  GET  /drive-coach                       drive coach panel
  GET  /development/db                    admin database viewer

Fragments (Unpoly)
  GET  /hx/events/summary                 event summary panel
  GET  /hx/matches/schedule               live match schedule (±15 min window)
  GET  /hx/teams/search  /hx/teams/data   team card, per-event data
  POST /hx/teams/fetch-past-events        force a FIRST re-sync for a team
  POST /hx/lead-scout/submissions/:id/approve|decline
  POST /hx/assignments/set|auto|clear-all|clear-match/:match_id
  POST /hx/devices/:id/rename
  GET  /hx/drive-coach/matches            coach schedule (polled)
  GET  /hx/development/db/table/:name     one page of one table
  GET  /hx/network/status                 connectivity badge

APIs
  POST /api/auth/login|signup|logout
  POST /api/account/change-password
  POST /api/events/select                 store selected event on the session
  POST /api/frc/sync                      manual FIRST sync (lead scout/admin)
  POST /api/device/heartbeat              tablet check-in
  GET  /api/pick-list                     pick list entries
  POST /api/pick-list/entry               upsert entry
  DEL  /api/pick-list/entry?team=         remove entry
  GET  /api/network/status                connectivity as JSON
  POST /submission                        queue a scouting submission
  GET  /submission/event-teams            team <select> for an event
```

`src/routes.rs` is the authority; keep it in sync with the Go and .NET routers.

## Identity, sessions and roles

- Sessions are **server-side rows** in `sessions`, keyed by a 256-bit random id.
  The cookie carries nothing but that id. They last 24 hours and expired rows
  are deleted lazily on the request that presents them.
- The session row also holds `selected_event_id`. **Selecting an event is the
  hinge of the whole app** — submissions, assignments, rankings, pick lists and
  the coach schedule are all scoped to it. It lives on the session, not the
  user, so one account can work two events from two devices.
- Roles are three independent booleans on `users`: `is_admin`, `is_lead_scout`,
  `is_coach`. `is_admin` implies the other two (`User::can_lead`,
  `User::can_coach`), and every account can scout.
- Authorization is checked **inside each controller action**, not in middleware,
  because the right response differs by route: a page redirects, a fragment
  endpoint returns a status code, a JSON API returns a JSON error. Hiding a nav
  link is cosmetic (`views::Nav`); the server always re-checks.
- Scouting tablets additionally carry a permanent `device_uuid` cookie planted
  by `static/js/device.js` and refreshed by `POST /api/device/heartbeat`. A
  device seen within the last 3 minutes counts as online and can be assigned
  robots even when nobody is signed in on it.

## Data model

Owned by the shared `migrations/` directory; the Rust port creates no tables of
its own.

| Domain | Tables | Written by |
|---|---|---|
| Identity | `users`, `sessions` | the app |
| Competition | `events`, `teams`, `event_teams` | FIRST sync |
| Schedule/results | `matches`, `team_event_stats` | TBA sync |
| Scouting | `scouting_submissions`, `scouting_data` | scouts + lead scout review |
| Coordination | `scout_assignments`, `devices`, `pick_list_entries` | the app |
| Configuration | `scouting_point_weights` | lead scout |
| Bookkeeping | `schema_migrations` | the migration runner |

Two details are easy to trip over:

- **Team numbers vs team ids.** `matches` stores FRC **team numbers** in its six
  robot slots (that is how FIRST and TBA publish schedules), while
  `scouting_data`, `event_teams` and `scout_assignments` reference the local
  **`teams.id`**. Model functions are named for which one they take
  (`team::id_by_number`, `team::number_by_id`, `team::lookup_for_event`).
- **`NUMERIC` columns need casts.** sqlx will not decode `NUMERIC` into `f64`,
  so every read of `team_event_stats` goes through
  `TeamEventStats::SELECT`, which casts with `::float8`.

### Scouting data flow

```text
scout submits ──▶ scouting_submissions ──▶ lead scout reviews ──┬─ approve ─▶ scouting_data
                                                                └─ decline ─▶ deleted
```

Scouts never write `scouting_data` directly. Approval moves the row in one
transaction, so a submission can never be both approved and still queued.

Each row carries `submitting_team_id` — the team that collected the
observation. **Free-text notes are only ever shown back to that team**; the
structured fields are shared with everyone, so alliance partners can see
capability without reading another team's private commentary. Legacy rows
predate the column, so approval resolves it from the scouter.

Structured fields are lowercase keyword strings (`"high"`, `"l3"`, `"trench"`),
not enums, matching the Go schema. `models::scouting_points` scores them against
a weight table: built-in defaults, overridden per option by rows in
`scouting_point_weights`, so a lead scout can retune what the team values for
this game without a redeploy. A team's ranking score is the **sum over all of
its scouted matches**, which is why it grows with matches scouted and is shown
next to a match count.

## External data

Two upstreams answering different questions:

| Source | Authoritative for | Client | Sync |
|---|---|---|---|
| FIRST Events API v3 | what **exists**: events, rosters, match schedule | `services::first_api` | `services::first_sync` |
| The Blue Alliance v3 | what **happened**: rankings, OPR/DPR, results | `services::tba` | `services::tba_stats_sync` |

FIRST sync entry points:

- `sync_on_boot` — once at startup, unless `FIRST_SYNC_ON_BOOT=false`.
- `sync_now` — the manual sync behind `POST /api/frc/sync`.
- `sync_team_for_user` — on sign-in, sign-up and team lookup; fetches only that
  team's events, then triggers a TBA stats sync for them.

What gets pulled is narrowed by `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER` and
`FIRST_COUNTRY` (default `USA`) — a whole season of worldwide events is far
more than one team's Pi needs. Everything upserts, so repeated syncs are safe.

TBA sync runs on a background loop whose cadence adapts: every 2 minutes while
an event is running or starts within 24 hours, every 3 hours otherwise.
Rankings move constantly during a competition and not at all between them.

Both clients are no-ops without credentials, in which case the app runs on
whatever is already in the database.

## Offline behaviour

This is the part most worth understanding before changing anything.

Every outbound call goes through `services::connectivity`:

- a TCP preflight to `1.1.1.1:443` (fast enough to run per call; cached for 3s
  so a burst probes once),
- up to 3 attempts with 250ms/500ms/1s backoff on throttling and 5xx,
- the outcome recorded in a process-wide snapshot.

That snapshot lets callers distinguish "the internet is down"
(`connectivity::is_internet_unavailable`) from "the API said no", which is what
the UI actually needs to tell a scout. It drives the status badge
(`GET /hx/network/status`) and the JSON endpoint (`GET /api/network/status`),
classified as `internet-ok` / `api-error` / `offline`. A recent successful API
call outranks a failed probe — if calls are landing, the uplink works.

LAN and loopback base URLs skip the check entirely, so a local mock or an
on-site mirror works with no internet at all.

Degradation is layered:

| Failure | Behaviour |
|---|---|
| FIRST unreachable, matches synced from TBA | home schedule shows the cached schedule with a note saying so |
| FIRST unreachable, nothing cached | schedule panel shows a sentence explaining why |
| No FIRST/TBA credentials | app runs on locally stored data; sync logs and skips |
| Database unreachable at boot | server starts, DB-backed pages degrade |

The general rule: **expected failures are rendered, not returned as errors.**
`web::AppError` (a 500) is reserved for the genuinely unexpected.

## Configuration

Loaded from the environment, with `.env` in the crate directory and then the
repo root filling in anything unset (real environment variables always win).

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | local dev DSN | PostgreSQL connection string |
| `RENDER_DATABASE_URL` | — | preferred over `DATABASE_URL` in prod mode |
| `PORT` | `8080` | bind port |
| `TEALTEAM_ENV` | `test` | `test` resets migration history on boot; `prod` does not |
| `RUST_LOG` | `info,sqlx=warn` | tracing filter |
| `FIRST_API_USERNAME`, `FIRST_API_KEY` | — | FIRST Events API credentials; absent disables FIRST sync |
| `FIRST_SEASON` | `2026` | season year |
| `FIRST_SYNC_ON_BOOT` | `true` | set `false` to skip the startup sync |
| `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER`, `FIRST_COUNTRY` | —, —, `USA` | narrow what the sync pulls |
| `TBA_AUTH_KEY` | — | enables the background stats/matches sync loop |

## Running

```sh
# start postgres (from repo root)
docker compose up -d db

cd rust/tealteam-web
cargo run                        # http://0.0.0.0:8080
```

LAN clients connect to `http://<server-ip>:8080`.

Rebuild the Tailwind CSS after editing templates (from the repo root):

```sh
npx tailwindcss -i ./web/tailwind/input.css \
  -o ./rust/tealteam-web/static/css/site.css --minify
```

### Building for the LAN server / Pi

```sh
cargo build --release                                   # host architecture
cargo build --release --target aarch64-unknown-linux-gnu # Raspberry Pi
```

The release binary is self-contained (rustls, no OpenSSL) and the templates are
compiled in, but `static/` and the repo `migrations/` must be reachable at
runtime — deploy the binary alongside a copy of both (the binary looks next to
itself first, then walks up the repo layout).

For Pi event deployment and autostart, see `docs/PI_EVENT_BOOT.md`.

## Working in the code

### Adding a page

1. **Model** — add the query to the right `src/models/*` module, returning
   entities or a small row struct. Read paths that a page can render without
   should degrade to an empty result; write paths return `Result`.
2. **View** — add the template to `templates/pages/`, and a matching struct in
   `src/views/*` with a `#[template(path = ...)]`. Put formatting, CSS class
   selection and sort order in the view, not the controller.
3. **Controller** — add the action in `src/controllers/*`: resolve the user,
   check authorization, resolve the selected event, load, render.
4. **Route** — register it in `src/routes.rs`.

### Adding a fragment

Same, but the template goes in `templates/partials/`, the controller returns
`Html(render_html(&Fragment { .. }))`, and — if the fragment is also embedded in
a page — the page passes it through as a pre-rendered `_html` field.

### Cross-port parity

Anything that changes the **routes**, the **database schema**, or the **session
cookie** must be mirrored in the Go and .NET ports, or the three stop being
interchangeable. Purely internal Rust changes (layering, naming, view models)
need no mirroring.

### Generated API docs

Every module and public item in the crate carries rustdoc:

```sh
cd rust/tealteam-web
cargo doc --no-deps --document-private-items --open
```

`--document-private-items` matters: this is a binary crate, so most items are
private to it and are omitted without the flag.

The module docs are the reference for anything this document summarises — each
one explains its own invariants (`models::scouting` on the submission
lifecycle, `models::schema` on DB-viewer injection safety,
`services::connectivity` on offline handling, `views` on escaping).
