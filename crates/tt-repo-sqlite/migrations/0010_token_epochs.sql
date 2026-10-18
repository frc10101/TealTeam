-- Signing a user out everywhere (T1). A user's offline tokens (C9) carry
-- the epoch they were issued at; bumping it here refuses every one of them
-- at once, on every tablet. No row is epoch 0. Never replicated: who was
-- signed out, and by whom, is the server's.
CREATE TABLE token_epochs (
    user_id    INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    epoch      INTEGER NOT NULL,
    bumped_at  TEXT    NOT NULL,
    bumped_by  INTEGER REFERENCES users (id) ON DELETE SET NULL
);

-- Who scanned a tablet's forms in (T1, S13): the observations name the
-- scout and their tablet, this names the lead whose scan recorded them.
-- One row per scan that got as far as recording, the same codes scanned
-- twice included.
CREATE TABLE handoff_scans (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    scanned_at  TEXT    NOT NULL,
    lead_id     INTEGER REFERENCES users (id) ON DELETE SET NULL,
    scout_id    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    -- The tablet's UUID, as the scout's token names it.
    device_uuid TEXT    NOT NULL,
    nonce       TEXT    NOT NULL,
    sent        INTEGER NOT NULL,
    recorded    INTEGER NOT NULL
);
