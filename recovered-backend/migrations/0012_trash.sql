-- Recently deleted: what a delete took, kept so it can be put back.
-- `payload` is JSON: every row the delete removed or emptied, and the files
-- it took with it (see src/trash.rs).
CREATE TABLE trash (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    ref_id      TEXT NOT NULL,
    name        TEXT NOT NULL,
    deleted_at  TEXT NOT NULL,
    payload     TEXT NOT NULL
);
CREATE INDEX trash_deleted_at ON trash(deleted_at);
