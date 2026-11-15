-- Relationship trees (plan section 22).
--
-- A tree owns no relationships (plan section 52). It records a question — start
-- at this entity, follow these kinds of relationship, this far — and the answer
-- is read from the `relationship` table every time it is drawn. Deleting a tree
-- cannot cost a single fact about the world.
--
-- What is genuinely the tree's own is curation: which branches the writer chose
-- to hide, and where they dragged a node to. That is placement, not fact, so it
-- lives in `config` beside the question rather than in a table of its own.

CREATE TABLE relationship_tree (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    -- The tree survives its root being deleted; it simply has nothing to draw
    -- until another root is chosen.
    root_entity_id TEXT REFERENCES entity(id) ON DELETE SET NULL,
    kinds          TEXT NOT NULL DEFAULT '[]',   -- JSON [relationship kinds followed; empty = all]
    config         TEXT NOT NULL DEFAULT '{}',   -- JSON: depth, orientation, hidden, positions
    sort_order     INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX idx_tree_root ON relationship_tree(root_entity_id);
