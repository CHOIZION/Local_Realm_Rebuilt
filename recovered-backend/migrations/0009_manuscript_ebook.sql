-- How a manuscript is laid out as an ebook (plan section 31): title page,
-- headings, paragraph style, cover and the rest, as JSON. Chosen once and kept
-- on the book, like the typeface for print. '{}' means the defaults.
ALTER TABLE manuscript ADD COLUMN ebook TEXT NOT NULL DEFAULT '{}';
