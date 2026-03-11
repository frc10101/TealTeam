-- Migration: 0009_add_accuracy_rating.sql
-- Description: Add accuracy_rating column to scouting_data and scouting_submissions tables

ALTER TABLE scouting_data
ADD COLUMN IF NOT EXISTS accuracy_rating VARCHAR(20);

ALTER TABLE scouting_submissions
ADD COLUMN IF NOT EXISTS accuracy_rating VARCHAR(20);
