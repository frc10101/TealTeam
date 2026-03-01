-- Migration: 0002_remove_auto_path_fields.sql
-- Description: Remove legacy auto path fields from scouting tables and drop auto_paths table

-- Drop legacy auto_paths table if it exists
DROP TABLE IF EXISTS auto_paths CASCADE;

-- Drop deprecated auto path columns (only drops if columns exist)
ALTER TABLE scouting_data
    DROP COLUMN IF EXISTS auto_path_data,
    DROP COLUMN IF EXISTS auto_path_image_url;

ALTER TABLE scouting_submissions
    DROP COLUMN IF EXISTS auto_path_data;
