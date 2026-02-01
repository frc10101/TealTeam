-- Migration: 0004_rename_tables.sql
-- Description: Rename tables for better clarity and remove unused items table

-- ============================================================
-- REMOVE UNUSED TABLES
-- ============================================================
DROP TABLE IF EXISTS items;

-- ============================================================
-- RENAME TABLES
-- ============================================================

-- Rename competitions → events (more FRC-friendly terminology)
ALTER TABLE competitions RENAME TO events;

-- Rename competition_teams → event_teams (clearer, shorter)
ALTER TABLE competition_teams RENAME TO event_teams;

-- Rename match_teams → scouting_data (this is the scouting/KPI table!)
ALTER TABLE match_teams RENAME TO scouting_data;

-- ============================================================
-- UPDATE FOREIGN KEY COLUMN NAMES FOR CLARITY
-- ============================================================

-- Update event_teams foreign key column name
ALTER TABLE event_teams RENAME COLUMN competition_id TO event_id;

-- Update matches foreign key column name
ALTER TABLE matches RENAME COLUMN competition_id TO event_id;

-- ============================================================
-- UPDATE INDEX NAMES TO MATCH NEW TABLE NAMES
-- ============================================================

-- Events indexes (competitions → events)
ALTER INDEX IF EXISTS idx_matches_competition RENAME TO idx_matches_event;

-- Event_teams indexes (competition_teams → event_teams)
ALTER INDEX IF EXISTS idx_competition_teams_competition RENAME TO idx_event_teams_event;
ALTER INDEX IF EXISTS idx_competition_teams_team RENAME TO idx_event_teams_team;

-- Scouting_data indexes (match_teams → scouting_data)
ALTER INDEX IF EXISTS idx_match_teams_match RENAME TO idx_scouting_data_match;
ALTER INDEX IF EXISTS idx_match_teams_team RENAME TO idx_scouting_data_team;
ALTER INDEX IF EXISTS idx_match_teams_alliance RENAME TO idx_scouting_data_alliance;

-- ============================================================
-- UPDATE TRIGGER NAMES
-- ============================================================

-- Events triggers (competitions → events)
ALTER TRIGGER update_competitions_updated_at ON events RENAME TO update_events_updated_at;

-- Matches trigger (already correct name)
-- Scouting_data trigger (match_teams → scouting_data)
ALTER TRIGGER update_match_teams_updated_at ON scouting_data RENAME TO update_scouting_data_updated_at;

-- ============================================================
-- SUMMARY OF CHANGES
-- ============================================================
-- 
-- OLD NAME              → NEW NAME
-- ─────────────────────────────────────────
-- competitions          → events
-- competition_teams     → event_teams
-- match_teams           → scouting_data
-- items                 → [DELETED]
-- 
-- teams, matches, users → [UNCHANGED]
--
-- All foreign keys, indexes, and triggers have been updated accordingly.
