-- How a map gets its picture (plan section 20).
--
-- 'image'   — a picture the user brought, stored in the media library.
-- 'painted' — a biome grid painted inside the application. The grid lives in
--             maps/<id>.png as an indexed-colour image: each pixel's palette
--             index is a biome, and the palette is the biome colours, so the
--             file is still an ordinary picture outside the application.
--
-- Every map made before this column existed was an uploaded image.
ALTER TABLE map ADD COLUMN kind TEXT NOT NULL DEFAULT 'image';
