# Auto Path Feature Retirement Record

## Summary

The historical auto-path scouting feature is retired in the current codebase.

This means:

- No active auto-path capture inputs in current scouting flow
- No active auto-path rendering blocks in current team display flow
- No active, dedicated auto-path migration file in the present migration chain

## Current Repository State

- Active migrations are `0001_init.sql` and `0005` through `0012`.
- Auto-path schema is not part of the currently deployed schema surface.
- Team scouting focuses on the currently captured qualitative fields plus TBA/FIRST synced metrics.

## If Reintroducing Auto Path in the Future

1. Add explicit columns and tables in a new migration file.
2. Add form input handling in `internal/handlers/submission.go`.
3. Add review handling in lead-scout workflow handlers/templates.
4. Add display and aggregation logic in team data templates/handlers.
5. Update seed scripts under `cmd/scripts/seed/main.go` if test fixture support is needed.
6. Update docs (`DataPoints.md`, `TEAM_PAGE_ANALYSIS.md`, `README.md`) in the same change.

## Documentation Note

This file is kept as an archival marker only. It intentionally avoids old file/line migration references that are no longer present in the repository.
