# TealTeam Architecture Overview

## System Overview
TealTeam is a scouting and analytics platform for FRC (FIRST Robotics Competition) teams. It aggregates data from multiple sources and displays team performance metrics and scouting information.

```
┌─────────────────────────────────────────────────────────────────┐
│                        TealTeam Application                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Frontend (Web)                        │   │
│  │  - Go templates (HTML/CSS/JS)                           │   │
│  │  - Tailwind CSS for styling                             │   │
│  │  - HTMX for dynamic updates                             │   │
│  │  - TypeScript/vanilla JS for interactivity              │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              ↑                                    │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                  Go Web Server (Gin)                     │   │
│  │  - Routes: /team, /submission, /admin, /auth, etc      │   │
│  │  - Handlers in internal/handlers/                       │   │
│  │  - Middleware: auth, logging, CORS                      │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              ↑                                    │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                   Business Logic Layer                   │   │
│  │  ┌──────────────────────────────────────────────────┐   │   │
│  │  │  internal/frc/                                   │   │   │
│  │  │  - TBA Client: API calls to The Blue Alliance   │   │   │
│  │  │  - FIRST Client: API calls to FIRST Events API  │   │   │
│  │  │  - Team Stats Syncer: Background sync process   │   │   │
│  │  │  - Sync: Event/team syncing logic                │   │   │
│  │  └──────────────────────────────────────────────────┘   │   │
│  │  ┌──────────────────────────────────────────────────┐   │   │
│  │  │  internal/models/                               │   │   │
│  │  │  - Team, Event, Match, ScoutingData, User       │   │   │
│  │  │  - TeamEventStats (aggregated stats)            │   │   │
│  │  └──────────────────────────────────────────────────┘   │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              ↑                                    │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                  Data Access Layer (GORM)              │   │
│  │  - internal/db/                                        │   │
│  │  - Handles migrations and database connections         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              ↑                                    │
└──────────────────┬───────────────────────────────────────────────┘
                   │
         ┌─────────┴──────────┬──────────────┬──────────────┐
         │                    │              │              │
    ┌────▼────┐        ┌─────▼──────┐ ┌───▼────┐     ┌───▼────┐
    │PostgreSQL│        │The Blue    │ │FIRST   │     │Manual  │
    │Database  │        │Alliance API│ │Events  │     │Scouting│
    │          │        │(TBA)       │ │API     │     │Forms   │
    └──────────┘        └────────────┘ └────────┘     └────────┘
```

## Data Flow Architecture

### 1. **Data Integration Layer** (Multiple Sources)

#### A. FIRST API Integration (`internal/frc/first_api.go`)
- **Purpose**: Sync official FRC event and team participation data
- **Trigger**: 
  - On application boot (`SyncOnBoot()` in `cmd/web/main.go`)
  - Manual sync via `/frc-sync` endpoint
- **Flow**:
  ```
  FIRST API → SyncNow() → Parse Events/Teams → Store in DB
  ```
- **Data Synced**:
  - Event details (name, location, dates, dates)
  - Team participation (which teams are at which events)
  - Event type and district information

#### B. The Blue Alliance (TBA) API Integration (`internal/frc/tba_client.go`)
- **Purpose**: Sync detailed team performance statistics
- **Trigger**: 
  - Background syncer (starts automatically if `TBA_AUTH_KEY` is set)
  - Runs every 2 minutes during events, every 3 hours between events
- **Data Synced**:
  - OPR, DPR, CCWM (overall performance metrics)
  - Component OPRs (Auto, Teleop, Endgame)
  - Team rankings and records (Wins, Losses, Ties, DQs)
  - Match schedules and results
  - Component breakdowns (auto scores, etc.)
- **Flow**:
  ```
  Background Timer → Check Active Events → Fetch TBA Data → Update Stats
  ```

#### C. Manual Scouting Input
- **Purpose**: Scouts submit match observations via web forms
- **Trigger**: Manual form submission at `/submission`
- **Data Captured**:
  - Starting position, defense rating, traversal (Bump/Trench multi-select)
  - Hang capabilities, hang position, scoring strategy
  - Alliance color, match details
  - Accuracy rating (Low / Medium / High)
  - Notes and observations
- **Submission Lifecycle**:
  - Submissions are stored in `scouting_submissions` for lead scout review
  - A background goroutine cleans up submissions older than 20 minutes
  - On approval, data is copied to `scouting_data` with `submitting_team_id` for ownership tracking
- **Notes Privacy**: Notes are only visible to the team that submitted them (filtered by `submitting_team_id`)

### 2. **Data Storage & Models** (`internal/models/`)

```
┌────────────────────────────────────────────────────┐
│           Database Schema Overview                 │
├────────────────────────────────────────────────────┤
│                                                    │
│  teams                → event_teams ← events      │
│  (team info)          (many-to-many)  (event info)│
│        ↓                                    ↑      │
│        └──────────────────────────────────┘       │
│                      ↓                             │
│  team_event_stats (aggregated stats per event)    │
│  matches (match schedule & results)               │
│  scouting_data (manual observations)              │
│  users (scouts, coaches, leads)                   │
│  sessions (authentication)                        │
│                                                    │
└────────────────────────────────────────────────────┘
```

**Key Models**:
- `Team`: Basic team info (number, name, school, location)
- `Event`: Competition details
- `TeamEventStats`: **Aggregated performance metrics per team per event**
  - `MatchesPlayed` (calculated from: Wins + Losses + Ties)
  - OPR, DPR, CCWM, component OPRs
  - Ranking and record
  - Points (qual, elim, award, alliance)
- `Match`: Individual match information
- `ScoutingData`: Manual observations from scouts
  - Includes `accuracy_rating` and `submitting_team_id` fields
  - Notes are team-private (filtered by submitting team at display time)

### 3. **Display Layer** (`web/templates/` + handlers)

#### Team Page Flow
```
Request: /team?team=6328&event=123
         ↓
    find_team_from_number()
         ↓
    match with events they're at
         ↓
    fetch team_event_stats for selected event
    fetch scouting_data for that event
         ↓
    render team_data.html with:
    - TBA stats (OPR, ranking, matches)
    - Scouting observations (aggregated)
    - Component OPRs
    - Match history
```

#### Data Displayed:
- **From `team_event_stats`**: OPR, DPR, CCWM, Rank, MatchesPlayed, Record, QualAverage, Points
- **From `scouting_data`**: Starting positions, defense ratings, traversals, hang info, accuracy ratings
- **Calculated**: Most common values, defense breakdowns, hang statistics
- **Notes**: Displayed per-team — only notes submitted by the viewer's team are shown
- **Past Events**: "Fetch Past Events" button syncs historical event data from FIRST API

## Key Issues & How They're Fixed

### Issue: Match Count Discrepancy
- **Problem**: `team_event_stats.matches_played` was using TBA's `matches_played` field directly, which could be out of sync
- **Solution** (Fixed in `team_stats_sync.go`):
  ```go
  // OLD:
  stats.MatchesPlayed = ranking.MatchesPlayed
  
  // NEW:
  stats.MatchesPlayed = ranking.Record.Wins + ranking.Record.Losses + ranking.Record.Ties
  ```
- **Why**: The record (W-L-T) is always the source of truth; matches played = total matches in record

## Data Sources & Priority

| Field | Source | Priority | Fallback |
|-------|--------|----------|----------|
| Team info | FIRST API → TBA API | FIRST (official) | Manual entry |
| Events | FIRST API | Official | TBA event list |
| Rankings | TBA API | TBA (real-time) | Manual input |
| OPR/DPR/CCWM | TBA API | TBA | None (calculated) |
| Match Results | TBA API | TBA | Scouting input |
| Scouting Notes | Manual forms | User input (team-private) | Aggregated observations |
| Accuracy Rating | Manual forms | User input | Per-submission quality flag |

## Configuration & Environment Variables

```bash
# FIRST API (for event/team sync on boot)
FIRST_API_USERNAME=<username>
FIRST_API_KEY=<api_key>
FIRST_SEASON=2026              # Current season
FIRST_SYNC_ON_BOOT=true        # Sync on startup
FIRST_EVENT_CODE=NHNA          # Focus event
FIRST_TEAM_NUMBER=6328         # Focus team

# TBA API (for background stats sync)
TBA_AUTH_KEY=<auth_key>        # Required for team stats sync

# Database
DATABASE_URL=postgres://user:password@localhost:5432/db

# Server
PORT=8080
```

## Data Sync Timing

```
Application Start
    ├─ Apply migrations
    ├─ SyncOnBoot() [if FIRST_SYNC_ON_BOOT=true]
    │  └─ Sync FIRST API events & teams (one-time on boot)
    │
    └─ Start background TeamStatsSyncer [if TBA_AUTH_KEY set]
       └─ Loop every 2-3 minutes:
          ├─ Find active events
          ├─ For each active event:
          │  ├─ Sync team stats (OPR, DPR, rankings)
          │  └─ Sync match schedule & results
          └─ Sleep for interval
```

## Current Data Issue Resolution

**Problem**: System showing seeded test data (3 wins/0 losses) instead of real TBA data (9-1-0 record)

**Root Cause**: 
1. Application uses test/seeded events with fake TBA keys (e.g., "2026txho1")
2. These keys don't exist on real TBA API, so background sync finds no data
3. Seeded test data persists in database

**Solution Path**:
1. Identify real TBA event key for Week 0 Nashua, NH 2026 event
2. Either:
   - Update event records with correct TBA keys, OR
   - Manually sync specific events with correct TBA keys
3. Delete old seeded data or reset database with correct event configuration
