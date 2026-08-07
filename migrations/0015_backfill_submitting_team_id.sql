-- Migration: 0015_backfill_submitting_team_id.sql
-- Description: Backfill submitting_team_id on scouting data and pending
--              submissions from the scouter's team. The columns existed since
--              0007/0008 but were never written, which prevented team notes
--              from being displayed (the team page filters notes by
--              submitting_team_id).

UPDATE scouting_data sd
SET submitting_team_id = t.id
FROM users u
JOIN teams t ON t.team_number = u.team_number
WHERE sd.scouter_id = u.id
  AND sd.submitting_team_id IS NULL
  AND u.team_number IS NOT NULL;

UPDATE scouting_submissions ss
SET submitting_team_id = t.id
FROM users u
JOIN teams t ON t.team_number = u.team_number
WHERE ss.scouter_id = u.id
  AND ss.submitting_team_id IS NULL
  AND u.team_number IS NOT NULL;
