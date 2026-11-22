-- VFS index schema, version 7: applied by db::open_rw on top of version 6 (VFS-37).
-- `dirs.unpopulated`: 1 when the folder last listed empty while Windows still marked it as
-- not all present locally (a cloud placeholder its provider never filled in), else NULL.
-- `roots.cloud_only_count`, `roots.links_count`: the root's cloud-only files and links at
-- the end of its last full walk, NULL until then. Nullable columns rewrite no row; the
-- partial index holds only marked folders, so counting them per root is cheap.
ALTER TABLE dirs ADD COLUMN unpopulated INTEGER;
CREATE INDEX dirs_unpopulated ON dirs(root_id) WHERE unpopulated IS NOT NULL;
ALTER TABLE roots ADD COLUMN cloud_only_count INTEGER;
ALTER TABLE roots ADD COLUMN links_count INTEGER;
