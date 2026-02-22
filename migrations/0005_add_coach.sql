-- Migration: 0005_add_coach.sql
-- Description: Add drive coach flag to users

ALTER TABLE users
ADD COLUMN IF NOT EXISTS is_coach BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_users_is_coach ON users(is_coach);
