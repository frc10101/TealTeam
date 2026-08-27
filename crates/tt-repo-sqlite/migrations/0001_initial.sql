-- TealTeam initial schema (D1-D12).
--
-- SQLite, STRICT tables. Timestamps are ISO-8601 UTC TEXT: readable in any
-- database browser, which matters when the person debugging at 9pm on a
-- Saturday is a student with a phone and sqlite3.
--
-- Deliberate departures from the retired PostgreSQL schema, each recorded in
-- docs/REBUILD_SPEC.md section 12:
--
--   * No per-season columns anywhere. Observations carry a JSON payload plus the
--     schema version that shaped it (D5, 12.1).
--   * Observations reference the match they describe (D6, 12.2).
--   * Real UNIQUE constraints wherever an upsert happens, so ON CONFLICT works
--     instead of a select-then-insert race (D2, 12.11).
--   * client_record_id on everything a client can create offline (D7).
--   * No awards or zebra_data tables: nothing ever wrote them (D12, 12.3).
--   * One observations table with a review state, rather than two near-identical
--     tables that rows were copied between (see the observations comment).

-- ── Identity ────────────────────────────────────────────────────────────────

CREATE TABLE users (
    id             INTEGER PRIMARY KEY,
    email          TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    password_hash  TEXT    NOT NULL,
    team_number    INTEGER,

    -- Independent capabilities, not a hierarchy. is_admin implies the others by
    -- OR at the call site, never by inheritance in the data.
    is_admin       INTEGER NOT NULL DEFAULT 0,
    is_lead_scout  INTEGER NOT NULL DEFAULT 0,
    is_coach       INTEGER NOT NULL DEFAULT 0,

    last_login_at  TEXT,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
) STRICT;

-- Case-insensitive: nobody should be able to register Alice@ alongside alice@.
CREATE UNIQUE INDEX idx_users_email ON users (lower(email));
CREATE INDEX idx_users_team ON users (team_number);

CREATE TABLE sessions (
    id          TEXT    PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at  TEXT    NOT NULL,
    created_at  TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_sessions_user ON sessions (user_id);
CREATE INDEX idx_sessions_expiry ON sessions (expires_at);

-- NOTE: no selected_event_id. The retired schema kept the current event on the
-- session, which made every page a session read, nothing bookmarkable, two tabs
-- impossible, and offline unreachable (12.12). Event selection is client state
-- now (U2).

-- ── Devices ─────────────────────────────────────────────────────────────────

-- A physical tablet, identified independently of whoever is signed in on it, so
-- a lead can assign "the tablet on the left" to a robot.
CREATE TABLE devices (
    id             INTEGER PRIMARY KEY,
    device_uuid    TEXT    NOT NULL UNIQUE,
    name           TEXT,
    team_number    INTEGER,
    last_seen_at   TEXT,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_devices_last_seen ON devices (last_seen_at);

-- ── Competition graph ───────────────────────────────────────────────────────

CREATE TABLE teams (
    -- UNIQUE, unlike the retired schema, so upserts are real (12.11).
    team_number  INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL,
    nickname     TEXT,
    school       TEXT,
    city         TEXT,
    state        TEXT,
    country      TEXT,
    rookie_year  INTEGER,
    website      TEXT,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL
) STRICT;

CREATE TABLE events (
    -- The TBA key ("2026mabil") is the natural key: stable, meaningful, and what
    -- both upstream APIs agree on.
    tba_key      TEXT    PRIMARY KEY,
    name         TEXT    NOT NULL,
    location     TEXT,
    -- IANA identifier. Match times are rendered in the event's zone, never the
    -- server's -- see docs/TIMEZONE_HANDLING.md.
    timezone     TEXT,
    start_date   TEXT,
    end_date     TEXT,
    event_code   TEXT,
    event_type   TEXT,
    district_key TEXT,
    week         INTEGER,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_events_dates ON events (start_date, end_date);

CREATE TABLE event_teams (
    event_key    TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,
    team_number  INTEGER NOT NULL REFERENCES teams (team_number) ON DELETE CASCADE,
    created_at   TEXT    NOT NULL,
    PRIMARY KEY (event_key, team_number)
) STRICT;

CREATE INDEX idx_event_teams_team ON event_teams (team_number);

-- ── Matches ─────────────────────────────────────────────────────────────────

CREATE TABLE matches (
    tba_key       TEXT    PRIMARY KEY,
    event_key     TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,

    -- qm | sf | f. Parsed by tt_core::matches::CompLevel.
    comp_level    TEXT    NOT NULL,
    set_number    INTEGER NOT NULL DEFAULT 1,
    match_number  INTEGER NOT NULL,

    -- Alliance slots, stored as team NUMBERS rather than references, so a match
    -- still displays when the roster has not synced yet.
    red1  INTEGER, red2  INTEGER, red3  INTEGER,
    blue1 INTEGER, blue2 INTEGER, blue3 INTEGER,

    red_score      INTEGER,
    blue_score     INTEGER,
    winner         TEXT,
    played         INTEGER NOT NULL DEFAULT 0,

    scheduled_at   TEXT,
    actual_at      TEXT,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL,

    UNIQUE (event_key, comp_level, set_number, match_number)
) STRICT;

-- NOTE: the retired schema carried ~38 columns of 2022 score breakdown that no
-- code ever wrote (12.1), and encoded playoff rounds as set_number * 100 +
-- match_number to force a unique integer (2.3). Both are gone: the round is
-- three honest columns, and tba_key is the identity.

CREATE INDEX idx_matches_event ON matches (event_key, comp_level, set_number, match_number);
CREATE INDEX idx_matches_schedule ON matches (event_key, scheduled_at);

-- ── Assignments ─────────────────────────────────────────────────────────────

-- One assignee per robot per match. This is the backbone of the whole scouting
-- flow: it is what replaces asking a scout to pick a team out of a list of 50.
CREATE TABLE scout_assignments (
    id            INTEGER PRIMARY KEY,
    match_key     TEXT    NOT NULL REFERENCES matches (tba_key) ON DELETE CASCADE,
    team_number   INTEGER NOT NULL REFERENCES teams (team_number) ON DELETE CASCADE,
    event_key     TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,

    -- Either a person or a tablet. The CHECK enforces at least one.
    scouter_id    INTEGER REFERENCES users (id) ON DELETE CASCADE,
    device_id     INTEGER REFERENCES devices (id) ON DELETE CASCADE,

    assigned_by   INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,

    UNIQUE (match_key, team_number),
    CHECK (scouter_id IS NOT NULL OR device_id IS NOT NULL)
) STRICT;

CREATE INDEX idx_assignments_event ON scout_assignments (event_key);
CREATE INDEX idx_assignments_scouter ON scout_assignments (scouter_id);
CREATE INDEX idx_assignments_device ON scout_assignments (device_id);

-- ── Observations ────────────────────────────────────────────────────────────

-- One scout's record of one robot in one match.
--
-- The retired design had TWO tables with near-identical columns --
-- scouting_submissions (pending) and scouting_data (approved) -- and copied rows
-- between them on approval, deleting the original. Declining also deleted, which
-- destroyed a scout's work with no record and no way to tell them why (12.5).
--
-- One table with a review state instead. Approve and decline are updates, not
-- moves; nothing is ever destroyed; and there is exactly one schema to change
-- when a field is added.
CREATE TABLE observations (
    id                 INTEGER PRIMARY KEY,

    -- Client-generated UUIDv7, so a device can create observations offline and
    -- have them de-duplicate on sync (D7). Time-ordered, so it also sorts.
    client_record_id   TEXT    NOT NULL UNIQUE,

    -- Which robot, in which match. The retired schema recorded event and team but
    -- NOT the match, so duplicate and missing coverage were undetectable (12.2).
    match_key          TEXT    NOT NULL REFERENCES matches (tba_key) ON DELETE CASCADE,
    team_number        INTEGER NOT NULL REFERENCES teams (team_number) ON DELETE CASCADE,
    event_key          TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,
    alliance           TEXT    NOT NULL,

    -- The season-shaped answers, and the schema that shaped them. Adding a field
    -- next January is a new seasons/*.json and a version bump -- not a migration,
    -- not a form rewrite, not a scoring rewrite (D5, 12.1).
    payload            TEXT    NOT NULL,
    schema_version     INTEGER NOT NULL,

    -- Who recorded it, from which tablet, and for which team. submitting_team
    -- drives the notes privacy rule: a team sees only its own notes.
    scouter_id         INTEGER REFERENCES users (id) ON DELETE SET NULL,
    device_id          INTEGER REFERENCES devices (id) ON DELETE SET NULL,
    submitting_team    INTEGER,

    -- pending | approved | declined. Declined rows are RETAINED with a reason
    -- (L10) so the scout can be told and the lead has an audit trail.
    review_state       TEXT    NOT NULL DEFAULT 'pending',
    review_note        TEXT,
    reviewed_by        INTEGER REFERENCES users (id) ON DELETE SET NULL,
    reviewed_at        TEXT,

    -- When the match was watched, which is not when the row reached the server.
    observed_at        TEXT    NOT NULL,
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL,

    CHECK (review_state IN ('pending', 'approved', 'declined')),
    CHECK (alliance IN ('red', 'blue'))
) STRICT;

CREATE INDEX idx_observations_review ON observations (review_state, created_at);
CREATE INDEX idx_observations_team_event ON observations (team_number, event_key);
CREATE INDEX idx_observations_match ON observations (match_key);
CREATE INDEX idx_observations_scouter ON observations (scouter_id);

-- Coverage: at most one approved or pending observation per scout per robot per
-- match. A scout who submits twice for the same robot is correcting a mistake,
-- not recording a second data point. Declined rows are excluded so a corrected
-- resubmission is possible.
CREATE UNIQUE INDEX idx_observations_coverage
    ON observations (match_key, team_number, scouter_id)
    WHERE review_state <> 'declined' AND scouter_id IS NOT NULL;

-- ── Derived statistics ──────────────────────────────────────────────────────

CREATE TABLE team_event_stats (
    team_number      INTEGER NOT NULL REFERENCES teams (team_number) ON DELETE CASCADE,
    event_key        TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,

    -- REAL, not NUMERIC. The retired schema needed an explicit ::float8 cast on
    -- every single select to decode these at all.
    opr              REAL,
    dpr              REAL,
    ccwm             REAL,
    auto_opr         REAL,
    teleop_opr       REAL,
    endgame_opr      REAL,

    rank             INTEGER,
    matches_played   INTEGER,
    qual_average     REAL,
    avg_match_points REAL,
    wins             INTEGER,
    losses           INTEGER,
    ties             INTEGER,
    dq_count         INTEGER,
    qual_points      INTEGER,
    elim_points      INTEGER,
    award_points     INTEGER,
    alliance_points  INTEGER,
    total_points     INTEGER,

    -- When this row was pulled from upstream. Drives the freshness badges that
    -- stop a lead scout picking on rankings that look live and are forty minutes
    -- stale (I12).
    synced_at        TEXT    NOT NULL,

    PRIMARY KEY (team_number, event_key)
) STRICT;

CREATE INDEX idx_stats_event_rank ON team_event_stats (event_key, rank);

-- ── Lead scout tools ────────────────────────────────────────────────────────

-- Runtime overrides for the point values declared in the season schema. A stale
-- row naming a field that no longer exists is ignored, never fatal.
CREATE TABLE scouting_point_weights (
    field_key   TEXT    NOT NULL,
    option_key  TEXT    NOT NULL,
    points      INTEGER NOT NULL,
    updated_at  TEXT    NOT NULL,
    PRIMARY KEY (field_key, option_key)
) STRICT;

CREATE TABLE pick_list_entries (
    id                  INTEGER PRIMARY KEY,
    client_record_id    TEXT    NOT NULL UNIQUE,
    -- Whose list this is.
    owning_team         INTEGER NOT NULL,
    event_key           TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,
    -- Who is on it.
    picked_team         INTEGER NOT NULL,
    color               TEXT,
    crossed             INTEGER NOT NULL DEFAULT 0,
    position            INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,

    UNIQUE (owning_team, event_key, picked_team)
) STRICT;

CREATE INDEX idx_pick_list_order ON pick_list_entries (owning_team, event_key, position);
