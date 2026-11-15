-- A timeline entry may be about several entities, and may happen at a pin on a map.
CREATE TABLE timeline_event_entity (
    event_id   TEXT NOT NULL REFERENCES timeline_event(id) ON DELETE CASCADE,
    entity_id  TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, entity_id)
);
CREATE INDEX idx_timeline_event_entity_entity ON timeline_event_entity(entity_id);
INSERT INTO timeline_event_entity (event_id, entity_id, sort_order)
    SELECT id, entity_id, 0 FROM timeline_event WHERE entity_id IS NOT NULL;

-- timeline_event.entity_id stays, holding the first of them, for anything
-- that reads one.
ALTER TABLE timeline_event ADD COLUMN marker_id TEXT REFERENCES map_marker(id) ON DELETE SET NULL;
