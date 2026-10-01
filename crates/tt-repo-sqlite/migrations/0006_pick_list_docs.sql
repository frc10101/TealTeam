-- Each team's pick list as a yrs document (L14): the thing copies of the list
-- merge, so two people's changes both land however they arrive. The rows in
-- pick_list_entries are what the document reads as, rewritten in the same
-- transaction as it, and are still what replicates through `changes`.
--
-- A list stored before this has no document. It gets one, made from its rows,
-- the first time it is read or changed.
CREATE TABLE pick_list_docs (
    owning_team INTEGER NOT NULL,
    event_key   TEXT    NOT NULL REFERENCES events (tba_key) ON DELETE CASCADE,
    -- The whole document, as one yrs v1 update.
    state       BLOB    NOT NULL,
    updated_at  TEXT    NOT NULL,

    PRIMARY KEY (owning_team, event_key)
) STRICT;
