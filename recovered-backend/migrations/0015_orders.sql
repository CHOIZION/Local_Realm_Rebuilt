-- Orders the writer chooses by dragging: areas and routes within a map
-- (the top of the list is drawn in front), and notes in the sidebar. 0 for
-- all means the old order: newest shape first, notes by title.
ALTER TABLE map_region ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
ALTER TABLE document ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
