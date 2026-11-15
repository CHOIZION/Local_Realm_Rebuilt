-- The typeface a manuscript compiles in (plan section 31).
--
-- It sits on the manuscript rather than being asked for at every compile,
-- because it is a property of the book — this one goes to the agent in standard
-- manuscript format, that one is a sample set in Times — and a writer who has
-- chosen once should not have to choose again every time they export.
--
-- Courier is the default, and an empty string reads as Courier, so a manuscript
-- written before this column existed compiles exactly as it did before.
ALTER TABLE manuscript ADD COLUMN compile_font TEXT NOT NULL DEFAULT 'courier';
