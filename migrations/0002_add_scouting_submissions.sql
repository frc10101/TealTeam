-- Migration: 0002_add_scouting_submissions.sql
-- Description: Add pending scouting submissions queue

CREATE TABLE IF NOT EXISTS scouting_submissions (
    id SERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    alliance_color VARCHAR(10) NOT NULL,
    alliance_position INTEGER NOT NULL,
    auto_score INTEGER DEFAULT 0,
    teleop_score INTEGER DEFAULT 0,
    endgame_score INTEGER DEFAULT 0,
    notes TEXT,
    starting_position VARCHAR(20),
    auto_path_data JSONB,
    defense_rating VARCHAR(20),
    traversal VARCHAR(20),
    throughput VARCHAR(20),
    scoring_strategy VARCHAR(50),
    shooting_speed VARCHAR(20),
    capacity VARCHAR(20),
    defendability TEXT,
    hang_level VARCHAR(10),
    auto_hang VARCHAR(10),
    hang_position VARCHAR(20),
    scouted_at TIMESTAMP WITH TIME ZONE,
    scouter_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(match_id, team_id),
    UNIQUE(match_id, alliance_color, alliance_position)
);

CREATE INDEX IF NOT EXISTS idx_scouting_submissions_match ON scouting_submissions(match_id);
CREATE INDEX IF NOT EXISTS idx_scouting_submissions_team ON scouting_submissions(team_id);
CREATE INDEX IF NOT EXISTS idx_scouting_submissions_scouter ON scouting_submissions(scouter_id);
CREATE INDEX IF NOT EXISTS idx_scouting_submissions_created_at ON scouting_submissions(created_at);
