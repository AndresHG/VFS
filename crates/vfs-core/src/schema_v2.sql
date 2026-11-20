-- VFS index schema, version 2: applied by db::open_rw on top of version 1.
-- Every file's folded relative path and name ("dir path_key/name_key", or name_key alone in
-- the root directory) in one trigram index, so a query whose terms are split between the
-- folders and the name is one FTS intersection (VFS-1 S2, AC-9). detail=none keeps rowids
-- only (about 320 MB at 2 M files instead of about 1 GB with positions); candidates are
-- verified, so the lost adjacency only admits a few false positives.
CREATE VIRTUAL TABLE file_paths_fts USING fts5(
  path, content='', contentless_delete=1, tokenize='trigram case_sensitive 1', detail='none');
INSERT INTO file_paths_fts(rowid, path)
  SELECT f.id, CASE d.path_key WHEN '' THEN f.name_key ELSE d.path_key || '/' || f.name_key END
  FROM files f JOIN dirs d ON d.id = f.dir_id;
