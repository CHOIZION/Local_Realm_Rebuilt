-- Entries in the order the writer drags them into within their category.
-- 0 for all keeps a category alphabetical.
ALTER TABLE entity ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
