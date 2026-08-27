# TealTeam — Rebuild Specification

**Status:** Authoritative. This document replaces the retired implementations.
**Date:** 2026-08-26
**Target stack:** Rust + axum + Askama + sqlx + Unpoly + Tailwind
**Companion document:** [RefurbishInstructions.md](../RefurbishInstructions.md) — the *forward* plan (what to change). This document is the *backward* record (what existed and why), written so the app can be rebuilt from an empty directory.

---

## 0. Why this document exists

Three complete implementations of TealTeam existed against one shared PostgreSQL schema — Go/Gin, ASP.NET Core, and Rust/axum. All three have been deleted, along with the Render deployment, the shared `migrations/` directory, the Docker/compose stack, the Pi boot scripts, and the Tailwind/TypeScript front-end sources.

They were deleted deliberately. The Rust port was a *faithful* port: ~187 inline `sqlx::query` calls lived inside axum handlers, which meant the query layer could not cross to `wasm32` and the client-centred architecture the team wants was unreachable without a rewrite anyway. Keeping three ports alive meant every feature cost three times what it should. Keeping one port alive would have anchored the rebuild to a server-centric shape.

What follows is everything worth carrying forward: the domain, the data model, the routes, the business rules, the upstream API behavior, and the specific bugs and design mistakes that should not be repeated.

**Read this document alongside `RefurbishInstructions.md`.** This one tells you what the app *did*. That one tells you what it *should* do differently. Where they conflict, the refurbish plan wins — it was written knowing this system's flaws.

---

## 1. What TealTeam is

An FRC (FIRST Robotics Competition) scouting and analytics application for a high-school robotics team. It combines two data sources:

1. **Manual scouting observations** — students in the stands watch individual robots during matches and record structured qualitative observations (defense rating, hang level, shooting speed, free-text notes).
2. **Official and community competition data** — event schedules, team rosters, rankings, OPR/DPR/CCWM statistics, and match results pulled from the FIRST Events API and The Blue Alliance (TBA).

It merges the two into per-team, per-event profiles used to make alliance-selection and match-strategy decisions during a competition weekend.

### The operating environment (this drives every design decision)

- The server runs **at the event**, on a Raspberry Pi 5, on a LAN with no reliable internet.
- Clients are **phones and tablets** held by students, on small screens, often with no signal.
- FRC event rules (**E143**) prohibit running your own Wi-Fi access point in the venue. Connectivity is wired Ethernet, USB tethering, or sneakernet.
- The game **changes completely every January**. Any structure hardcoded to one season's game is a January rewrite.
- The developers and users are **high-school students**. Complexity that a professional team would absorb is complexity this team cannot maintain.

### Roles

| Role | Flag | What they do |
| --- | --- | --- |
| **Scout** | (default) | Submits observations for one robot in one match. |
| **Lead Scout** | `is_lead_scout` | Assigns scouts to robots per match, reviews and approves/declines submissions, tunes ranking weights, builds the pick list. |
| **Drive Coach** | `is_coach` | Reads the match schedule, alliance partners, and opponent stats. |
| **Admin** | `is_admin` | Everything a lead scout can do, plus the database viewer. |

Role flags are independent booleans on `users`, not a hierarchy. `is_admin` grants lead-scout and coach access by OR, never by inheritance in the data.

---

## 2. Data model

The schema below is the consolidated end state after 13 migrations. PostgreSQL types are given because that is what existed; **the rebuild targets SQLite** (see the refurbish plan, item O19), so dialect notes follow each section.

### 2.1 Identity

```sql
users (
    id              SERIAL PRIMARY KEY,
    email           VARCHAR(255) UNIQUE NOT NULL,
    name            VARCHAR(255) NOT NULL,
    password_hash   VARCHAR(255) NOT NULL DEFAULT '',
    team_number     INTEGER,              -- FRC team number, nullable
    role            VARCHAR(50) DEFAULT 'user',   -- vestigial; the booleans are authoritative
    is_admin        BOOLEAN NOT NULL DEFAULT FALSE,
    is_lead_scout   BOOLEAN NOT NULL DEFAULT FALSE,
    is_coach        BOOLEAN NOT NULL DEFAULT FALSE,
    last_login      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at      TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
)
-- INDEX on (email)

sessions (
    session_id          VARCHAR(255) PRIMARY KEY,   -- 32 random bytes, URL-safe base64
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    selected_event_id   INTEGER REFERENCES events(id) ON DELETE SET NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
)
-- INDEX on (user_id), (expires_at), (selected_event_id)
```

**`sessions.selected_event_id` is load-bearing.** The currently selected event is *server session state*, not a URL parameter or client state. Nearly every page reads it. This is one of the most server-centric decisions in the old design and the rebuild should reconsider it — see §12.

### 2.2 Competition graph

```sql
teams (
    id            SERIAL PRIMARY KEY,
    team_number   INTEGER NOT NULL,       -- NOT unique in the old schema; it should be
    name          VARCHAR(255) NOT NULL,
    school        TEXT,
    city          VARCHAR(255),
    state         VARCHAR(50),
    tba_key       VARCHAR(20),            -- "frc{team_number}"
    nickname      VARCHAR(255),
    school_name   TEXT,
    country       VARCHAR(100),
    rookie_year   INTEGER,
    motto         TEXT,
    website       VARCHAR(500),
    created_at    TIMESTAMPTZ, updated_at TIMESTAMPTZ
)
-- INDEX on (team_number), (tba_key)

events (
    id            SERIAL PRIMARY KEY,
    name          VARCHAR(255) NOT NULL,
    location      VARCHAR(255),           -- "venue, city, stateprov, country" joined with ", "
    timezone      VARCHAR(50),            -- IANA identifier, e.g. "America/Los_Angeles"
    start_date    DATE,
    end_date      DATE,
    tba_key       VARCHAR(20),            -- "{year}{event_code_lowercase}", e.g. "2026mabil". NOT unique; it should be
    event_type    VARCHAR(50),
    district_key  VARCHAR(20),
    week          INTEGER,
    created_at    TIMESTAMPTZ, updated_at TIMESTAMPTZ
)
-- INDEX on (tba_key), (timezone)

event_teams (
    id          SERIAL PRIMARY KEY,
    event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    team_id     INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ,
    UNIQUE(event_id, team_id)
)
```

### 2.3 Matches

```sql
matches (
    id                SERIAL PRIMARY KEY,
    event_id          INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    match_number      INTEGER NOT NULL,
    match_type        VARCHAR(50) DEFAULT 'qualification',  -- in practice written as comp_level: qm/sf/f
    red_score         INTEGER DEFAULT 0,
    blue_score        INTEGER DEFAULT 0,
    played            BOOLEAN DEFAULT FALSE,
    tba_key           VARCHAR(50),
    comp_level        VARCHAR(10),        -- qm | sf | f
    set_number        INTEGER,
    scheduled_time    TIMESTAMPTZ,
    actual_time       TIMESTAMPTZ,
    winning_alliance  VARCHAR(10),        -- "red" | "blue" | ""

    -- Alliance robot slots, stored as team NUMBERS (not FKs) so unsynced teams
    -- don't block display. Added in migration 0016.
    red1, red2, red3, blue1, blue2, blue3   INTEGER,

    -- ~38 columns of 2022-season-specific score breakdown, per alliance:
    --   {red|blue}_auto_tower_points, _endgame_tower_points,
    --   _hub_{auto|teleop|endgame|total}_{count|points},
    --   _energized_achieved, _supercharged_achieved, _traversal_achieved,
    --   _minor_foul_count, _major_foul_count, _foul_points, _rp,
    --   _total_auto_points, _total_teleop_points
    -- ALL DEAD. Never written by any sync path. See §12.1.

    created_at TIMESTAMPTZ, updated_at TIMESTAMPTZ,
    UNIQUE(event_id, match_number, match_type)
)
-- INDEX on (event_id), (match_number), (tba_key)
```

**Match number normalization:** for `comp_level = "qm"` or `set_number <= 0`, `match_number` is TBA's match number verbatim. Otherwise it is `set_number * 100 + match_number`, so playoff sets collapse into a single sortable integer. This is a hack that survived because `UNIQUE(event_id, match_number, match_type)` needed a unique integer; a rebuild should key on `tba_key` instead.

### 2.4 Scouting

Two tables with **near-identical column sets**: `scouting_submissions` (the pending queue) and `scouting_data` (approved, canonical). A submission is *moved* from one to the other on approval.

```sql
scouting_submissions (
    id                   SERIAL PRIMARY KEY,
    event_id             INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    team_id              INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    alliance_color       VARCHAR(10) NOT NULL,   -- "red" | "blue"
    notes                TEXT,
    starting_position    VARCHAR(20),   -- left | center | right
    defense_rating       VARCHAR(20),   -- high | mid | low
    traversal            VARCHAR(20),   -- trench | bump
    scoring_strategy     VARCHAR(50),   -- scoring | defending | passing
    shooting_speed       VARCHAR(20),   -- fast | medium | slow
    capacity             VARCHAR(20),   -- high | medium | low
    defendability        TEXT,          -- free text
    hang_level           VARCHAR(10),   -- none | l1 | l2 | l3
    auto_hang            VARCHAR(10),   -- yes | no
    hang_position        VARCHAR(20),   -- left | center | right
    accuracy_rating      VARCHAR(20),   -- written by nothing; read by the team page aggregate
    scouted_at           TIMESTAMPTZ,
    scouter_id           INTEGER REFERENCES users(id) ON DELETE SET NULL,
    submitting_team_id   INTEGER REFERENCES teams(id) ON DELETE SET NULL,
    status               VARCHAR(20) NOT NULL DEFAULT 'pending',  -- vestigial, see below
    rejection_reason     TEXT,                                     -- vestigial
    created_at           TIMESTAMPTZ
)

scouting_data (
    -- same columns, minus status/rejection_reason, plus updated_at
    -- and NO match reference. See §12.2.
)
```

**Every one of those enumerated fields is hardcoded to the 2022 game.** "Hub", "traversal", "tower" are 2022 Rapid React vocabulary — meaningless for 2026's game, Rebuilt. This is the single biggest structural flaw in the old design and the reason `RefurbishInstructions.md` §2C exists: the rebuild must store a **season-schema-driven JSON payload** (`payload jsonb` + `schema_version`) instead of fixed columns, with the season's fields declared in something like `seasons/2026.json`.

**`status` / `rejection_reason` are dead.** Migration 0010 added a rejection workflow; migration 0012 normalized blank values to `'pending'`. But approve *deletes* the row and decline *deletes* the row, so nothing ever holds any other status. Either build the workflow or drop the columns — do not ship the half-state again.

### 2.5 Derived statistics

```sql
team_event_stats (
    id               SERIAL PRIMARY KEY,
    team_id          INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    event_id         INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    opr, dpr, ccwm                        DECIMAL(10,4),
    auto_opr, teleop_opr, endgame_opr     DECIMAL(10,4),   -- TBA component OPRs
    rank             INTEGER,
    matches_played   INTEGER DEFAULT 0,
    qual_average     DECIMAL(10,4),
    avg_match_points NUMERIC(8,4),
    wins, losses, ties, dq_count          INTEGER,
    qual_points, elim_points, award_points, alliance_points, total_points  INTEGER,
    created_at TIMESTAMPTZ, updated_at TIMESTAMPTZ,
    UNIQUE(team_id, event_id)
)
```

**Postgres `NUMERIC` decoding gotcha:** sqlx cannot decode `NUMERIC` into `f64` without an explicit cast. Every select against this table used `opr::float8 AS opr` and friends. Under SQLite this disappears — store these as `REAL`.

### 2.6 Devices and assignments

```sql
devices (
    id            SERIAL PRIMARY KEY,
    device_uuid   VARCHAR(64) UNIQUE NOT NULL,  -- browser-generated, persistent
    name          VARCHAR(100),                 -- lead-scout-assigned friendly name
    team_number   INTEGER,
    last_seen_at  TIMESTAMPTZ,                  -- heartbeat
    created_at TIMESTAMPTZ, updated_at TIMESTAMPTZ
)

scout_assignments (
    id           SERIAL PRIMARY KEY,
    match_id     INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id      INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    event_id     INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    scouter_id   INTEGER REFERENCES users(id) ON DELETE CASCADE,
    device_id    INTEGER REFERENCES devices(id) ON DELETE CASCADE,
    assigned_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ, updated_at TIMESTAMPTZ,
    UNIQUE(match_id, team_id),
    CHECK (scouter_id IS NOT NULL OR device_id IS NOT NULL)
)
```

One assignment per robot per match. The assignee is **either** a signed-in user **or** a physical device — the `CHECK` enforces at least one. Assigning by device is what lets a lead scout say "the tablet on the left scouts red 1" regardless of who is logged in on it.

### 2.7 Supporting tables

```sql
scouting_point_weights (
    id           SERIAL PRIMARY KEY,
    metric_key   VARCHAR(64) NOT NULL,   -- e.g. "defense_rating"
    option_key   VARCHAR(64) NOT NULL,   -- e.g. "high"
    points       INTEGER NOT NULL,
    UNIQUE(metric_key, option_key)
)

pick_list_entries (
    id                  SERIAL PRIMARY KEY,
    team_number         INTEGER NOT NULL,   -- the OWNING team (whose list this is)
    event_id            INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    picked_team_number  INTEGER NOT NULL,   -- the team being ranked
    color               VARCHAR(50),
    crossed             BOOLEAN DEFAULT FALSE,
    position            INTEGER DEFAULT 0,
    UNIQUE(team_number, event_id, picked_team_number)
)

awards (id, event_id, team_id, tba_award_type, name, awardee, year)   -- schema only; never written
zebra_data (id, match_id, team_id, timestamps JSONB, x_positions JSONB, y_positions JSONB)  -- schema only; never written

schema_migrations (id, filename UNIQUE, applied_at)  -- migration runner bookkeeping
```

`awards` and `zebra_data` were created in the initial migration and never populated by any code path in any of the three ports. Do not recreate them unless something will write to them.

### 2.8 Migration runner

Migrations were plain `.sql` files in a shared `migrations/` directory, applied in filename sort order at boot, each inside a transaction, with applied filenames recorded in `schema_migrations`. When `TEALTEAM_ENV=test`, the runner **dropped `schema_migrations` first**, forcing every migration to re-run — which worked only because every statement was written idempotently (`CREATE TABLE IF NOT EXISTS`, `ADD COLUMN IF NOT EXISTS`). Migration `0001_init.sql` also opened with unconditional `DROP TABLE ... CASCADE` for every table.

This is a footgun: `TEALTEAM_ENV` defaulted to `test`, so a misconfigured production boot dropped the entire schema. **The rebuild should default to the safe mode and require an explicit opt-in to reset.**

Numbering note: files `0002`, `0003`, and `0004` never existed in the retired tree — the chain was `0001`, then `0005`–`0016`.

---

## 3. Authentication and sessions

### Password handling

- **bcrypt, cost 12.** Chosen so hashes were interchangeable across the Go, C#, and Rust ports — a user created in one could sign into another. With one implementation that constraint is gone; Argon2id is the better choice for a rebuild.
- Minimum password length 8. Email validated only by "contains `@` and contains `.`".
- Login returns a **generic** "Invalid email or password" for both unknown-email and wrong-password, to prevent user enumeration. Preserve this.

### Session lifecycle

1. On successful login, generate 32 random bytes, URL-safe-base64 encode them, insert a `sessions` row with `expires_at = now + 24h`.
2. Set cookie `session_id`: `Path=/`, `Max-Age=24h`, `HttpOnly`, `SameSite=Lax`, **`Secure=false`** (the event LAN is plain HTTP; there is no TLS terminator on the Pi).
3. Every request that needs a user reads the cookie, loads the session, and **deletes it if expired**, returning no user.
4. Logout deletes the session row and clears the cookie.

Sessions are looked up on essentially every request, meaning a database round-trip per request for auth. Acceptable at 50 clients on a LAN; not acceptable for a client-centred rebuild. The refurbish plan proposes PASETO offline auth tokens layered on device identity (item O12).

### Device identity (separate from user identity)

A small client script gives every browser a permanent UUID, independent of who is signed in:

```js
// 1. Read/create a UUID in localStorage under key "tealteam_device_uuid".
//    Fall back to a random per-session id if localStorage throws (private mode).
// 2. Mirror it into a ten-year cookie: device_uuid=<uuid>; path=/; max-age=315360000; samesite=lax
//    (a cookie, so the SERVER can read it on ordinary requests — localStorage cannot be read server-side)
// 3. POST /api/device/heartbeat immediately, then every 60 seconds. Ignore failures (offline is normal).
```

The server upserts `devices` on each heartbeat, keyed on `device_uuid`, updating `last_seen_at` and filling `team_number` from the signed-in user if the device does not already have one (`COALESCE(devices.team_number, EXCLUDED.team_number)` — first team wins, so a borrowed device does not get relabelled).

A device is considered **online** if `last_seen_at >= now - 3 minutes`. A *user* is considered online if they have any unexpired session — which is a much weaker signal (a 24-hour session means "online" for a day after they close the tab). Fix this in the rebuild; use heartbeats for both.

---

## 4. Route surface

Complete route table from the retired Rust router. `/hx/*` denotes fragment endpoints (named for the HTMX era; the app later moved to Unpoly but the prefix stayed).

### Pages

| Method | Path | Access | Purpose |
| --- | --- | --- | --- |
| GET | `/` | public | Home; event selector + event summary |
| GET | `/help` | public | Static help page |
| GET | `/sign-in` | anon only | Sign-in form (redirects to `/` if signed in) |
| GET | `/sign-up` | anon only | Sign-up form |
| GET | `/account` | auth | Profile, role badges, change-password form |
| GET | `/submission` | auth | Scouting entry form |
| GET | `/teams` | public | Team lookup (`?team=<number>`) |
| GET | `/lead-scout` | admin OR lead | Pending queue, team rankings, pick list |
| GET | `/lead-scout/submissions/:id` | admin OR lead | One submission in detail |
| GET | `/lead-scout/weights` | admin OR lead | Point weight editor |
| GET | `/lead-scout/assignments` | admin OR lead | Per-match robot assignment grid |
| GET | `/drive-coach` | admin OR coach | Match schedule with alliance partners |
| GET | `/development/db` | (see §12.4) | Raw table browser |

### JSON / form APIs

| Method | Path | Access | Purpose |
| --- | --- | --- | --- |
| POST | `/api/auth/login` | public | Returns an error fragment on failure, `X-Up-Events: tt:navigate` on success |
| POST | `/api/auth/signup` | public | Same response shape |
| POST | `/api/auth/logout` | auth | Deletes session, navigates to `/sign-in` |
| POST | `/api/account/change-password` | auth | Returns a success/error fragment |
| POST | `/api/events/select` | auth | Writes `sessions.selected_event_id` |
| POST | `/api/frc/sync` | admin OR lead | Manual full FIRST sync; JSON counts |
| POST | `/api/device/heartbeat` | public | Device presence; reads `device_uuid` cookie |
| GET | `/api/pick-list` | auth + team + event | `{entries: [...]}` |
| POST | `/api/pick-list/entry` | auth + team + event | Upsert one entry (JSON body) |
| DELETE | `/api/pick-list/entry?team=<n>` | auth + team + event | Delete one entry |
| GET | `/api/network/status` | public | `{status, data}` connectivity snapshot |

### Fragments

| Method | Path | Returns |
| --- | --- | --- |
| GET | `/hx/events/summary` | Event summary card (team count, match count, roster) |
| GET | `/hx/teams/search` | `#team-info-container` |
| GET | `/hx/teams/data` | Team/event data panel |
| POST | `/hx/teams/fetch-past-events` | Forces a FIRST team sync, re-renders team info |
| GET | `/hx/matches/schedule` | Match schedule list |
| GET | `/hx/drive-coach/matches` | Coach match cards |
| GET | `/submission/event-teams` | A whole `<select id="team-id">` of the event's teams |
| POST | `/hx/lead-scout/submissions/:id/approve` | Navigates to `/lead-scout` |
| POST | `/hx/lead-scout/submissions/:id/decline` | Navigates to `/lead-scout` |
| POST | `/hx/assignments/set` | Re-rendered assignment table |
| POST | `/hx/assignments/auto` | Re-rendered assignment table |
| POST | `/hx/assignments/clear-all` | Re-rendered assignment table |
| POST | `/hx/assignments/clear-match/:match_id` | Re-rendered assignment table |
| POST | `/hx/devices/:id/rename` | Re-rendered device list |
| GET | `/hx/network/status` | Connectivity badge |
| GET | `/hx/development/db/table/:name` | Paged table contents |

`GET /static/*` served the vendored Unpoly bundle, compiled CSS, and client JS from disk.

**Access-control pattern to preserve:** every guarded handler re-loads the user from the session and re-checks the role flags itself. There was no middleware layer doing it once. That is repetitive but it means no route can be accidentally left unguarded by a routing mistake — and it is what should be re-created as an explicit extractor/guard type in the rebuild, not as scattered `if !user.is_admin && !user.is_lead_scout` lines.

---

## 5. Feature behavior

### 5.1 Event selection

The user picks an event once; it is stored on the session and every other page reads it.

- Events offered to a user = **their team's events** (`event_teams` joined through `teams.team_number`), or **all events** if the user has no team number.
- `POST /api/events/select` writes `sessions.selected_event_id` and re-renders the selector plus the summary fragment.
- The event summary shows event name, team count, match count, and the full team roster, plus a warning — *"Your team is not listed for this event yet."* — if the viewer's team is absent from `event_teams`.

### 5.2 Scouting submission

The form is pre-filled from the scout's **next unplayed match assignment**:

```sql
SELECT sa.team_id, teams.team_number, teams.name, m.event_id, m.match_number
FROM scout_assignments sa
JOIN matches m ON m.id = sa.match_id
JOIN teams   ON teams.id = sa.team_id
LEFT JOIN devices ON devices.id = sa.device_id
WHERE m.event_id = ?
  AND m.played = FALSE
  AND (sa.scouter_id = ? OR devices.device_uuid = ?)
ORDER BY m.match_number ASC
```

Note the `OR` — an assignment matches either the signed-in user **or this physical device**. The first row's team pre-selects the team dropdown.

**The form is pre-filled but not locked.** A scout can override the team selection freely and submit for the wrong robot. Closing this gap is the highest-leverage item in the refurbish plan.

Required fields: `event_id`, `team_id`, `alliance_color`, `starting_position`. Everything else is optional. All enumerated values are lowercased and trimmed on write; free-text fields (`notes`, `defendability`) are trimmed only.

`submitting_team_id` is resolved from the scouter's `team_number` at write time. **This field drives the notes privacy rule** (§5.4) — if it is null, the scout's own teammates cannot see their notes. Migration 0015 exists solely to backfill it after this was missed for two migrations.

On submit, a row is inserted into `scouting_submissions` with `scouted_at = now`. Nothing is written to `scouting_data` yet.

### 5.3 Review pipeline

1. Lead scout opens `/lead-scout` and sees all pending submissions ordered by `created_at`, each flagged **"Missing notes"** (yellow) or **"Clean"** (teal) based on whether `notes` is blank.
2. **Approve** — inside one transaction: copy every field into `scouting_data`, then `DELETE` the submission row. If `submitting_team_id` is null, resolve it from the scouter's team first. Afterwards, kick off a background FIRST sync for that team.
3. **Decline** — `DELETE` the submission row. **The data is destroyed with no record.** A scout gets no feedback and no chance to correct. Fix this: retract, don't delete.

### 5.4 Team profile

`/teams?team=<number>` renders team identity, then a per-event data panel:

- **Synced stats** from `team_event_stats`: rank, W-L-T record, matches played, qual average, average match points, DQ count, OPR/DPR/CCWM, component OPRs, and the point families. Formatted to 1 or 2 decimals; absent values render as empty strings, not zeros.
- **Scouting aggregates** — the *modal* (most common) value across all `scouting_data` rows for that team+event, for: starting position, defense rating, traversal, scoring strategy, hang level, hang position, accuracy rating.
- **Latest-row fields** — shooting speed, capacity, defendability, and scoring strategy come from the single most recent row, not the mode. This inconsistency is not principled; it is an accident.
- **Recent alliances** — the alliance colors of the last 5 scouting rows.
- **Notes** — filtered to `submitting_team_id == viewer's team id`. **Cross-team note visibility is deliberately blocked.** A viewer with no team sees no notes at all.

If a team has no local events, the page triggers a synchronous FIRST sync for that team and retries — a request that can block for many seconds. Do not repeat that pattern.

### 5.5 Ranking score

A configurable weighted sum over the enumerated scouting fields. Defaults:

| Metric | Options and points |
| --- | --- |
| `defense_rating` | high 5, mid 3, low 1 |
| `traversal` | trench 3, bump 2 |
| `shooting_speed` | fast 4, medium 2, slow 1 |
| `capacity` | high 4, medium 2, low 1 |
| `scoring_strategy` | scoring 4, defending 3, passing 2 |
| `hang_level` | none 0, l1 2, l2 4, l3 6 |
| `auto_hang` | yes 3, no 0 |
| `hang_position` | left 1, center 2, right 1 |
| `starting_position` | left 1, center 2, right 1 |

Rows in `scouting_point_weights` **override** defaults per `(metric_key, option_key)`; unknown metrics in the table are ignored, so a stale row cannot introduce a phantom metric. A team's score is the sum over **every** one of its `scouting_data` rows (not an average), so a heavily-scouted team outranks a lightly-scouted one purely on volume. That is a real bug — display `n=` alongside the score, or average.

Lead-scout weight editing accepts integers in `[-100, 100]`; any invalid value rejects the whole form. Form fields are named `weight_{metric_key}__{option_key}`.

Ranking table sorts: `rank` (default, nulls last), `points` (desc), `number`, `name`, each tie-broken by team number then name.

### 5.6 Assignments

The grid is matches (rows) × six robot slots (columns), for the selected event. Slot team numbers come from `matches.red1..blue3`; team names are resolved through a `team_number → team` lookup built from `event_teams`, falling back to `"TBD"` when the team is not in the local roster.

- **Set** — `POST /hx/assignments/set` with `match_id`, `team_id`, and `assignee`. The assignee is `"u:<user_id>"` or `"d:<device_id>"`; an empty string deletes the assignment. Upsert on `(match_id, team_id)`.
- **Auto-distribute** — takes checked assignees, or, if none were checked, every online scout (unexpired session) plus every online device (heartbeat within 3 min). Finds all unassigned slots in unplayed matches and round-robins the pool across them (`pool[i % pool.len()]`). Purely positional: no fairness tracking, no history, no per-scout load balancing.
- **Clear all** (event) and **clear match** delete assignments outright.
- **Rename device** sets a friendly name; the default display name is `"Device " || substr(device_uuid, 1, 8)`.

Every mutation responds with the **entire re-rendered assignment table**. On a 80-match event that is a large payload for every single click. The refurbish plan replaces this with SSE push (item A2/S9).

### 5.7 Drive coach

Reads the **FIRST match schedule live over the network**, filtered to the coach's team number — it does not read the local `matches` table. This means the drive coach panel is **completely non-functional offline**, at exactly the event where it matters most. It is the clearest example of a feature that a client-centred rebuild must invert.

Match status is derived from scheduled start time vs now:

| Condition | Status |
| --- | --- |
| more than 15 min in the past | Completed |
| within ±15 min | **Current Match** |
| more than 15 min in the future | Upcoming |

OPR/DPR for every team on the schedule is joined in from the local `team_event_stats`. The coach's own alliance and partners are identified by matching `station` prefix (`"Red"` / `"Blue"`) against the user's team number.

### 5.8 Pick list

Per-owning-team, per-event ordered list of picked teams with a color tag, a crossed-off flag, and a position. Requires the user to have a team number and a selected event. Read/upsert/delete over JSON. Reordering is client-side; `position` is whatever the client sends.

**No concurrency control whatsoever.** Two lead scouts reordering simultaneously silently clobber each other. This is the one place in the app where a CRDT genuinely earns its keep (refurbish item O14, `yrs`).

---

## 6. Upstream integrations

Detailed endpoint catalogs and payload examples are in [FRC_API_Calls.md](FRC_API_Calls.md) and [TBA_SCHEMA_FIX_SUMMARY.md](TBA_SCHEMA_FIX_SUMMARY.md). What follows is the behavior around them.

### 6.1 FIRST Events API

Base URL `https://frc-api.firstinspires.org/v3.0`, HTTP Basic auth (`FIRST_API_USERNAME` : `FIRST_API_KEY`).

| Call | Path | Notes |
| --- | --- | --- |
| Season events | `/{season}/events` | Optional `eventCode`, `teamNumber` filters |
| Event teams | `/{season}/teams?eventCode=` | |
| Match schedule | `/{season}/schedule/{event_code}` | Optional `teamNumber` or `tournamentLevel=Qualification` |

Note the response envelopes differ in casing: events come back under `"Events"`, schedule under `"Schedule"`, but teams under lowercase `"teams"`.

**Sync behavior:**
- Runs at boot unless `FIRST_SYNC_ON_BOOT=false`, with a 60-second timeout. Also available on demand at `POST /api/frc/sync` (90s timeout).
- Filter precedence: if neither `FIRST_EVENT_CODE` nor `FIRST_TEAM_NUMBER` is set, results are filtered client-side by `FIRST_COUNTRY` (default `"USA"`).
- Also runs **per-team on signup and login** (detached task, 60s timeout) for users with a team number, then chains a background TBA stats sync for that team's events.

**Upserts** (all "select first, then update or insert" because neither `events.tba_key` nor `teams.team_number` had a unique constraint — fix that and use real upserts):
- `events` keyed on derived `tba_key = "{start_year}{event_code_lowercase}"`. `location` is `venue, city, stateprov, country` joined with `", "`, skipping blanks.
- `teams` keyed on `team_number`. `tba_key` is set to `"frc{team_number}"`.
- `event_teams` inserted with `ON CONFLICT DO NOTHING`.

Event dates are parsed leniently: `%Y-%m-%d`, then `%Y-%m-%dT%H:%M:%S`, then RFC3339.

### 6.2 The Blue Alliance

Base URL `https://www.thebluealliance.com/api/v3`, header `X-TBA-Auth-Key`.

| Call | Path |
| --- | --- |
| OPRs | `/event/{key}/oprs` |
| Component OPRs | `/event/{key}/coprs` |
| Rankings | `/event/{key}/rankings` |
| Matches | `/event/{key}/matches` |

Event keys are normalized to `"{season}{code}"` if not already prefixed. Team keys are `"frc{number}"`.

**Background sync loop** (only starts if `TBA_AUTH_KEY` is set):
- Every **2 minutes** when any event has `start_date <= today <= end_date`, or when an event starts within 24 hours.
- Every **3 hours** otherwise.
- Each pass has a 120-second timeout. If no event is currently active, it falls back to events within ±7 days.
- Per event: sync `team_event_stats`, then sync `matches`. Component OPRs are treated as non-critical — a failure there logs and continues with nulls.

**Match upsert derivations:**
- `comp_level` defaults to `"qm"` when blank; `match_type` is written with the same value.
- `winning_alliance` is `"red"` / `"blue"` / `""`, decided only when both scores are `>= 0`.
- `played` is true if `actual_time > 0`, **or** the score breakdown is non-null and both scores are `>= 0`.
- `red1..blue3` are parsed from TBA team keys by stripping the `frc` prefix.
- Unix timestamps of `0` or less become `NULL`, not epoch.

**TBA schema variance is real and recurring.** Ranking and points fields move between seasons; the retired code carried fallback extraction (`effective_qual_average`, `effective_total_points`, `effective_qual_points`, `effective_avg_match_points`) that tried several field shapes. Read `TBA_SCHEMA_FIX_SUMMARY.md` before writing the deserializers — that document exists because this cost real debugging time.

### 6.3 HTTP client conventions (both APIs)

- **3 attempts max.** Retry only on HTTP 429 and 5xx, or on a transport error. Backoff: 250 ms, 500 ms, then 1 s.
- Error bodies are truncated to 4096 bytes before logging.
- rustls, not OpenSSL, so the release binary has no system TLS dependency.
- Every call records success or failure into the connectivity tracker (§7).

### 6.4 Connectivity tracking

A process-global snapshot: `checked_at`, `internet_reachable`, `internet_error`, `last_api_success_at`, `last_api_error_at`, `last_api_error`, `last_successful_sync`.

- The probe is a **raw TCP connect to `1.1.1.1:443` with a 1500 ms timeout**, cached for 3 seconds. Not an HTTP request — deliberately cheap and DNS-free.
- Probes are **skipped entirely** for localhost, loopback, and RFC1918 / link-local addresses (`10.*`, `172.16–31.*`, `192.168.*`, `169.254.*`), so a LAN-local API mock does not trip the offline path.
- A successful API call *proves* connectivity and back-fills the probe state.

Status classification, in order:
1. An API success within the last 10 minutes that is newer than the last error → **`internet-ok`** ("Internet OK", teal).
2. Not internet-reachable → **`offline`** ("Offline", red).
3. An API error newer than the last success → **`api-error`** ("API Error", amber).
4. Otherwise → `internet-ok`.

This three-state model is the ancestor of the four-state connection chip in refurbish item O11. Note that it describes the *server's* internet, not the *client's* connection to the server — a distinction that confused users and is the reason "offline mode" language needs to disappear.

---

## 7. Front-end conventions

### Rendering model

Server-rendered HTML, no SPA, no client router, no build-time JS framework. Full pages extend one layout; interactive regions are HTML fragments fetched over HTTP and swapped in place. **Unpoly** does the swapping (migrated from HTMX; the `/hx/` route prefix is a fossil of that).

The retired app used Askama for templates — compile-time-checked, so a malformed template is a build error rather than a runtime 500. Keep that property; it is worth more than it costs on a team of students.

**Askama gotchas encountered:** loop-variable fields are passed by reference, so comparison helpers must take `&i32` rather than `i32`. Entity structs mapped the full schema, so `#![allow(dead_code)]` was needed at crate level.

### The Unpoly glue layer

Unpoly extracts the target selector *from the response*, but several endpoints returned bare inner fragments. A ~70-line glue script bridged three gaps. **Re-create these three contracts or explicitly design them away:**

1. **Server-driven navigation.** The server responds with header `X-Up-Events: [{"type":"tt:navigate","url":"/"}]` plus `X-Up-Target: :none`; the client listens for `tt:navigate` and calls `window.location.assign(url)`. This is the Unpoly analog of HTMX's `HX-Redirect`, used after login, signup, logout, and submission approve/decline.

2. **`[tt-src]` — self-loading and polling regions.** Attributes: `tt-src` (URL), `tt-load` (fetch immediately), `tt-poll="<ms>"` (interval), `tt-on="evtA,evtB"` (refetch on those events). Fetches, sets `innerHTML`, calls `up.hello()` to recompile. Guards against overlapping requests with a `busy` flag and cleans up its timer and subscriptions on destroy.

3. **`[tt-change]` — change-driven fragment render.** `<select tt-change="/path?event_id={value}" tt-target="#foo" tt-load>` renders the target through Unpoly whenever the select changes, substituting `{value}` with the URL-encoded current value.

Because Unpoly does **not** execute `<script>` inside swapped fragments, forms that need a post-swap action use `up-on-inserted` inline (e.g. resetting the change-password form on success).

### Response conventions

- Handlers check `X-Up-Version` on the request to decide between a fragment response and a full redirect. Preserve this dual-mode behavior — it is what keeps the app working with JavaScript disabled or Unpoly failing to load.
- Fragment responses must have a **root element whose id matches the `up-target`**, e.g. `<div id="form-response">…</div>`, or the swap silently does nothing.
- Error fragments were built as inline format strings in Rust with `html_escape::encode_text` on the message. That was expedient and wrong — put them in templates.

### Styling

Tailwind CSS 3, dark theme only (`<html class="dark">`, body `bg-gradient-to-b from-gray-950 via-gray-900 to-gray-950`), with a custom `teal` palette (50 `#f0fdfa` → 900 `#134e4a`) as the brand accent. Tailwind scanned templates and handler source files, since class names appeared in Rust strings.

The component layer defined with `@apply`, worth re-creating:

`.btn` + `.btn-primary` / `.btn-secondary` / `.btn-danger` / `.btn-accent-red` / `.btn-accent-yellow` / `.btn-sm` / `.btn-lg` · `.card` / `.card-header` (+`.accent-red`, `.accent-yellow`) / `.card-body` / `.card-footer` · `.form-label` / `.form-input` / `.form-select` / `.form-checkbox` / `.form-error` · `.nav-link` / `.nav-link-active` · `.data-table` (with `thead` / `th` / `td` / `tbody tr` rules) · `.alert` + `.alert-success` / `.alert-error` / `.alert-warning` / `.alert-info` · `.badge` + `.badge-gray` / `.badge-teal` / `.badge-red` / `.badge-yellow` · `.loading-spinner` · `.border-accent-teal` / `.border-accent-red` / `.border-accent-yellow`

**Unpoly and Tailwind output were vendored locally**, never CDN-loaded, so the UI works on an event LAN with no internet. Keep this absolutely — it is not an optimization, it is a hard requirement.

### Navigation

Global chrome carried: Home, Scouting Submission, Lead Scout Panel (lead/admin only), Drive Coach Panel (coach/admin only), Help, a team-number search box, and Account/Logout or Sign In. Mobile collapsed to a hamburger menu. The refurbish plan replaces this with bottom navigation and 44px touch targets (item U10).

---

## 8. Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | `postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable` | Connection string |
| `PORT` | `8080` | Listen port; binds `0.0.0.0` |
| `TEALTEAM_ENV` | **`test`** | `test` resets migration history on boot; `prod` does not. Any other value is a fatal error. **Invert this default in the rebuild.** |
| `FIRST_API_USERNAME` | — | FIRST Events API basic-auth user |
| `FIRST_API_KEY` | — | FIRST Events API basic-auth key |
| `FIRST_SEASON` | `2026` | Season year for all FIRST calls |
| `FIRST_SYNC_ON_BOOT` | enabled | Set to `false` to skip the boot sync |
| `FIRST_EVENT_CODE` | — | Optional sync filter |
| `FIRST_TEAM_NUMBER` | — | Optional sync filter |
| `FIRST_COUNTRY` | `USA` | Client-side filter, applied only when neither filter above is set |
| `TBA_AUTH_KEY` | — | Enables the background stats sync loop; absent disables it entirely |

`.env` was loaded from the app directory then the repo root, with existing environment variables always winning.

**Startup sequence:**
1. Load `.env`, initialize tracing (`info,sqlx=warn` default filter).
2. Validate `TEALTEAM_ENV`, resolve `PORT` and `DATABASE_URL`.
3. Connect the pool **lazily** (max 25 connections, 10s acquire timeout).
4. Probe with `SELECT 1`. **If the database is unreachable, log a warning and start anyway** — DB-backed pages degrade rather than the whole server failing to boot. At an event, a server that boots half-working beats a server that does not boot.
5. If reachable: optionally reset migration history, apply migrations, run the FIRST boot sync.
6. Spawn the TBA background sync task.
7. Build the router, mount `/static`, add request tracing, and serve.

---

## 9. Crate layout for the rebuild

The retired Rust port was a single binary crate: `main.rs` + 10 modules + `handlers/` with one module per controller. Every SQL statement lived inline in a handler. That is exactly what blocked the client-centred direction.

**Build the ports-and-adapters split on day one** — retrofitting it was scored XL (the largest single item) in the refurbish plan. Target workspace:

```
tt-core/        pure domain: entities, scoring, aggregation, season schema,
                match-status classification, connectivity classification.
                NO sqlx, NO axum, NO tokio-net. Must compile to wasm32.
tt-templates/   Askama templates + view models. Must compile to wasm32.
tt-repo/        the `Repo` trait (use `trait-variant` to generate Send and
                non-Send variants — wasm futures are not Send).
tt-repo-sqlite/ server-side implementation.
tt-client/      wasm: browser-side repo over SQLite-WASM/OPFS, outbox, sync.
tt-web/         axum binary: routing, extractors, auth guards, static serving.
```

**Add a CI job that builds `tt-core` and `tt-templates` for `wasm32-unknown-unknown` from the very first commit.** It is nearly free and it is the only thing that keeps the boundary honest — every server-only dependency that leaks into the core layer breaks the build immediately instead of six months later.

Askama compiles to wasm, so the entire render layer crosses for free. That is why the refurbish plan rejects a Leptos/Yew SPA rewrite: the existing server-rendered UI can be served by wasm handlers behind a Service Worker without a framework or a client router.

---

## 10. Deployment target

Single Raspberry Pi 5 at the event. No cloud tier — Render is retired along with the Go and .NET ports, and Postgres with it.

The retired Pi setup used a two-container docker-compose (Postgres + the app) with data on a host-mounted volume, plus shell scripts for first boot, autostart, and showing the Pi's IP on an LCD. Those scripts are deleted; they were written against the Go binary and the Postgres container.

**Rebuild requirements** (details in `RefurbishInstructions.md` §1):
- SQLite in WAL mode with a single writer, on NVMe or USB SSD — **not** the SD card.
- A **DS3231 RTC module** (~$5). Without it, a power cycle at a venue with no internet gives you a server whose clock is wrong by hours, which silently corrupts every timestamp-ordered thing in the design. Do this before anything else.
- Avahi so clients reach `http://tealteam.local` instead of memorizing an IP.
- Wired Ethernet to clients; USB tethering for the uplink. **No Wi-Fi access point** — it violates FRC rule E143.
- Backups on a timer to the SSD, copied to a USB stick between match blocks.

Static assets and the binary must be co-locatable: the retired binary resolved `static/` and `migrations/` by walking up from both its own location and the working directory, so it worked under `cargo run` and from a bare `target/release/` deploy alike. Keep that, or embed the assets in the binary outright.

---

## 11. Testing and verification

The retired tree had almost no tests — a single TypeScript unit test for pick-list ordering, and a `go test ./...` step inside the Docker build. **That is the main reason this rebuild is being done from a specification rather than by refactoring.**

Minimum for the rebuild:

- Unit tests on `tt-core` for scoring, aggregation (mode calculation), match-status classification, connectivity classification, match-number normalization, and TBA field-fallback extraction. All of these are pure functions with real edge cases and all of them had bugs.
- Deserialization tests against **recorded** TBA and FIRST payloads, including at least one payload from a previous season, since upstream schema drift is the recurring failure mode.
- Round-trip tests on the repo trait, run against both implementations, so the server and browser adapters cannot diverge.
- A load test before the season: 30 simulated clients on realistic submit-and-sync cycles for two hours, measuring p95 latency and connection stability, with the cable pulled and the power killed mid-run deliberately.

---

## 12. Do not repeat these

Collected defects and design mistakes, so the rebuild does not inherit them.

### 12.1 Season coupling
`scouting_data`, `scouting_submissions`, and ~38 columns of `matches` are hardcoded to the 2022 Rapid React game. The match score-breakdown columns were never written by anything. Every January this schema is wrong. **Fix:** season-schema-driven JSON payload with a `schema_version`, per refurbish items U1–U3.

### 12.2 Scouting data has no match reference
`scouting_data` records `event_id` and `team_id` but **not `match_id`**, even though assignments are per-match and the form knows the match number. Consequences: you cannot tell two observations of the same robot apart, cannot detect duplicate or missing coverage, cannot correlate an observation with the match result, and "matches scouted" is inferred from row count. **Add `match_id` from day one.**

### 12.3 Dead schema
`awards` and `zebra_data` are created and never written. `scouting_submissions.status` and `rejection_reason` exist for a rejection workflow that does not exist — approve and decline both delete the row. `users.role` is vestigial next to the boolean flags. Either wire them up or leave them out.

### 12.4 The database viewer is unguarded
`GET /development/db` and `/hx/development/db/table/:name` browse **arbitrary tables** with paging. The handler masks `users.password_hash` and is grouped under tabs, but — unlike every other guarded route in the app — **it does not check `is_admin`**. Session rows, every user's email, and all scouting data are readable by any visitor who knows the URL. If a viewer is rebuilt at all, guard it, and do not expose `sessions`.

### 12.5 Decline destroys data silently
No audit trail, no notification, no correction path for the scout. Retract-not-delete, and show the scout what happened.

### 12.6 The drive coach panel requires the internet
It fetches the schedule live from the FIRST API instead of reading the local `matches` table that the background sync already populates. It is broken at exactly the moment it is needed. Read locally; sync separately.

### 12.7 Synchronous upstream calls inside page renders
`/teams` triggers a blocking FIRST sync when a team has no local events. `/submission/event-teams` falls back to a live FIRST call when the local roster is empty. Both make a page render depend on the internet. Never let a render block on a network call.

### 12.8 Full-table re-renders for single-cell edits
Every assignment mutation returns the whole grid. Use targeted fragments plus SSE push.

### 12.9 Ranking score sums instead of averaging
Teams scouted more often score higher for that reason alone. Show `n=`, and average.

### 12.10 The dangerous default
`TEALTEAM_ENV` defaults to `test`, and `test` drops `schema_migrations` on boot while `0001_init.sql` opens with `DROP TABLE ... CASCADE`. One missing environment variable in production erases everything. **Default to safe; require an explicit flag to reset.**

### 12.11 No unique constraints where upserts happen
`events.tba_key` and `teams.team_number` have indexes but no unique constraints, forcing hand-rolled select-then-insert-or-update in place of real upserts — which is also a race. Add the constraints and use `ON CONFLICT`.

### 12.12 Session-stored event selection
Putting `selected_event_id` on the server session means every page render needs a session read, no page is bookmarkable or shareable, two tabs cannot view two events, and nothing works offline. The refurbish plan moves this to a persistent header switcher (item U9). Make the event part of the client's own state.

### 12.13 "Online" for users means "has an unexpired session"
That is true for 24 hours after someone closes the tab, so auto-distribute happily assigns robots to people who went home. Devices already heartbeat correctly — use heartbeats for users too.

---

## 13. Documents that survived

| Document | Why it survived |
| --- | --- |
| [FRC_API_Calls.md](FRC_API_Calls.md) | Endpoint catalog and payload examples for FIRST and TBA. Upstream, not ours — still true. |
| [TBA_SCHEMA_FIX_SUMMARY.md](TBA_SCHEMA_FIX_SUMMARY.md) | Hard-won knowledge about TBA schema variance across seasons. Read before writing deserializers. |
| [TIMEZONE_HANDLING.md](TIMEZONE_HANDLING.md) | Event-timezone rules. Store UTC, render in the event's IANA zone. |
| [DataPoints.md](DataPoints.md) | What gets collected and what gets derived, in domain terms. |
| [TEAM_STATS_DISPLAY.md](TEAM_STATS_DISPLAY.md) | What the team page shows and why each number matters to a scout. |
| [MATCH_DETECTION.md](MATCH_DETECTION.md) | Match-source flow sketch. |
| [PREDICTIONS_REIMPLEMENTATION.md](PREDICTIONS_REIMPLEMENTATION.md) | The removed OPR/DPR score-prediction feature and its formulas, kept for reimplementation. File paths in it refer to deleted code; the math does not. |
| [TEALTEAM_DETAILED_OVERVIEW_SOURCE.md](TEALTEAM_DETAILED_OVERVIEW_SOURCE.md) | Dense domain synthesis — purpose, users, outcomes. Its stack and deployment sections describe the retired implementation; ignore those. |

Deleted as server-architecture-specific or as descriptions of code that no longer exists: `ARCHITECTURE.md`, `ARCHITECTURE_DIAGRAM.md`, `PI_EVENT_BOOT.md`, `SIGNUP_DATA_SYNC.md`, `TEAM_PAGE_ANALYSIS.md`, `TEAM_STATS_SYNC.md`, `AUTO_PATH_REMOVAL_RECORD.md`. Their surviving content — sync cadence, upsert rules, the auth-time team sync, the team-page flow — is in §5 and §6 above.

---

## 14. Recovering the retired code

Nothing is lost. The last commit containing all three implementations is on `main`, immediately before the retirement commit:

```sh
git log --oneline --all              # find the retirement commit
git show <retirement-commit>~1 --stat
git show <retirement-commit>~1:rust/tealteam-web/src/handlers/assignments.rs
git checkout <retirement-commit>~1 -- rust/tealteam-web   # restore a subtree
```

Useful paths in that tree: `rust/tealteam-web/src/` (the most complete port), `migrations/*.sql` (the full schema), `web/tailwind/input.css` (the design system), `rust/tealteam-web/static/js/` (`device.js`, `tt-unpoly.js`), `dotnet/TealTeam.Web/` and `cmd/` + `internal/` (the other two ports).
