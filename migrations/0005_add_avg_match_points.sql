-- Add avg_match_points column to team_event_stats table
ALTER TABLE team_event_stats 
ADD COLUMN IF NOT EXISTS avg_match_points NUMERIC(8,4);
