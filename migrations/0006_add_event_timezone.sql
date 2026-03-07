-- Add timezone field to events table
-- This stores the IANA timezone identifier (e.g., "America/Los_Angeles")
-- for accurate match time display regardless of server location

ALTER TABLE events ADD COLUMN timezone VARCHAR(50);

-- Set default timezones based on common FRC regional patterns
-- You can update these after running sync to pull actual location data
UPDATE events SET timezone = 'America/New_York' WHERE timezone IS NULL;

-- Add index for faster lookups
CREATE INDEX idx_events_timezone ON events(timezone);
