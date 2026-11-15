-- An area may change colour over time as well as shape: each later outline
-- carries its own colour, empty for the area's own.
ALTER TABLE map_region_outline ADD COLUMN color TEXT NOT NULL DEFAULT '';

-- A pin linked to an entity always showed the entity's name, and the label
-- stored beside it was never seen. Pins now show their own label when they
-- have one, as areas do, so those unseen labels are cleared: each pin goes
-- on showing its entity's name, and follows it when the entity is renamed.
UPDATE map_marker SET label = ''
 WHERE entity_id IS NOT NULL AND entity_id IN (SELECT id FROM entity);
