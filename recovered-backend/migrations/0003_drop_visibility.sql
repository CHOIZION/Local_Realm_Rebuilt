-- Visibility is gone. Local Realm runs on one machine, for one person: there is
-- no audience to hide an entity from, so "private | public | hidden | spoiler"
-- only ever added a decision the writer had to make for no return. Exports are
-- now all-or-nothing, which is what a local tool can honestly promise.
ALTER TABLE entity DROP COLUMN visibility;
