-- Migration: 0002_competitions.sql
-- Description: Competition, teams, and match rounds schema

-- ============================================================
-- COMPETITIONS TABLE
-- ============================================================
CREATE TABLE IF NOT EXISTS competitions (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    location VARCHAR(255),
    start_date DATE,
    end_date DATE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================
-- TEAMS TABLE
-- ============================================================
CREATE TABLE IF NOT EXISTS teams (
    id SERIAL PRIMARY KEY,
    team_number INTEGER NOT NULL,
    name VARCHAR(255) NOT NULL,
    school VARCHAR(255),
    city VARCHAR(255),
    state VARCHAR(50),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Index for team number lookups
CREATE INDEX IF NOT EXISTS idx_teams_team_number ON teams(team_number);

-- ============================================================
-- COMPETITION_TEAMS TABLE (Many-to-Many relationship)
-- ============================================================
CREATE TABLE IF NOT EXISTS competition_teams (
    id SERIAL PRIMARY KEY,
    competition_id INTEGER NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(competition_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_competition_teams_competition ON competition_teams(competition_id);
CREATE INDEX IF NOT EXISTS idx_competition_teams_team ON competition_teams(team_id);

-- ============================================================
-- MATCH_ROUNDS TABLE
-- ============================================================
CREATE TABLE IF NOT EXISTS match_rounds (
    id SERIAL PRIMARY KEY,
    competition_id INTEGER NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    round_number INTEGER NOT NULL,
    
    -- Match metadata
    match_type VARCHAR(50) DEFAULT 'qualification', -- qualification, playoff, final
    alliance_color VARCHAR(10), -- red, blue
    alliance_position INTEGER, -- 1, 2, 3
    
    -- Score fields (placeholder - to be expanded later)
    total_score INTEGER DEFAULT 0,
    auto_score INTEGER DEFAULT 0,
    teleop_score INTEGER DEFAULT 0,
    endgame_score INTEGER DEFAULT 0,
    
    -- Match outcome
    won BOOLEAN DEFAULT FALSE,
    tied BOOLEAN DEFAULT FALSE,
    
    -- TODO: Add match-specific fields here later
    -- Example fields that might be added:
    -- auto_mobility BOOLEAN DEFAULT FALSE,
    -- auto_pieces_scored INTEGER DEFAULT 0,
    -- teleop_pieces_scored INTEGER DEFAULT 0,
    -- endgame_climb_level INTEGER DEFAULT 0,
    -- penalties INTEGER DEFAULT 0,
    
    -- Notes and metadata
    notes TEXT,
    scouter_name VARCHAR(255),
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(competition_id, team_id, round_number)
);

CREATE INDEX IF NOT EXISTS idx_match_rounds_competition ON match_rounds(competition_id);
CREATE INDEX IF NOT EXISTS idx_match_rounds_team ON match_rounds(team_id);
CREATE INDEX IF NOT EXISTS idx_match_rounds_round ON match_rounds(round_number);

-- Apply updated_at triggers
DROP TRIGGER IF EXISTS update_competitions_updated_at ON competitions;
CREATE TRIGGER update_competitions_updated_at
    BEFORE UPDATE ON competitions
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_teams_updated_at ON teams;
CREATE TRIGGER update_teams_updated_at
    BEFORE UPDATE ON teams
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_match_rounds_updated_at ON match_rounds;
CREATE TRIGGER update_match_rounds_updated_at
    BEFORE UPDATE ON match_rounds
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
