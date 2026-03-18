# TealTeam UI Communication Flow

This diagram focuses on how browser interactions move through the Gin server, data layer, and external APIs.

```mermaid
flowchart LR
  U[Users\nScout, Lead Scout, Coach, Admin]

  subgraph CLIENT[Client]
    B[Browser]
    P[Full Pages\n/, /submission, /teams, /lead-scout, /drive-coach, /account]
    H[HTMX Requests\n/hx/events/summary\n/hx/teams/search\n/hx/teams/data\n/hx/matches/schedule]
  end

  subgraph SERVER[Go Service - Gin]
    R[Router + Middleware\nrequest logging, recovery, session checks]
    PH[Page Handlers]
    AH[API Handlers\n/api/auth/*\n/api/events/select\n/api/frc/sync]
    SVC[Service Layer\nauth, scouting, FIRST sync, TBA sync]
  end

  subgraph DB[PostgreSQL]
    CORE[(users, sessions)]
    OPS[(events, teams, event_teams, matches)]
    SCOUT[(scouting_submissions, scouting_data)]
    STATS[(team_event_stats, awards, zebra_data)]
    MIG[(schema_migrations)]
  end

  subgraph EXT[External APIs]
    F[FIRST Events API]
    T[TBA API]
  end

  U --> B
  B --> P
  B --> H

  P --> R
  H --> R

  R --> PH
  R --> AH
  PH --> SVC
  AH --> SVC

  SVC --> CORE
  SVC --> OPS
  SVC --> SCOUT
  SVC --> STATS
  SVC --> MIG

  SVC --> F
  SVC --> T
```

## Runtime Bootstrap

```text
cmd/web/main.go
  -> resolve env mode and DATABASE_URL
  -> db.Connect()
  -> ApplyMigrations(migrations/)
  -> SyncOnBoot() unless FIRST_SYNC_ON_BOOT=false
  -> Start TeamStatsSyncer() when TBA_AUTH_KEY is set
  -> serve routes
```

## Notes

- Full pages return complete HTML with layout.
- HTMX routes return partial HTML fragments.
- `POST /api/frc/sync` exists and is available to admin or lead scout sessions.
- Background TBA sync runs at two cadences:
  - 2 minutes during active event windows
  - 3 hours between events
- Migrations are tracked in `schema_migrations`.
