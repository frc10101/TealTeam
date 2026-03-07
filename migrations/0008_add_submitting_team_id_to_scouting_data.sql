-- Migration: 0008_add_submitting_team_id_to_scouting_data.sql
-- Description: Add submitting_team_id to scouting_data table for audit trail

ALTER TABLE scouting_data
ADD COLUMN submitting_team_id INTEGER REFERENCES teams(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_scouting_data_submitting_team ON scouting_data(submitting_team_id);
