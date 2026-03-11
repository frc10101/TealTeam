-- Migration: 0010_add_submission_status.sql
-- Description: Add status and rejection_reason to scouting_submissions for rejection workflow

ALTER TABLE scouting_submissions
ADD COLUMN status VARCHAR(20) NOT NULL DEFAULT 'pending';

ALTER TABLE scouting_submissions
ADD COLUMN rejection_reason TEXT;

CREATE INDEX IF NOT EXISTS idx_scouting_submissions_status ON scouting_submissions(status);
CREATE INDEX IF NOT EXISTS idx_scouting_submissions_scouter_status ON scouting_submissions(scouter_id, status);
