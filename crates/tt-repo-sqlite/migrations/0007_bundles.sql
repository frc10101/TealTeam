-- Upstream bundles (S5): FIRST and TBA responses a client fetched with its own
-- signal and pushed to the Pi as a small SQLite file.

-- How far each remote has been read: one row per source, a cursor into that
-- source's own log. A bundle's source is `bundle:<log>`, the id of the
-- client's upstream log, so a bundle pushed twice, or one overlapping the
-- last, imports only what is new. A client that loses its storage starts a
-- new log with a new id, and is read from the beginning again.
CREATE TABLE sync_state (
    source     TEXT    PRIMARY KEY,
    cursor     INTEGER NOT NULL,
    applied_at TEXT    NOT NULL
);

-- Who pushed what. Only a lead scout may push, and upstream rows are
-- last-write-wins, so a bad bundle is undone by the next good fetch; this is
-- how anyone finds out where one came from.
CREATE TABLE bundle_imports (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    imported_at TEXT    NOT NULL,
    user_id     INTEGER REFERENCES users (id) ON DELETE SET NULL,
    -- The pushing tablet's device id, when it had one.
    device      TEXT,
    log         TEXT    NOT NULL,
    -- The bundle's rows past the cursor: (from_seq, to_seq].
    from_seq    INTEGER NOT NULL,
    to_seq      INTEGER NOT NULL,
    -- New content, appended to the upstream log.
    appended    INTEGER NOT NULL,
    -- The same body the log already had as newest.
    unchanged   INTEGER NOT NULL,
    -- Fetched before the log's newest for that path.
    stale       INTEGER NOT NULL,
    -- Not a FIRST or TBA response this Pi can read.
    refused     INTEGER NOT NULL
);
