-- The FTC Events API (FTC_BETA_PLAN.md, F7) logs its responses as 'ftc'.
-- SQLite cannot change a CHECK in place, so the table is rebuilt with every
-- row and its seq, and the AUTOINCREMENT counter carried over: a pull
-- cursor taken before this migration still means the same thing after it.
--
-- For 'ftc' rows, `etag` holds the response's Last-Modified, which the next
-- pull sends back as If-Modified-Since.

CREATE TABLE upstream_new (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    api        TEXT    NOT NULL CHECK (api IN ('first', 'tba', 'ftc')),
    path       TEXT    NOT NULL,
    etag       TEXT,
    body       TEXT    NOT NULL,
    fetched_at TEXT    NOT NULL,
    via        TEXT    NOT NULL DEFAULT 'pi'
);

INSERT INTO upstream_new (seq, api, path, etag, body, fetched_at, via)
    SELECT seq, api, path, etag, body, fetched_at, via FROM upstream;

DELETE FROM sqlite_sequence WHERE name = 'upstream_new';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'upstream_new', seq FROM sqlite_sequence WHERE name = 'upstream';

DROP TABLE upstream;
ALTER TABLE upstream_new RENAME TO upstream;

CREATE INDEX idx_upstream_path ON upstream (api, path, seq);
