-- How a manuscript's pages are set for print (PDF and DOCX): paper, type
-- size, line spacing, first-line indent and margins, as JSON. '{}' means
-- standard manuscript format.
ALTER TABLE manuscript ADD COLUMN layout TEXT NOT NULL DEFAULT '{}';
