-- The venue stream (S2): every insert, update, and delete of what scouts and
-- leads write -- observations (reviews and declines included), assignments,
-- and pick lists -- in one append-only log that clients pull by cursor.
--
-- Written by triggers, in the same transaction as the change, so no code path
-- can forget to log and a deletion is a row like any other. Not per-table
-- `updated_at` watermarks: those cannot see a deleted row, and a sequence
-- taken at INSERT but visible at COMMIT can be skipped for good. The pull
-- serves only changes a couple of seconds old, which closes that race too.
--
-- Only these three tables replicate. users, sessions, and devices have no
-- triggers, and must never get one: that omission is the allowlist.
--
-- team_scope: NULL is public; a team number is visible to that team only.
-- Observations are public; their notes are removed at pull time for anyone
-- but the writing team (U13). A pick list belongs to its team.

CREATE TABLE changes (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    entity     TEXT    NOT NULL CHECK (entity IN ('observation', 'assignment', 'pick_list_entry')),
    -- Stable across devices: client_record_id, or match_key:team_number.
    entity_pk  TEXT    NOT NULL,
    op         TEXT    NOT NULL CHECK (op IN ('upsert', 'delete')),
    -- The whole row as JSON; NULL for a delete.
    payload    TEXT,
    event_key  TEXT,
    team_scope INTEGER,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ── Observations ────────────────────────────────────────────────────────────

CREATE TRIGGER changes_observation_insert AFTER INSERT ON observations BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key)
    VALUES ('observation', NEW.client_record_id, 'upsert', json_object(
        'client_record_id', NEW.client_record_id, 'match_key', NEW.match_key,
        'team_number', NEW.team_number, 'event_key', NEW.event_key, 'alliance', NEW.alliance,
        'payload', json(NEW.payload), 'schema_version', NEW.schema_version,
        'scouter_id', NEW.scouter_id, 'device_id', NEW.device_id,
        'submitting_team', NEW.submitting_team, 'review_state', NEW.review_state,
        'review_note', NEW.review_note, 'reviewed_by', NEW.reviewed_by,
        'reviewed_at', NEW.reviewed_at, 'observed_at', NEW.observed_at,
        'updated_at', NEW.updated_at), NEW.event_key);
END;

CREATE TRIGGER changes_observation_update AFTER UPDATE ON observations BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key)
    VALUES ('observation', NEW.client_record_id, 'upsert', json_object(
        'client_record_id', NEW.client_record_id, 'match_key', NEW.match_key,
        'team_number', NEW.team_number, 'event_key', NEW.event_key, 'alliance', NEW.alliance,
        'payload', json(NEW.payload), 'schema_version', NEW.schema_version,
        'scouter_id', NEW.scouter_id, 'device_id', NEW.device_id,
        'submitting_team', NEW.submitting_team, 'review_state', NEW.review_state,
        'review_note', NEW.review_note, 'reviewed_by', NEW.reviewed_by,
        'reviewed_at', NEW.reviewed_at, 'observed_at', NEW.observed_at,
        'updated_at', NEW.updated_at), NEW.event_key);
END;

CREATE TRIGGER changes_observation_delete AFTER DELETE ON observations BEGIN
    INSERT INTO changes (entity, entity_pk, op, event_key)
    VALUES ('observation', OLD.client_record_id, 'delete', OLD.event_key);
END;

-- ── Assignments ─────────────────────────────────────────────────────────────

CREATE TRIGGER changes_assignment_insert AFTER INSERT ON scout_assignments BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key)
    VALUES ('assignment', NEW.match_key || ':' || NEW.team_number, 'upsert', json_object(
        'match_key', NEW.match_key, 'team_number', NEW.team_number, 'event_key', NEW.event_key,
        'scouter_id', NEW.scouter_id, 'device_id', NEW.device_id,
        'assigned_by', NEW.assigned_by, 'updated_at', NEW.updated_at), NEW.event_key);
END;

CREATE TRIGGER changes_assignment_update AFTER UPDATE ON scout_assignments BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key)
    VALUES ('assignment', NEW.match_key || ':' || NEW.team_number, 'upsert', json_object(
        'match_key', NEW.match_key, 'team_number', NEW.team_number, 'event_key', NEW.event_key,
        'scouter_id', NEW.scouter_id, 'device_id', NEW.device_id,
        'assigned_by', NEW.assigned_by, 'updated_at', NEW.updated_at), NEW.event_key);
END;

CREATE TRIGGER changes_assignment_delete AFTER DELETE ON scout_assignments BEGIN
    INSERT INTO changes (entity, entity_pk, op, event_key)
    VALUES ('assignment', OLD.match_key || ':' || OLD.team_number, 'delete', OLD.event_key);
END;

-- ── Pick lists ──────────────────────────────────────────────────────────────

CREATE TRIGGER changes_pick_insert AFTER INSERT ON pick_list_entries BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key, team_scope)
    VALUES ('pick_list_entry', NEW.client_record_id, 'upsert', json_object(
        'client_record_id', NEW.client_record_id, 'owning_team', NEW.owning_team,
        'event_key', NEW.event_key, 'picked_team', NEW.picked_team, 'color', NEW.color,
        'crossed', NEW.crossed, 'position', NEW.position, 'updated_at', NEW.updated_at),
        NEW.event_key, NEW.owning_team);
END;

CREATE TRIGGER changes_pick_update AFTER UPDATE ON pick_list_entries BEGIN
    INSERT INTO changes (entity, entity_pk, op, payload, event_key, team_scope)
    VALUES ('pick_list_entry', NEW.client_record_id, 'upsert', json_object(
        'client_record_id', NEW.client_record_id, 'owning_team', NEW.owning_team,
        'event_key', NEW.event_key, 'picked_team', NEW.picked_team, 'color', NEW.color,
        'crossed', NEW.crossed, 'position', NEW.position, 'updated_at', NEW.updated_at),
        NEW.event_key, NEW.owning_team);
END;

CREATE TRIGGER changes_pick_delete AFTER DELETE ON pick_list_entries BEGIN
    INSERT INTO changes (entity, entity_pk, op, event_key, team_scope)
    VALUES ('pick_list_entry', OLD.client_record_id, 'delete', OLD.event_key, OLD.owning_team);
END;

-- ── What already exists ─────────────────────────────────────────────────────
-- Logged once, so the log alone rebuilds the current state. A no-op UPDATE
-- fires the update triggers with each row's own values.

UPDATE observations SET id = id;
UPDATE scout_assignments SET id = id;
UPDATE pick_list_entries SET id = id;
