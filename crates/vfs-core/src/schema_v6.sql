-- VFS index schema, version 6: applied by db::open_rw on top of version 5 (VFS-10).
-- `dirs.error`: why a folder could not be listed (0 no permission, 1 not available,
-- 2 cloud-only, 3 disk error), NULL once it lists. Adding a nullable column rewrites no row.
-- The partial index holds only folders with an error, so counting them per root is cheap.
ALTER TABLE dirs ADD COLUMN error INTEGER;
CREATE INDEX dirs_unreadable ON dirs(root_id) WHERE error IS NOT NULL;
