-- Outbox entries the Pi refused (C10): an observation a scout saved on a
-- tablet, pushed later, that broke a rule the form would have caught, such
-- as a match not on the schedule yet or a robot not in that match. The
-- device keeps it with the reason; this is the lead scout's copy, so it
-- reaches someone who can fix it rather than waiting on one tablet.
--
-- Kept as pushed, with what the push resolved: who, from which tablet, for
-- which team, and when, corrected by the tablet's clock (S12). A lead
-- either records it, perhaps against another match or robot, or dismisses
-- it; the row stays either way, saying which and who.
CREATE TABLE refused_entries (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    -- As sent. A second push of the same entry updates the reason.
    client_record_id TEXT    NOT NULL UNIQUE,
    match_key        TEXT    NOT NULL,
    team_number      INTEGER NOT NULL,
    payload          TEXT    NOT NULL,
    schema_version   INTEGER NOT NULL,
    observed_at      TEXT    NOT NULL,
    scouter_id       INTEGER REFERENCES users (id) ON DELETE SET NULL,
    device_id        INTEGER REFERENCES devices (id) ON DELETE SET NULL,
    submitting_team  INTEGER,
    reason           TEXT    NOT NULL,
    refused_at       TEXT    NOT NULL,
    -- NULL while waiting for a lead.
    resolution       TEXT    CHECK (resolution IN ('recorded', 'dismissed')),
    resolved_by      INTEGER REFERENCES users (id) ON DELETE SET NULL,
    resolved_at      TEXT,
    -- The observation a 'recorded' resolution made.
    observation_id   INTEGER REFERENCES observations (id) ON DELETE SET NULL
);

