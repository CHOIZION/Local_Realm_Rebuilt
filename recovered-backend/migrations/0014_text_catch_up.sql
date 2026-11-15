-- Work a realm owes once, done the next time it opens (projects::open).
-- 'text': rebuild the search index and recount every text's words, after
-- Chinese and Japanese came to be searched and counted a character at a time.
CREATE TABLE pending_work (task TEXT PRIMARY KEY);
INSERT INTO pending_work (task) VALUES ('text');
