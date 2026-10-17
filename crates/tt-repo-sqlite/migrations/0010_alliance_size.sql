-- FTC (BIOBUZZ) plays alliances of two robots, not three
-- (tt_core::ALLIANCE_SIZE). The third slot of each alliance goes, so no stale
-- FRC row can carry a robot this build would not show. This is also a new
-- schema number (S11): a tablet still on the FRC build is refused at sync,
-- push, snapshot, and bundle, rather than misreading a four-robot match.
ALTER TABLE matches DROP COLUMN red3;
ALTER TABLE matches DROP COLUMN blue3;
