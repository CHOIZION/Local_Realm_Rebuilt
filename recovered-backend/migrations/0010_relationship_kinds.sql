-- Relationship kinds a world adds to the built-in menu (entities/relations.rs).
CREATE TABLE relationship_kind (
    key           TEXT PRIMARY KEY,               -- BONDED_TO
    label         TEXT NOT NULL,                  -- Bonded to
    inverse_key   TEXT NOT NULL DEFAULT '',
    inverse_label TEXT NOT NULL DEFAULT '',
    symmetrical   INTEGER NOT NULL DEFAULT 0,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
