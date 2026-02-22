-- Migration: 0004_add_lead_scout.sql
-- Description: Add lead scout flag to users

ALTER TABLE users
ADD COLUMN IF NOT EXISTS is_lead_scout BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_users_is_lead_scout ON users(is_lead_scout);
