# TealTeam

TealTeam is an FRC scouting and analytics web app built with Go, Gin, server-rendered templates, HTMX, and PostgreSQL.

## Stack

- Backend: Go 1.24+, `gin-gonic/gin`
- Rendering: Go `html/template` + HTMX fragments
- Styling and client assets: Tailwind CSS + TypeScript
- Database: PostgreSQL (local Docker or Render managed Postgres)
- Deployment: Render (`render.yaml`, Docker runtime)

## Repository Layout

```text
cmd/
  web/main.go                    # App entrypoint, env mode, router bootstrap
  scripts/                       # One-off operational and sync scripts
internal/
  db/                            # DB connection and migration runner
  frc/                           # FIRST and TBA clients/sync logic
  handlers/                      # Page, API, and HTMX handlers
  logging/                       # slog setup + request/recovery middleware
  middleware/                    # Shared middleware package
  models/                        # DB model structs
migrations/                      # Ordered SQL migrations (0001, 0005-0012)
web/
  templates/                     # Full pages and partials
  static/                        # Built CSS/JS and static assets
  tailwind/input.css             # Tailwind source
docs/                            # Operational and architecture docs
render.yaml                      # Render Blueprint
docker-compose.yml               # Local dev stack
docker-compose.pi.yml            # Raspberry Pi event stack
```

## Quick Start

### Prerequisites

- Go 1.24+
- Node.js 18+
- Docker
- `psql` CLI (optional but useful for manual DB checks)

### 1. Install dependencies

```bash
npm install
go mod download
```

### 2. Start local services

```bash
docker-compose up -d db
```

### 3. Build frontend assets

```bash
npm run build
# or run watchers during development:
# npm run dev
```

### 4. Run the server

```bash
go run ./cmd/web
```

The default mode is `-env=test`, which targets local Postgres.

Open `http://localhost:8080`.

## Runtime Modes

The app supports:

- `-env=test` (default): local/test mode
- `-env=prod`: production mode

Examples:

```bash
go run ./cmd/web -env=test
go run ./cmd/web -env=prod
```

`cmd/web/main.go` resolves the database URL as follows:

- `test`: `DATABASE_URL` if set, otherwise local default `postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable`
- `prod`: `RENDER_DATABASE_URL` first, then `DATABASE_URL`

## Database and Migrations

- Migrations are auto-applied on startup by `internal/db.ApplyMigrations`.
- Applied files are tracked in `schema_migrations`.
- SQL files currently in use:
  - `migrations/0001_init.sql`
  - `migrations/0005_add_avg_match_points.sql`
  - `migrations/0006_add_event_timezone.sql`
  - `migrations/0007_add_submitting_team_id.sql`
  - `migrations/0008_add_submitting_team_id_to_scouting_data.sql`
  - `migrations/0009_add_accuracy_rating.sql`
  - `migrations/0010_add_submission_status.sql`
  - `migrations/0011_add_scouting_point_weights.sql`
  - `migrations/0012_normalize_submission_status.sql`

Note on `test` mode:

- On startup, `ResetMigrations` drops only `schema_migrations` before reapplying migrations.
- This does not wipe application tables but does force migration re-checking.

## Data Sync Behavior

### FIRST sync

- Runs at boot unless `FIRST_SYNC_ON_BOOT=false`
- Also available as an on-demand API call: `POST /api/frc/sync`
- Endpoint requires an authenticated admin or lead scout session

### TBA sync

- Background sync starts only when `TBA_AUTH_KEY` is configured
- Sync cadence:
  - Every 2 minutes during active events
  - Every 3 hours between events
- Syncs:
  - Team event stats (OPR/DPR/CCWM/component OPRs/rankings)
  - Match schedule and results

### Team-based sync on auth

- During signup/login, the app syncs FIRST data for the user team and asynchronously syncs TBA stats for that team's events.

## Core Routes

Full pages:

- `GET /`
- `GET /submission`
- `GET /teams`
- `GET /lead-scout`
- `GET /drive-coach`
- `GET /account`
- `GET /sign-in`
- `GET /sign-up`

API:

- `POST /api/auth/login`
- `POST /api/auth/signup`
- `POST /api/auth/logout`
- `POST /api/account/change-password`
- `POST /api/events/select`
- `POST /api/frc/sync`

HTMX fragments:

- `GET /hx/events/summary`
- `GET /hx/teams/search`
- `GET /hx/teams/data`
- `GET /hx/matches/schedule`
- `POST /hx/lead-scout/submissions/:id/approve`
- `POST /hx/lead-scout/submissions/:id/decline`

## Environment Variables

Required for database-backed operation:

- `DATABASE_URL`

FIRST API:

- `FIRST_API_USERNAME`
- `FIRST_API_KEY`
- `FIRST_SEASON` (default: `2026`)
- `FIRST_SYNC_ON_BOOT` (default behavior is enabled)
- `FIRST_EVENT_CODE` (optional filter)
- `FIRST_TEAM_NUMBER` (optional filter)
- `FIRST_COUNTRY` (optional filter, defaults to `USA` when no event/team filter is set)

TBA:

- `TBA_AUTH_KEY`

Server:

- `PORT` (default: `8080`)

## Local Docker Notes

- `docker-compose.yml` persists Postgres data to `${DB_DATA_PATH:-./.data/postgres}`.
- Avoid `docker-compose down -v` unless you intentionally want to remove local DB data.

## Render Deployment

Render is configured through `render.yaml`.

Current blueprint behavior:

- Docker runtime using `Dockerfile`
- Start command: `/server -env=prod`
- Managed Postgres bound into `DATABASE_URL`
- Required secrets left unsynced for manual setup (`FIRST_API_USERNAME`, `FIRST_API_KEY`, `TBA_AUTH_KEY`)
- FIRST defaults in blueprint:
  - `FIRST_SEASON=2026`
  - `FIRST_SYNC_ON_BOOT=true`

Deploy flow:

1. Push to `main`.
2. Render auto-deploys with the blueprint.
3. Set secret env vars in Render dashboard.
4. Verify app health on `/` and logs for migration/sync startup messages.

## Pi Event Mode

For Raspberry Pi event deployment and autostart setup, see `docs/PI_EVENT_BOOT.md`.

## Additional Docs

- `ARCHITECTURE.md`
- `docs/ARCHITECTURE_DIAGRAM.md`
- `docs/TIMEZONE_HANDLING.md`
- `SIGNUP_DATA_SYNC.md`
- `TEAM_STATS_SYNC.md`
- `TBA_SCHEMA_FIX_SUMMARY.md`
