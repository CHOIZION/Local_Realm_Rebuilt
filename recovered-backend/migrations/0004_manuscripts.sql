-- The manuscript system (plan sections 24 and 25).
--
-- A manuscript does not own characters (plan section 52). A chapter's prose is
-- an ordinary `document` row with kind = 'chapter', so it already carries word
-- count, revision history and the entity links the rest of the world reads;
-- what lives here is structure — where a chapter sits, and what the writer
-- knows about it that the prose itself does not say.

CREATE TABLE manuscript (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    goal_words  INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- Parts are optional. Chapters with no part hang directly off the book, so a
-- novel that never needed parts never has to invent one.
CREATE TABLE manuscript_part (
    id            TEXT PRIMARY KEY,
    manuscript_id TEXT NOT NULL REFERENCES manuscript(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_part_manuscript ON manuscript_part(manuscript_id, sort_order);

CREATE TABLE chapter (
    id            TEXT PRIMARY KEY,
    manuscript_id TEXT NOT NULL REFERENCES manuscript(id) ON DELETE CASCADE,
    part_id       TEXT REFERENCES manuscript_part(id) ON DELETE SET NULL,
    document_id   TEXT NOT NULL REFERENCES document(id) ON DELETE CASCADE,
    pov_entity_id TEXT REFERENCES entity(id) ON DELETE SET NULL,
    status        TEXT NOT NULL DEFAULT 'outline',  -- outline | draft | revised | final
    notes         TEXT NOT NULL DEFAULT '',
    goal_words    INTEGER NOT NULL DEFAULT 0,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_chapter_manuscript ON chapter(manuscript_id, sort_order);
CREATE INDEX idx_chapter_part ON chapter(part_id, sort_order);
CREATE UNIQUE INDEX idx_chapter_document ON chapter(document_id);
CREATE INDEX idx_chapter_pov ON chapter(pov_entity_id);

-- Words written per day, for the daily count section 25 asks for. Recorded as
-- a delta on every chapter save, so the streak survives closing the program.
CREATE TABLE writing_day (
    day           TEXT NOT NULL,                    -- YYYY-MM-DD, the writer's local day
    manuscript_id TEXT NOT NULL REFERENCES manuscript(id) ON DELETE CASCADE,
    words         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, manuscript_id)
);
