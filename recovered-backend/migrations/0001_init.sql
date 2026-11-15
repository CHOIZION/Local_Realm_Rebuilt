-- WorldForge world database, schema version 1.
-- Structure lives here; long form prose lives in Markdown files next to it.

CREATE TABLE world_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE media (
    id         TEXT PRIMARY KEY,
    filename   TEXT NOT NULL,
    path       TEXT NOT NULL,            -- relative to the world directory
    mime       TEXT NOT NULL DEFAULT '',
    kind       TEXT NOT NULL DEFAULT 'other', -- image | audio | video | document | model | other
    bytes      INTEGER NOT NULL DEFAULT 0,
    sha256     TEXT NOT NULL DEFAULT '',
    caption    TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_media_sha ON media(sha256);

-- Entity types are data, never SQL tables (§48).
CREATE TABLE entity_type (
    id          TEXT PRIMARY KEY,
    parent_id   TEXT REFERENCES entity_type(id) ON DELETE SET NULL,
    name        TEXT NOT NULL,
    plural      TEXT NOT NULL DEFAULT '',
    icon        TEXT NOT NULL DEFAULT '',
    color       TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_entity_type_name ON entity_type(name COLLATE NOCASE);
CREATE INDEX idx_entity_type_parent ON entity_type(parent_id);

CREATE TABLE property_definition (
    id         TEXT PRIMARY KEY,
    type_id    TEXT NOT NULL REFERENCES entity_type(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    label      TEXT NOT NULL,
    data_type  TEXT NOT NULL,
    config     TEXT NOT NULL DEFAULT '{}',   -- JSON: options, target type, min/max, ...
    hint       TEXT NOT NULL DEFAULT '',
    required   INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_propdef_type_key ON property_definition(type_id, key);

CREATE TABLE entity (
    id             TEXT PRIMARY KEY,
    type_id        TEXT NOT NULL REFERENCES entity_type(id) ON DELETE RESTRICT,
    name           TEXT NOT NULL,
    summary        TEXT NOT NULL DEFAULT '',
    image_media_id TEXT REFERENCES media(id) ON DELETE SET NULL,
    visibility     TEXT NOT NULL DEFAULT 'private', -- private | public | hidden | spoiler
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX idx_entity_type ON entity(type_id);
CREATE INDEX idx_entity_name ON entity(name COLLATE NOCASE);

CREATE TABLE alias (
    id        TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    name      TEXT NOT NULL
);
CREATE INDEX idx_alias_entity ON alias(entity_id);
CREATE UNIQUE INDEX idx_alias_unique ON alias(entity_id, name COLLATE NOCASE);

CREATE TABLE property_value (
    entity_id   TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    property_id TEXT NOT NULL REFERENCES property_definition(id) ON DELETE CASCADE,
    value       TEXT NOT NULL,             -- JSON encoded
    PRIMARY KEY (entity_id, property_id)
);

CREATE TABLE relationship (
    id           TEXT PRIMARY KEY,
    source_id    TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    target_id    TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,                    -- SPOUSE_OF
    inverse_kind TEXT NOT NULL DEFAULT '',         -- CHILD_OF for PARENT_OF
    symmetrical  INTEGER NOT NULL DEFAULT 0,
    note         TEXT NOT NULL DEFAULT '',
    start_date   TEXT NOT NULL DEFAULT '',
    end_date     TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_rel_source ON relationship(source_id);
CREATE INDEX idx_rel_target ON relationship(target_id);

CREATE TABLE tag (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX idx_tag_name ON tag(name COLLATE NOCASE);

CREATE TABLE entity_tag (
    entity_id TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    tag_id    TEXT NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (entity_id, tag_id)
);

-- Markdown documents. The body is a file on disk; this row is the index entry.
CREATE TABLE document (
    id         TEXT PRIMARY KEY,
    entity_id  TEXT REFERENCES entity(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL DEFAULT 'article',  -- article | note | chapter
    title      TEXT NOT NULL,
    path       TEXT NOT NULL,                    -- relative to the world directory
    word_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_document_entity ON document(entity_id) WHERE entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_document_path ON document(path);

CREATE TABLE revision (
    id          TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES document(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    word_count  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_revision_doc ON revision(document_id, created_at DESC);

CREATE TABLE entity_media (
    entity_id  TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    media_id   TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    role       TEXT NOT NULL DEFAULT 'attachment',
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (entity_id, media_id, role)
);

-- Unified reference index feeding backlinks (§16). Rebuilt on every save.
CREATE TABLE link (
    source_kind TEXT NOT NULL,              -- entity | document
    source_id   TEXT NOT NULL,
    target_id   TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    context     TEXT NOT NULL DEFAULT '',   -- property label, or 'body'
    PRIMARY KEY (source_kind, source_id, target_id, context)
);
CREATE INDEX idx_link_target ON link(target_id);

CREATE VIRTUAL TABLE search_index USING fts5(
    kind UNINDEXED,
    ref_id UNINDEXED,
    title,
    aliases,
    body,
    tags,
    props,
    tokenize = "unicode61 remove_diacritics 2"
);
