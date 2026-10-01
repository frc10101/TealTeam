-- The key the Pi signs offline auth tokens with (C9): an Ed25519 seed, made
-- on first use and kept for the life of the database. One row. A new
-- database is a new key, and every token the old one signed is refused,
-- which is what a reset should do. Never replicated: whoever holds this can
-- sign in as anyone.
CREATE TABLE token_key (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    seed       BLOB    NOT NULL CHECK (length(seed) = 32),
    created_at TEXT    NOT NULL
);
