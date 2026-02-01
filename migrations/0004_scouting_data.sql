-- Migration: 0004_scouting_data.sql
-- Description: Scouting data fields based on DataPoints.md
-- Season: 2026

-- ============================================================
-- TEAMS TABLE - Add Blue Alliance API fields
-- ============================================================
ALTER TABLE teams ADD COLUMN IF NOT EXISTS tba_key VARCHAR(20); -- e.g., "frc254"
ALTER TABLE teams ADD COLUMN IF NOT EXISTS nickname VARCHAR(255);
ALTER TABLE teams ADD COLUMN IF NOT EXISTS school_name VARCHAR(255);
ALTER TABLE teams ADD COLUMN IF NOT EXISTS country VARCHAR(100);
ALTER TABLE teams ADD COLUMN IF NOT EXISTS rookie_year INTEGER;
ALTER TABLE teams ADD COLUMN IF NOT EXISTS motto TEXT;
ALTER TABLE teams ADD COLUMN IF NOT EXISTS website VARCHAR(500);

CREATE INDEX IF NOT EXISTS idx_teams_tba_key ON teams(tba_key);

-- ============================================================
-- COMPETITIONS TABLE - Add Blue Alliance API fields
-- ============================================================
ALTER TABLE competitions ADD COLUMN IF NOT EXISTS tba_key VARCHAR(20); -- e.g., "2026txho"
ALTER TABLE competitions ADD COLUMN IF NOT EXISTS event_type VARCHAR(50); -- regional, district, championship
ALTER TABLE competitions ADD COLUMN IF NOT EXISTS district_key VARCHAR(20);
ALTER TABLE competitions ADD COLUMN IF NOT EXISTS week INTEGER;

CREATE INDEX IF NOT EXISTS idx_competitions_tba_key ON competitions(tba_key);

-- ============================================================
-- TEAM_EVENT_STATS TABLE - Blue Alliance stats per team per event
-- ============================================================
CREATE TABLE IF NOT EXISTS team_event_stats (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    competition_id INTEGER NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    
    -- OPR/DPR/CCWM from Blue Alliance
    opr DECIMAL(10, 4),
    dpr DECIMAL(10, 4),
    ccwm DECIMAL(10, 4),
    
    -- Component OPRs (2026 specific)
    auto_opr DECIMAL(10, 4),
    teleop_opr DECIMAL(10, 4),
    endgame_opr DECIMAL(10, 4),
    
    -- Rankings
    rank INTEGER,
    matches_played INTEGER DEFAULT 0,
    qual_average DECIMAL(10, 4),
    wins INTEGER DEFAULT 0,
    losses INTEGER DEFAULT 0,
    ties INTEGER DEFAULT 0,
    dq_count INTEGER DEFAULT 0,
    
    -- District points
    qual_points INTEGER,
    elim_points INTEGER,
    award_points INTEGER,
    alliance_points INTEGER,
    total_points INTEGER,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(team_id, competition_id)
);

CREATE INDEX IF NOT EXISTS idx_team_event_stats_team ON team_event_stats(team_id);
CREATE INDEX IF NOT EXISTS idx_team_event_stats_competition ON team_event_stats(competition_id);

-- ============================================================
-- MATCHES TABLE - Add Blue Alliance fields
-- ============================================================
ALTER TABLE matches ADD COLUMN IF NOT EXISTS tba_key VARCHAR(50); -- e.g., "2026txho_qm1"
ALTER TABLE matches ADD COLUMN IF NOT EXISTS comp_level VARCHAR(10); -- qm, ef, qf, sf, f
ALTER TABLE matches ADD COLUMN IF NOT EXISTS set_number INTEGER;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS scheduled_time TIMESTAMP WITH TIME ZONE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS actual_time TIMESTAMP WITH TIME ZONE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS winning_alliance VARCHAR(10); -- red, blue, or empty for tie

-- 2026 Alliance Score Breakdowns (per alliance)
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_auto_tower_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_endgame_tower_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_auto_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_auto_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_teleop_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_teleop_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_endgame_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_endgame_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_total_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_hub_total_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_energized_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_supercharged_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_traversal_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_minor_foul_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_major_foul_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_foul_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_rp INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_total_auto_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS red_total_teleop_points INTEGER DEFAULT 0;

ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_auto_tower_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_endgame_tower_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_auto_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_auto_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_teleop_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_teleop_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_endgame_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_endgame_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_total_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_hub_total_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_energized_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_supercharged_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_traversal_achieved BOOLEAN DEFAULT FALSE;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_minor_foul_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_major_foul_count INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_foul_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_rp INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_total_auto_points INTEGER DEFAULT 0;
ALTER TABLE matches ADD COLUMN IF NOT EXISTS blue_total_teleop_points INTEGER DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_matches_tba_key ON matches(tba_key);

-- ============================================================
-- MATCH_TEAMS TABLE - Manual scouting data per team per match
-- ============================================================

-- Starting position
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS starting_position VARCHAR(20); -- left, center, right

-- Auto path (stored as JSON or path reference)
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS auto_path_data JSONB; -- Stores path coordinates
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS auto_path_image_url TEXT; -- Optional: stored image URL

-- Auto performance (2026 specific)
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS auto_tower_level VARCHAR(20); -- none, level1, level2, level3
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS auto_hand INTEGER DEFAULT 0; -- 0, 1, 2, 3

-- Scoring - subjective Likert scale
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS scoring_rating INTEGER CHECK (scoring_rating >= 1 AND scoring_rating <= 5);

-- Endgame
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS endgame_tower_level VARCHAR(20); -- none, level1, level2, level3
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS endgame_hang INTEGER DEFAULT 0; -- 0, 1, 2, 3

-- Defense rating
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS defense_rating VARCHAR(20); -- low, mid, high

-- Throughput / cycling speed
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS throughput VARCHAR(20); -- low, mid, high

-- Scoring strategy
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS scoring_strategy VARCHAR(50); -- passer, stealer, scorer

-- Traversal / mobility
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS traversal VARCHAR(20); -- trench, bump

-- Hub scoring contribution (scouter observed)
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS hub_auto_count INTEGER DEFAULT 0;
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS hub_teleop_count INTEGER DEFAULT 0;
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS hub_endgame_count INTEGER DEFAULT 0;

-- Penalties observed
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS penalties_caused INTEGER DEFAULT 0;

-- Scouting metadata
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS scouted_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE match_teams ADD COLUMN IF NOT EXISTS scouter_id INTEGER REFERENCES users(id) ON DELETE SET NULL;

-- ============================================================
-- AUTO_PATHS TABLE - Store reusable auto paths for teams
-- ============================================================
CREATE TABLE IF NOT EXISTS auto_paths (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    
    name VARCHAR(255), -- e.g., "3-piece left side"
    description TEXT,
    
    -- Path data
    path_data JSONB NOT NULL, -- Array of {x, y, timestamp} coordinates
    starting_position VARCHAR(20), -- left, center, right
    
    -- Performance stats
    times_used INTEGER DEFAULT 0,
    avg_success_rate DECIMAL(5, 2), -- percentage
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_auto_paths_team ON auto_paths(team_id);

-- ============================================================
-- AWARDS TABLE - Blue Alliance awards data
-- ============================================================
CREATE TABLE IF NOT EXISTS awards (
    id SERIAL PRIMARY KEY,
    competition_id INTEGER NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    team_id INTEGER REFERENCES teams(id) ON DELETE SET NULL, -- Can be null for individual awards
    
    tba_award_type INTEGER,
    name VARCHAR(255) NOT NULL,
    awardee VARCHAR(255), -- For individual awards
    year INTEGER NOT NULL,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_awards_team ON awards(team_id);
CREATE INDEX IF NOT EXISTS idx_awards_competition ON awards(competition_id);

-- ============================================================
-- ZEBRA_DATA TABLE - Robot tracking data (optional, for events that have it)
-- ============================================================
CREATE TABLE IF NOT EXISTS zebra_data (
    id SERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    
    -- Tracking data stored as JSONB arrays
    timestamps JSONB, -- Array of time values
    x_positions JSONB, -- Array of X coordinates
    y_positions JSONB, -- Array of Y coordinates
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(match_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_zebra_data_match ON zebra_data(match_id);
CREATE INDEX IF NOT EXISTS idx_zebra_data_team ON zebra_data(team_id);

-- ============================================================
-- Apply updated_at triggers to new tables
-- ============================================================
DROP TRIGGER IF EXISTS update_team_event_stats_updated_at ON team_event_stats;
CREATE TRIGGER update_team_event_stats_updated_at
    BEFORE UPDATE ON team_event_stats
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_auto_paths_updated_at ON auto_paths;
CREATE TRIGGER update_auto_paths_updated_at
    BEFORE UPDATE ON auto_paths
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
