-- Maps, timelines, calendars and saved table views.
--
-- None of these own world facts. A map does not own its locations and a
-- timeline does not own its events: they hold placement, and point at entities
-- (plan section 52).

CREATE TABLE calendar (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    definition TEXT NOT NULL DEFAULT '{}',   -- JSON: months, leap rule, eras, day length
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE timeline (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    calendar_id TEXT REFERENCES calendar(id) ON DELETE SET NULL,
    config      TEXT NOT NULL DEFAULT '{}',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE timeline_event (
    id           TEXT PRIMARY KEY,
    timeline_id  TEXT NOT NULL REFERENCES timeline(id) ON DELETE CASCADE,
    entity_id    TEXT REFERENCES entity(id) ON DELETE SET NULL,
    title        TEXT NOT NULL,
    summary      TEXT NOT NULL DEFAULT '',
    kind         TEXT NOT NULL DEFAULT 'event',   -- event | period
    start_date   TEXT NOT NULL,                   -- JSON WorldDate
    end_date     TEXT,                            -- JSON WorldDate, periods only
    start_minute INTEGER NOT NULL,                -- derived from the calendar, for ordering
    end_minute   INTEGER,
    color        TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_timeline_event_order ON timeline_event(timeline_id, start_minute);
CREATE INDEX idx_timeline_event_entity ON timeline_event(entity_id);

CREATE TABLE map (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    media_id    TEXT REFERENCES media(id) ON DELETE SET NULL,
    width       INTEGER NOT NULL DEFAULT 0,
    height      INTEGER NOT NULL DEFAULT 0,
    config      TEXT NOT NULL DEFAULT '{}',   -- JSON: distance scale, default view
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE map_layer (
    id         TEXT PRIMARY KEY,
    map_id     TEXT NOT NULL REFERENCES map(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    visible    INTEGER NOT NULL DEFAULT 1,
    locked     INTEGER NOT NULL DEFAULT 0,
    opacity    REAL NOT NULL DEFAULT 1.0,
    color      TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_map_layer_map ON map_layer(map_id, sort_order);

-- Coordinates are normalised to 0..1 of the base image, so replacing the
-- artwork never moves a pin (plan section 20).
CREATE TABLE map_marker (
    id         TEXT PRIMARY KEY,
    map_id     TEXT NOT NULL REFERENCES map(id) ON DELETE CASCADE,
    layer_id   TEXT REFERENCES map_layer(id) ON DELETE SET NULL,
    entity_id  TEXT REFERENCES entity(id) ON DELETE SET NULL,
    label      TEXT NOT NULL DEFAULT '',
    note       TEXT NOT NULL DEFAULT '',
    x          REAL NOT NULL,
    y          REAL NOT NULL,
    icon       TEXT NOT NULL DEFAULT 'place',
    color      TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_map_marker_map ON map_marker(map_id);
CREATE INDEX idx_map_marker_entity ON map_marker(entity_id);

CREATE TABLE map_region (
    id         TEXT PRIMARY KEY,
    map_id     TEXT NOT NULL REFERENCES map(id) ON DELETE CASCADE,
    layer_id   TEXT REFERENCES map_layer(id) ON DELETE SET NULL,
    entity_id  TEXT REFERENCES entity(id) ON DELETE SET NULL,
    label      TEXT NOT NULL DEFAULT '',
    kind       TEXT NOT NULL DEFAULT 'area',   -- area | route
    points     TEXT NOT NULL DEFAULT '[]',     -- JSON [[x, y], ...]
    color      TEXT NOT NULL DEFAULT '',
    opacity    REAL NOT NULL DEFAULT 0.25,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_map_region_map ON map_region(map_id);
CREATE INDEX idx_map_region_entity ON map_region(entity_id);

-- A saved query over entities, not a copy of them (plan section 27).
CREATE TABLE table_view (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    type_id    TEXT REFERENCES entity_type(id) ON DELETE CASCADE,
    config     TEXT NOT NULL DEFAULT '{}',   -- JSON: columns, filters, sort, grouping
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
