-- Migration: 0011_add_scouting_point_weights.sql
-- Description: Persist configurable scouting point weights for lead scout ranking control

CREATE TABLE IF NOT EXISTS scouting_point_weights (
    id SERIAL PRIMARY KEY,
    metric_key VARCHAR(64) NOT NULL,
    option_key VARCHAR(64) NOT NULL,
    points INTEGER NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(metric_key, option_key)
);

CREATE INDEX IF NOT EXISTS idx_scouting_point_weights_metric ON scouting_point_weights(metric_key);
