-- Presence for people, not just tablets (A6, L2).
--
-- Auto-distribute hands robots to whoever is here. The retired app decided
-- that from an unexpired 24-hour session, so it assigned robots to people who
-- had gone home (REBUILD_SPEC.md 12.13). Tablets already heartbeat every 60s;
-- a heartbeat from a signed-in browser now marks its user as seen too.

ALTER TABLE users ADD COLUMN last_seen_at TEXT;

-- Who was signed in at a tablet's latest heartbeat, so a lead scout can tell
-- "Device 0191f7ad" from "Device 0191f7ac" before naming them.
ALTER TABLE devices ADD COLUMN last_user_id INTEGER REFERENCES users (id) ON DELETE SET NULL;
