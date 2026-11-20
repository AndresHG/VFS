-- VFS index schema, version 3: applied by db::open_rw on top of version 2. Smaller index,
-- same search results (docs/research/index-footprint.md).
--
-- files_fts keeps rowids only (detail=none): every name is stored after U+0001 so the
-- trigrams of U+0001 + term find the names starting with the term; search.rs narrows the
-- trigram candidates to the exact phrase matches in Rust. 148 MB -> 58 MB at 2 M files.
-- Both per-file indexes drop the per-row size table (columnsize=0, about 25 MB each at 2 M
-- files), which contentless_delete=1 requires, so their rows are deleted with the
-- 'delete' command and the indexed value, rebuilt from files and dirs (db::ingest).
DROP TABLE files_fts;
CREATE VIRTUAL TABLE files_fts USING fts5(
  name, content='', columnsize=0, detail='none', tokenize='trigram case_sensitive 1');
INSERT INTO files_fts(rowid, name) SELECT id, char(1) || name_key FROM files;

DROP TABLE file_paths_fts;
CREATE VIRTUAL TABLE file_paths_fts USING fts5(
  path, content='', columnsize=0, detail='none', tokenize='trigram case_sensitive 1');
INSERT INTO file_paths_fts(rowid, path)
  SELECT f.id, CASE d.path_key WHEN '' THEN f.name_key ELSE d.path_key || '/' || f.name_key END
  FROM files f JOIN dirs d ON d.id = f.dir_id;
