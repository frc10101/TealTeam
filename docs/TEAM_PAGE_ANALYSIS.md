# Team Page Analysis

```mermaid
flowchart LR
   User --> Search[HandleTeamSearch]
   Search --> GetEvents[GetEventsForTeam]
   GetEvents -->|none| FIRSTSync[frc.SyncTeamForUser]
   GetEvents --> Render[Render partials (team_info, team_data)]
```

## Scope

This document describes how the `/teams` experience works in the current codebase.

Primary files:

- `internal/handlers/team.go`
- `web/templates/pages/team.html`
- `web/templates/partials/team_info.html`
- `web/templates/partials/team_data.html`

## Route Surface

Full page:

- `GET /teams` -> `HandleTeamPage`

HTMX fragments:

- `GET /hx/teams/search` -> `HandleTeamSearch`
- `GET /hx/teams/data` -> `HandleTeamEventData`

## Request Flow

### Team lookup

1. User opens `/teams`.
2. User enters team number and submits search.
3. `HandleTeamSearch` resolves team from `teams` table.
4. Handler loads team events via `GetEventsForTeam`.
5. If no local events are found, handler triggers `frc.SyncTeamForUser(...)` and retries event lookup.
6. `team_info` partial is rendered.

### Event data panel

1. User selects event.
2. `HandleTeamEventData` resolves team and event IDs.
3. Handler loads:
   - `team_event_stats` row for `(team_id, event_id)`
   - all `scouting_data` rows for `(team_id, event_id)`
4. Aggregates are calculated (most common values, alliance color distribution).
5. Notes are privacy-filtered by `submitting_team_id` of viewer's team.
6. `team_data` partial is rendered.

## Data Displayed

From `team_event_stats`:

- Rank
- OPR, DPR, CCWM
- Auto/Teleop/Endgame OPR
- Matches played
- W-L-T and DQ count
- Qual average
- Average match points
- Qual/Elim/Award/Alliance/Total points

From `scouting_data` aggregates:

- Most common starting position
- Most common defense rating
- Most common traversal
- Most common scoring strategy
- Most common hang level
- Most common auto hang
- Most common hang position
- Most common accuracy rating
- Alliance color counts

## Privacy Rule

Competition notes are team-private:

- A viewer only sees notes submitted by their own team.
- If no viewer team is resolved, note list is empty.

## Sync Dependencies

- Team event presence is primarily sourced from FIRST sync (`event_teams`).
- Team statistics and matches are sourced from background TBA sync when `TBA_AUTH_KEY` is configured.

## Current Status

- Team page is active and backed by `internal/handlers/team.go`.
- Previous references to removed migration files (`0002`/`0003`/`0004` legacy names) are no longer applicable in this repository state.
- Current migration chain is documented in `README.md` and starts with `0001_init.sql` plus `0005`-`0012`.
