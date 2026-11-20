-- VFS index schema, version 4: applied by db::open_rw on top of version 3 (VFS-7).
-- Files are only ever looked up by folder (ingest::list_children, subtree deletes), never
-- by (folder, name): UNIQUE(dir_id, name) stored every name a second time (85 MB at 2 M
-- files) for nothing. A plain index on dir_id (22 MB) serves the same lookups. Names stay
-- unique per folder because the indexer inserts only names its listing of that folder
-- lacks. Ids are kept, so the FTS rows (rowid = files.id) stay valid.
CREATE TABLE files_v4(
  id            INTEGER PRIMARY KEY,
  dir_id        INTEGER NOT NULL,
  name          TEXT NOT NULL,        -- exactly as on disk (NFC or NFD preserved)
  name_key      TEXT NOT NULL,        -- fold_key(name)
  kind          INTEGER NOT NULL,     -- 0 other 1 document 2 photo 3 video 4 audio
  size          INTEGER NOT NULL,
  mtime         INTEGER NOT NULL,     -- unix seconds
  flags         INTEGER NOT NULL DEFAULT 0,  -- bit 0: name was not valid Unicode (stored lossily)
  content_state INTEGER NOT NULL DEFAULT 0   -- content search (content::ContentState); always 0 in v1
);
INSERT INTO files_v4(id, dir_id, name, name_key, kind, size, mtime, flags, content_state)
  SELECT id, dir_id, name, name_key, kind, size, mtime, flags, content_state FROM files
  ORDER BY id;
DROP TABLE files;
ALTER TABLE files_v4 RENAME TO files;
CREATE INDEX files_dir ON files(dir_id);
