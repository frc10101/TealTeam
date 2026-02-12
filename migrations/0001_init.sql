-- Migration: 0001_init.sql
-- Description: Consolidated schema for test database

-- Enable UUID extension (optional)
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ============================================================
-- DROP EXISTING TABLES (TEST DB RESET)
-- ============================================================
DROP TABLE IF EXISTS zebra_data CASCADE;
DROP TABLE IF EXISTS awards CASCADE;
DROP TABLE IF EXISTS auto_paths CASCADE;
DROP TABLE IF EXISTS team_event_stats CASCADE;
DROP TABLE IF EXISTS scouting_data CASCADE;
DROP TABLE IF EXISTS matches CASCADE;
DROP TABLE IF EXISTS event_teams CASCADE;
DROP TABLE IF EXISTS events CASCADE;
DROP TABLE IF EXISTS teams CASCADE;
DROP TABLE IF EXISTS sessions CASCADE;
DROP TABLE IF EXISTS users CASCADE;

-- ============================================================
-- UPDATED_AT TRIGGER FUNCTION
-- ============================================================
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ language 'plpgsql';

-- ============================================================
-- USERS + SESSIONS
-- ============================================================
CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    password_hash VARCHAR(255) NOT NULL DEFAULT '',
    role VARCHAR(50) DEFAULT 'user',
    last_login TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

CREATE TABLE IF NOT EXISTS sessions (
    session_id VARCHAR(255) PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);

-- ============================================================
-- TEAMS
-- ============================================================
CREATE TABLE IF NOT EXISTS teams (
    id SERIAL PRIMARY KEY,
    team_number INTEGER NOT NULL,
    name VARCHAR(255) NOT NULL,
    school VARCHAR(255),
    city VARCHAR(255),
    state VARCHAR(50),
    tba_key VARCHAR(20),
    nickname VARCHAR(255),
    school_name VARCHAR(255),
    country VARCHAR(100),
    rookie_year INTEGER,
    motto TEXT,
    website VARCHAR(500),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_teams_team_number ON teams(team_number);
CREATE INDEX IF NOT EXISTS idx_teams_tba_key ON teams(tba_key);

-- ============================================================
-- EVENTS
-- ============================================================
CREATE TABLE IF NOT EXISTS events (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    location VARCHAR(255),
    start_date DATE,
    end_date DATE,
    tba_key VARCHAR(20),
    event_type VARCHAR(50),
    district_key VARCHAR(20),
    week INTEGER,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_events_tba_key ON events(tba_key);

-- ============================================================
-- EVENT TEAMS (Many-to-Many)
-- ============================================================
CREATE TABLE IF NOT EXISTS event_teams (
    id SERIAL PRIMARY KEY,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(event_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_event_teams_event ON event_teams(event_id);
CREATE INDEX IF NOT EXISTS idx_event_teams_team ON event_teams(team_id);

-- ============================================================
-- MATCHES
-- ============================================================
CREATE TABLE IF NOT EXISTS matches (
    id SERIAL PRIMARY KEY,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    match_number INTEGER NOT NULL,
    match_type VARCHAR(50) DEFAULT 'qualification',
    red_score INTEGER DEFAULT 0,
    blue_score INTEGER DEFAULT 0,
    played BOOLEAN DEFAULT FALSE,
    tba_key VARCHAR(50),
    comp_level VARCHAR(10),
    set_number INTEGER,
    scheduled_time TIMESTAMP WITH TIME ZONE,
    actual_time TIMESTAMP WITH TIME ZONE,
    winning_alliance VARCHAR(10),

    red_auto_tower_points INTEGER DEFAULT 0,
    red_endgame_tower_points INTEGER DEFAULT 0,
    red_hub_auto_count INTEGER DEFAULT 0,
    red_hub_auto_points INTEGER DEFAULT 0,
    red_hub_teleop_count INTEGER DEFAULT 0,
    red_hub_teleop_points INTEGER DEFAULT 0,
    red_hub_endgame_count INTEGER DEFAULT 0,
    red_hub_endgame_points INTEGER DEFAULT 0,
    red_hub_total_count INTEGER DEFAULT 0,
    red_hub_total_points INTEGER DEFAULT 0,
    red_energized_achieved BOOLEAN DEFAULT FALSE,
    red_supercharged_achieved BOOLEAN DEFAULT FALSE,
    red_traversal_achieved BOOLEAN DEFAULT FALSE,
    red_minor_foul_count INTEGER DEFAULT 0,
    red_major_foul_count INTEGER DEFAULT 0,
    red_foul_points INTEGER DEFAULT 0,
    red_rp INTEGER DEFAULT 0,
    red_total_auto_points INTEGER DEFAULT 0,
    red_total_teleop_points INTEGER DEFAULT 0,

    blue_auto_tower_points INTEGER DEFAULT 0,
    blue_endgame_tower_points INTEGER DEFAULT 0,
    blue_hub_auto_count INTEGER DEFAULT 0,
    blue_hub_auto_points INTEGER DEFAULT 0,
    blue_hub_teleop_count INTEGER DEFAULT 0,
    blue_hub_teleop_points INTEGER DEFAULT 0,
    blue_hub_endgame_count INTEGER DEFAULT 0,
    blue_hub_endgame_points INTEGER DEFAULT 0,
    blue_hub_total_count INTEGER DEFAULT 0,
    blue_hub_total_points INTEGER DEFAULT 0,
    blue_energized_achieved BOOLEAN DEFAULT FALSE,
    blue_supercharged_achieved BOOLEAN DEFAULT FALSE,
    blue_traversal_achieved BOOLEAN DEFAULT FALSE,
    blue_minor_foul_count INTEGER DEFAULT 0,
    blue_major_foul_count INTEGER DEFAULT 0,
    blue_foul_points INTEGER DEFAULT 0,
    blue_rp INTEGER DEFAULT 0,
    blue_total_auto_points INTEGER DEFAULT 0,
    blue_total_teleop_points INTEGER DEFAULT 0,

    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,

    UNIQUE(event_id, match_number, match_type)
);

CREATE INDEX IF NOT EXISTS idx_matches_event ON matches(event_id);
CREATE INDEX IF NOT EXISTS idx_matches_number ON matches(match_number);
CREATE INDEX IF NOT EXISTS idx_matches_tba_key ON matches(tba_key);

-- ============================================================
-- SCOUTING DATA (per team per match)
-- ============================================================
CREATE TABLE IF NOT EXISTS scouting_data (
    id SERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    alliance_color VARCHAR(10) NOT NULL,
    alliance_position INTEGER NOT NULL,
    auto_score INTEGER DEFAULT 0,
    teleop_score INTEGER DEFAULT 0,
    endgame_score INTEGER DEFAULT 0,
    notes TEXT,
    scouter_name VARCHAR(255),
    starting_position VARCHAR(20),
    auto_path_data JSONB,
    auto_path_image_url TEXT,
    auto_tower_level VARCHAR(20),
    auto_hand INTEGER DEFAULT 0,
    scoring_rating INTEGER CHECK (scoring_rating >= 1 AND scoring_rating <= 5),
    endgame_tower_level VARCHAR(20),
    endgame_hang INTEGER DEFAULT 0,
    defense_rating VARCHAR(20),
    throughput VARCHAR(20),
    scoring_strategy VARCHAR(50),
    shooting_speed VARCHAR(20),
    capacity VARCHAR(20),
    defendability TEXT,
    traversal VARCHAR(20),
    hang_level VARCHAR(10),
    auto_hang VARCHAR(10),
    hang_position VARCHAR(20),
    hub_auto_count INTEGER DEFAULT 0,
    hub_teleop_count INTEGER DEFAULT 0,
    hub_endgame_count INTEGER DEFAULT 0,
    penalties_caused INTEGER DEFAULT 0,
    scouted_at TIMESTAMP WITH TIME ZONE,
    scouter_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(match_id, team_id),
    UNIQUE(match_id, alliance_color, alliance_position)
);

CREATE INDEX IF NOT EXISTS idx_scouting_data_match ON scouting_data(match_id);
CREATE INDEX IF NOT EXISTS idx_scouting_data_team ON scouting_data(team_id);
CREATE INDEX IF NOT EXISTS idx_scouting_data_alliance ON scouting_data(alliance_color);

-- ============================================================
-- TEAM EVENT STATS
-- ============================================================
CREATE TABLE IF NOT EXISTS team_event_stats (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    opr DECIMAL(10, 4),
    dpr DECIMAL(10, 4),
    ccwm DECIMAL(10, 4),
    auto_opr DECIMAL(10, 4),
    teleop_opr DECIMAL(10, 4),
    endgame_opr DECIMAL(10, 4),
    rank INTEGER,
    matches_played INTEGER DEFAULT 0,
    qual_average DECIMAL(10, 4),
    wins INTEGER DEFAULT 0,
    losses INTEGER DEFAULT 0,
    ties INTEGER DEFAULT 0,
    dq_count INTEGER DEFAULT 0,
    qual_points INTEGER,
    elim_points INTEGER,
    award_points INTEGER,
    alliance_points INTEGER,
    total_points INTEGER,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(team_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_team_event_stats_team ON team_event_stats(team_id);
CREATE INDEX IF NOT EXISTS idx_team_event_stats_event ON team_event_stats(event_id);

-- ============================================================
-- AUTO PATHS
-- ============================================================
CREATE TABLE IF NOT EXISTS auto_paths (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name VARCHAR(255),
    description TEXT,
    path_data JSONB NOT NULL,
    starting_position VARCHAR(20),
    times_used INTEGER DEFAULT 0,
    avg_success_rate DECIMAL(5, 2),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_auto_paths_team ON auto_paths(team_id);

-- ============================================================
-- AWARDS
-- ============================================================
CREATE TABLE IF NOT EXISTS awards (
    id SERIAL PRIMARY KEY,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    team_id INTEGER REFERENCES teams(id) ON DELETE SET NULL,
    tba_award_type INTEGER,
    name VARCHAR(255) NOT NULL,
    awardee VARCHAR(255),
    year INTEGER NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_awards_team ON awards(team_id);
CREATE INDEX IF NOT EXISTS idx_awards_event ON awards(event_id);

-- ============================================================
-- ZEBRA DATA
-- ============================================================
CREATE TABLE IF NOT EXISTS zebra_data (
    id SERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    timestamps JSONB,
    x_positions JSONB,
    y_positions JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(match_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_zebra_data_match ON zebra_data(match_id);
CREATE INDEX IF NOT EXISTS idx_zebra_data_team ON zebra_data(team_id);

-- ============================================================
-- UPDATED_AT TRIGGERS
-- ============================================================
DROP TRIGGER IF EXISTS update_users_updated_at ON users;
CREATE TRIGGER update_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_teams_updated_at ON teams;
CREATE TRIGGER update_teams_updated_at
    BEFORE UPDATE ON teams
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_events_updated_at ON events;
CREATE TRIGGER update_events_updated_at
    BEFORE UPDATE ON events
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_matches_updated_at ON matches;
CREATE TRIGGER update_matches_updated_at
    BEFORE UPDATE ON matches
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_scouting_data_updated_at ON scouting_data;
CREATE TRIGGER update_scouting_data_updated_at
    BEFORE UPDATE ON scouting_data
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

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
