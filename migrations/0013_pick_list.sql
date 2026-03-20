-- Migration: 0013_pick_list.sql
-- Description: Team-wide pick list persistence

-- ============================================================
-- PICK LIST ENTRIES
-- ============================================================
CREATE TABLE IF NOT EXISTS pick_list_entries (
    id SERIAL PRIMARY KEY,
    team_number INTEGER NOT NULL,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    picked_team_number INTEGER NOT NULL,
    color VARCHAR(50),
    crossed BOOLEAN DEFAULT FALSE,
    position INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(team_number, event_id, picked_team_number)
);

CREATE INDEX IF NOT EXISTS idx_pick_list_entries_team_event 
    ON pick_list_entries(team_number, event_id);
CREATE INDEX IF NOT EXISTS idx_pick_list_entries_event 
    ON pick_list_entries(event_id);

-- Trigger for updated_at
DROP TRIGGER IF EXISTS pick_list_entries_updated_at ON pick_list_entries;

CREATE TRIGGER pick_list_entries_updated_at
    BEFORE UPDATE ON pick_list_entries
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
