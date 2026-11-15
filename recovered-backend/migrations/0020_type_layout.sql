-- How a category's entries show their fields beside the article: NULL is the
-- infobox, as before; otherwise JSON {"view": "infobox" | "sheet",
-- "columns": 1-4, "cards": [{"key": section key or "", "wide": bool}]}.
-- Subcategories without a layout of their own use their parent's.
ALTER TABLE entity_type ADD COLUMN layout TEXT;
