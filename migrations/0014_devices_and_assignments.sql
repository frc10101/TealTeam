-- Migration: 0014_devices_and_assignments.sql
-- Description: Registered scouting devices (permanent device IDs) and
--              lead-scout robot assignments used to pre-fill scouting forms.

-- ============================================================
-- DEVICES
-- ============================================================
-- A device registers itself with a persistent browser-generated UUID and
-- heartbeats while connected, so leads can assign robots to a physical
-- tablet regardless of who is signed in on it.
CREATE TABLE IF NOT EXISTS devices (
    id SERIAL PRIMARY KEY,
    device_uuid VARCHAR(64) UNIQUE NOT NULL,
    name VARCHAR(100),
    team_number INTEGER,
    last_seen_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen_at);

-- ============================================================
-- SCOUT ASSIGNMENTS
-- ============================================================
-- One assignment per robot per event. The assignee is either a user
-- (scouter_id) or a registered device (device_id).
CREATE TABLE IF NOT EXISTS scout_assignments (
    id SERIAL PRIMARY KEY,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    scouter_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    device_id INTEGER REFERENCES devices(id) ON DELETE CASCADE,
    assigned_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(event_id, team_id),
    CHECK (scouter_id IS NOT NULL OR device_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_scout_assignments_event ON scout_assignments(event_id);
CREATE INDEX IF NOT EXISTS idx_scout_assignments_scouter ON scout_assignments(scouter_id);
CREATE INDEX IF NOT EXISTS idx_scout_assignments_device ON scout_assignments(device_id);
