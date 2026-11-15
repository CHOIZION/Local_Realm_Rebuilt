-- Stamps: pictures placed on a map as objects of their own (plan section 20).
--
-- A stamp never changes the map underneath it. It can be moved, resized,
-- rotated, recoloured or removed at any time, and it belongs to a layer like a
-- pin does. Its picture is a PNG in maps/stamps/, named by the hash of its
-- contents, so the world keeps every stamp's picture even if the brush pack it
-- came from is changed or removed, and identical pictures are stored once.
CREATE TABLE map_stamp (
    id         TEXT PRIMARY KEY,
    map_id     TEXT NOT NULL REFERENCES map(id) ON DELETE CASCADE,
    layer_id   TEXT REFERENCES map_layer(id) ON DELETE SET NULL,
    image      TEXT NOT NULL,                -- file name in maps/stamps/
    label      TEXT NOT NULL DEFAULT '',     -- where it came from, for people
    x          REAL NOT NULL,                -- centre, as fractions of the map
    y          REAL NOT NULL,
    width      REAL NOT NULL,                -- fraction of the map's width
    aspect     REAL NOT NULL DEFAULT 1,      -- picture height / width
    rotation   REAL NOT NULL DEFAULT 0,      -- degrees, clockwise
    flip       INTEGER NOT NULL DEFAULT 0,   -- mirrored left to right
    tint       TEXT NOT NULL DEFAULT '',     -- '' keeps the picture's colours
    opacity    REAL NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,   -- higher draws on top
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_map_stamp_map ON map_stamp(map_id, sort_order);
