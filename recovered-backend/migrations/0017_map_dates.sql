-- Map features in time: the moment a pin, area, route or stamp comes into the
-- world and the moment it leaves it, as JSON dates in the realm's default
-- calendar. NULL is "always": a feature without dates is shown at every
-- moment, so maps made before dates existed look exactly as they did.
ALTER TABLE map_marker ADD COLUMN valid_from TEXT;
ALTER TABLE map_marker ADD COLUMN valid_until TEXT;
ALTER TABLE map_region ADD COLUMN valid_from TEXT;
ALTER TABLE map_region ADD COLUMN valid_until TEXT;
ALTER TABLE map_stamp ADD COLUMN valid_from TEXT;
ALTER TABLE map_stamp ADD COLUMN valid_until TEXT;

-- Borders that move: an area's later outlines, each in force from its date
-- until the next one. The area's own points are its outline before the first.
CREATE TABLE map_region_outline (
    id         TEXT PRIMARY KEY,
    region_id  TEXT NOT NULL REFERENCES map_region(id) ON DELETE CASCADE,
    date       TEXT NOT NULL,                 -- JSON WorldDate
    points     TEXT NOT NULL DEFAULT '[]',    -- JSON [[x, y], ...]
    created_at TEXT NOT NULL
);
CREATE INDEX idx_map_region_outline_region ON map_region_outline(region_id);
