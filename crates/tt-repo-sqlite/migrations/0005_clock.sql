-- Each tablet's clock against the server's (S12), as the tablet last measured
-- it: server time minus tablet time, in milliseconds. A tablet whose clock is
-- off stamps its observations with the wrong time, which matters once they
-- are recorded offline and synced later (C7).
ALTER TABLE devices ADD COLUMN clock_offset_ms INTEGER;
ALTER TABLE devices ADD COLUMN clock_checked_at TEXT;
