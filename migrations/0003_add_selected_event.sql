-- Migration: 0003_add_selected_event.sql
-- Description: Store selected event per session

ALTER TABLE sessions
ADD COLUMN IF NOT EXISTS selected_event_id INTEGER REFERENCES events(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_selected_event ON sessions(selected_event_id);
