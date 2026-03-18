# Match Detection Notes

## Scope

This note describes how match and event context is currently determined in handler logic.

## Event Filtering

Submission and team workflows filter event visibility by user context:

- Team users see events available to their team through event-team associations.
- Admin and non-team contexts can access broader event sets as allowed by handler logic.

Key implementation points:

- `buildSubmissionPageData()` in `internal/handlers/submission.go`
- `GetAvailableEventsForUser(...)` helper usage in handlers

## Team Match Context

Team and coach views rely on synced match data from `matches` plus team relationships.

- Match schedule and results are synced by the TBA background syncer.
- Match rows are keyed by `(event_id, match_number, match_type)` and updated on each sync.

## Status Windows

Coach/drive-coach displays classify matches with time-window logic around scheduled times.

Typical categories:

- completed
- current/in-progress
- upcoming

Implementation references:

- `internal/handlers/coach.go`
- `internal/handlers/drive_coach.go`
- `internal/handlers/matches.go`

## Data Sources

- FIRST API sync: events and event-team registration graph
- TBA sync: rankings/stats and match schedule/results
- Local DB tables: `events`, `event_teams`, `matches`, `team_event_stats`, scouting tables

## Operational Dependencies

- `FIRST_API_USERNAME` and `FIRST_API_KEY` enable FIRST sync paths.
- `TBA_AUTH_KEY` enables periodic match/stat sync from TBA.
- Without these keys, handlers still run but coverage degrades to available local data.
