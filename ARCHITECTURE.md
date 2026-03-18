# TealTeam Architecture Overview

## System Summary

TealTeam is a server-rendered scouting platform for FRC teams. It combines manual scouting submissions with automated FIRST and TBA data sync, then serves team and event insights through HTML pages and HTMX fragments.

## High-Level Components

```text
Browser (pages + HTMX)
  -> Gin router and handlers
    -> Service logic (auth, sync, scoring, aggregation)
      -> PostgreSQL (events, teams, stats, submissions, sessions)
      -> External APIs (FIRST Events, The Blue Alliance)
```

## Runtime Startup Flow

1. `cmd/web/main.go` loads `.env`, parses `-env`, and resolves `DATABASE_URL`.
2. DB connection is established via `internal/db.Connect`.
3. SQL migrations are auto-applied from `migrations/` and recorded in `schema_migrations`.
4. FIRST sync runs on boot unless `FIRST_SYNC_ON_BOOT=false`.
5. TBA background sync starts if `TBA_AUTH_KEY` is configured.
6. Gin routes are registered for full pages, APIs, and HTMX fragments.

## Routing Model

Full page routes (examples):

- `/`
- `/submission`
- `/teams`
- `/lead-scout`
- `/drive-coach`
- `/account`
- `/sign-in`
- `/sign-up`

API routes:

- `/api/auth/login`
- `/api/auth/signup`
- `/api/auth/logout`
- `/api/account/change-password`
- `/api/events/select`
- `/api/frc/sync`

HTMX routes:

- `/hx/events/summary`
- `/hx/teams/search`
- `/hx/teams/data`
- `/hx/matches/schedule`
- `/hx/lead-scout/submissions/:id/approve`
- `/hx/lead-scout/submissions/:id/decline`

## Data Domains

Core relational domains:

- Identity: `users`, `sessions`
- Competition graph: `events`, `teams`, `event_teams`
- Performance sync: `team_event_stats`, `matches`, `awards`, `zebra_data`
- Scouting intake and review: `scouting_submissions`, `scouting_data`
- Operational metadata: `schema_migrations`

## FIRST Integration

Code path: `internal/frc/sync.go`, `internal/frc/first_api.go`

Behavior:

- Pulls season events and event team attendance from FIRST Events API
- Upserts `events`, `teams`, and `event_teams`
- Supports optional filtering with `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER`, and `FIRST_COUNTRY`
- Admin/lead scout can trigger manual sync via `POST /api/frc/sync`

## TBA Integration

Code path: `internal/frc/team_stats_sync.go`, `internal/frc/tba_client.go`

Behavior:

- Background sync loop cadence:
  - Every 2 minutes during active events
  - Every 3 hours between events
- For each active/recent event with a valid TBA key:
  - Sync rankings and OPR families into `team_event_stats`
  - Sync match schedule/results into `matches`
- Handles season schema variance with fallback extraction logic for ranking/points fields and component OPR payloads.

## Submission Review Pipeline

Code path: `internal/handlers/submission.go`, `internal/handlers/lead_scout.go`

Pipeline:

1. Scouts submit observations into `scouting_submissions`.
2. Lead scout reviews pending submissions.
3. Approve action copies normalized data into `scouting_data`.
4. Decline action removes the pending entry.
5. Legacy null/blank statuses are normalized by `migrations/0012_normalize_submission_status.sql`.

Privacy rule:

- Notes are scoped by submitting team (`submitting_team_id`) when rendered to team users.

## Auth and Team Bootstrapping

Code path: `internal/handlers/auth.go`, `internal/frc/sync.go`

- Signup/login creates session state and can trigger team-specific FIRST sync.
- Team-specific TBA sync is launched asynchronously for that team's events when credentials are available.

## Deployment Architecture (Render)

Source of truth: `render.yaml`

- Web service runs from the Docker image produced by `Dockerfile`
- Container command: `/server -env=prod`
- Managed PostgreSQL connection injected as `DATABASE_URL`
- Secret env vars are configured manually in Render dashboard:
  - `FIRST_API_USERNAME`
  - `FIRST_API_KEY`
  - `TBA_AUTH_KEY`

## Operational Notes

- The app can start even if DB is unavailable, but DB-backed pages and features will degrade.
- In `-env=test`, migration history table is reset before migration apply.
- Pi and offline event mode are documented in `docs/PI_EVENT_BOOT.md`.
