-- How a portrait sits in its frames: the point of the picture to keep in view
-- and how far to zoom in, as JSON {"x": 0-100, "y": 0-100, "zoom": 1-4}. It
-- belongs to the entry, not the picture, so two entries can frame one picture
-- differently; NULL is the picture centred and whole, as before.
ALTER TABLE entity ADD COLUMN image_frame TEXT;
