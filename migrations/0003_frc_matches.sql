-- Migration: 0003_frc_matches.sql
-- Description: FRC-style match structure with alliances

-- Drop old match_rounds table and recreate with proper FRC structure
DROP TABLE IF EXISTS match_rounds;

-- ============================================================
-- MATCHES TABLE (One row per match)
-- ============================================================
CREATE TABLE IF NOT EXISTS matches (
    id SERIAL PRIMARY KEY,
    competition_id INTEGER NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    match_number INTEGER NOT NULL,
    match_type VARCHAR(50) DEFAULT 'qualification', -- qualification, quarterfinal, semifinal, final
    
    -- Alliance scores
    red_score INTEGER DEFAULT 0,
    blue_score INTEGER DEFAULT 0,
    
    -- Match status
    played BOOLEAN DEFAULT FALSE,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(competition_id, match_number, match_type)
);

CREATE INDEX IF NOT EXISTS idx_matches_competition ON matches(competition_id);
CREATE INDEX IF NOT EXISTS idx_matches_number ON matches(match_number);

-- ============================================================
-- MATCH_TEAMS TABLE (6 teams per match - 3 red, 3 blue)
-- ============================================================
CREATE TABLE IF NOT EXISTS match_teams (
    id SERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    
    -- Alliance info
    alliance_color VARCHAR(10) NOT NULL, -- red, blue
    alliance_position INTEGER NOT NULL,  -- 1, 2, 3 (driver station position)
    
    -- Team-specific scouting data for this match
    -- Score contributions (placeholder - expand later)
    auto_score INTEGER DEFAULT 0,
    teleop_score INTEGER DEFAULT 0,
    endgame_score INTEGER DEFAULT 0,
    
    -- TODO: Add match-specific fields here later
    -- Example fields that might be added:
    -- auto_leave BOOLEAN DEFAULT FALSE,
    -- auto_pieces_scored INTEGER DEFAULT 0,
    -- teleop_pieces_scored INTEGER DEFAULT 0,
    -- endgame_status VARCHAR(50),
    -- defense_rating INTEGER,
    -- penalties INTEGER DEFAULT 0,
    
    -- Notes and metadata
    notes TEXT,
    scouter_name VARCHAR(255),
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(match_id, team_id),
    UNIQUE(match_id, alliance_color, alliance_position)
);

CREATE INDEX IF NOT EXISTS idx_match_teams_match ON match_teams(match_id);
CREATE INDEX IF NOT EXISTS idx_match_teams_team ON match_teams(team_id);
CREATE INDEX IF NOT EXISTS idx_match_teams_alliance ON match_teams(alliance_color);

-- Apply updated_at triggers
DROP TRIGGER IF EXISTS update_matches_updated_at ON matches;
CREATE TRIGGER update_matches_updated_at
    BEFORE UPDATE ON matches
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_match_teams_updated_at ON match_teams;
CREATE TRIGGER update_match_teams_updated_at
    BEFORE UPDATE ON match_teams
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
