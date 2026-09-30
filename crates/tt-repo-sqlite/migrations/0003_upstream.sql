-- The upstream log (S1): every FIRST and TBA response with new content, as
-- received. The events, matches, rankings, and stats tables are what the pages
-- read, derived from these responses; this is the record they came from, and
-- the stream clients will pull (S2) and bundles will feed (S5).
--
-- Append-only, except pruning: per request, only the newest few bodies stay.
-- Upstream data is last-write-wins, so the newest response per path is the
-- whole current state, and a client whose cursor is older than a pruned row
-- loses nothing by never seeing it. AUTOINCREMENT keeps seq from ever being
-- reused after a prune, so a cursor stays meaningful.

CREATE TABLE upstream (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    api        TEXT    NOT NULL CHECK (api IN ('first', 'tba')),
    -- The request, query included, with parameters in sorted order.
    path       TEXT    NOT NULL,
    etag       TEXT,
    -- The JSON as received.
    body       TEXT    NOT NULL,
    fetched_at TEXT    NOT NULL,
    -- Who fetched it: 'pi' for the server's own clients; a device, later, for
    -- a bundle a client fetched and pushed (S5).
    via        TEXT    NOT NULL DEFAULT 'pi'
);

CREATE INDEX idx_upstream_path ON upstream (api, path, seq);
