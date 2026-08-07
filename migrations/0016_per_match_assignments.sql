-- Migration: 0016_per_match_assignments.sql
-- Description: Changes scout_assignments from per-event-team to per-match-team
--              so the lead scout can assign scouts to specific robots for each
--              individual match. Also adds alliance team columns to matches so
--              the assignment UI knows who is in each slot without an API call.

-- Alliance robot slots on each match (stored as team numbers; resolved to
-- team rows in C# rather than FK so missing/unsynced teams don't block display)
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red1 INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red2 INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red3 INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue1 INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue2 INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue3 INTEGER;

-- Old per-event assignments are no longer valid under the new model.
DROP TABLE IF EXISTS scout_assignments CASCADE;

-- Per-match assignments: one assignment per robot per match.
CREATE TABLE scout_assignments (
    id         SERIAL PRIMARY KEY,
    match_id   INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id    INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    event_id   INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    scouter_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    device_id  INTEGER REFERENCES devices(id) ON DELETE CASCADE,
    assigned_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(match_id, team_id),
    CHECK (scouter_id IS NOT NULL OR device_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_scout_assignments_match   ON scout_assignments(match_id);
CREATE INDEX IF NOT EXISTS idx_scout_assignments_event   ON scout_assignments(event_id);
CREATE INDEX IF NOT EXISTS idx_scout_assignments_scouter ON scout_assignments(scouter_id);
CREATE INDEX IF NOT EXISTS idx_scout_assignments_device  ON scout_assignments(device_id);
