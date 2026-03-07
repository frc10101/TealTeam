-- Migration: 0007_add_submitting_team_id.sql
-- Description: Add submitting_team_id to track which team submitted each form

ALTER TABLE scouting_submissions
ADD COLUMN submitting_team_id INTEGER REFERENCES teams(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_scouting_submissions_submitting_team ON scouting_submissions(submitting_team_id);
