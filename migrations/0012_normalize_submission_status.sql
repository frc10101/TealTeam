-- Migration: 0012_normalize_submission_status.sql
-- Description: Normalize legacy/blank submission status values to pending

UPDATE scouting_submissions
SET status = 'pending'
WHERE status IS NULL OR BTRIM(status) = '';
