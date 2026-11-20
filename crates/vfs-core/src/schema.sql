-- VFS index schema, version 1 (PRAGMA user_version = 1). Applied once by db::open_rw.
-- FTS rows are written by db::ingest in the same transaction as the base row; no triggers.
-- No REFERENCES clauses: the indexer deletes subtrees in its own order.

CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;

CREATE TABLE volumes(
  id          INTEGER PRIMARY KEY,
  volume_key  TEXT NOT NULL UNIQUE,   -- Windows: volume GUID path; macOS: mount point
  label       TEXT,
  last_mount  TEXT
);

CREATE TABLE roots(
  id                INTEGER PRIMARY KEY,
  volume_id         INTEGER NOT NULL,
  path              TEXT NOT NULL UNIQUE,   -- absolute, native separators, no trailing separator
  path_key          TEXT NOT NULL,          -- fold_key(path with '/' separators)
  status            INTEGER NOT NULL DEFAULT 0,  -- 0 pending 1 scanning 2 ready 3 unavailable 4 removing
  last_full_scan_at INTEGER,                -- unix seconds; NULL until the first full scan ends
  files_count       INTEGER NOT NULL DEFAULT 0,
  dirs_count        INTEGER NOT NULL DEFAULT 0,
  added_at          INTEGER NOT NULL
);

-- Every directory once (empty ones included). Files never repeat their path. Directories
-- are search results too (VFS-1 decision D1): name_key and dirs_fts.name_key serve that.
CREATE TABLE dirs(
  id          INTEGER PRIMARY KEY,
  root_id     INTEGER NOT NULL,
  parent_id   INTEGER,                -- NULL for the root directory itself
  rel_path    TEXT NOT NULL,          -- original names, '/'-separated, relative to the root; '' = root
  path_key    TEXT NOT NULL,          -- fold_key(rel_path)
  name_key    TEXT NOT NULL,          -- fold_key(last component); root dir: last component of roots.path
  mtime       INTEGER NOT NULL DEFAULT 0,  -- unix seconds; 0 until the parent is listed
  scan_state  INTEGER NOT NULL DEFAULT 1,  -- 0 clean, 1 pending deep, 2 pending shallow
  UNIQUE(root_id, rel_path)
);
CREATE INDEX dirs_pending ON dirs(root_id) WHERE scan_state <> 0;
-- Serves ingest::list_children (child directories of one directory).
CREATE INDEX dirs_parent ON dirs(parent_id);

CREATE TABLE files(
  id            INTEGER PRIMARY KEY,
  dir_id        INTEGER NOT NULL,
  name          TEXT NOT NULL,        -- exactly as on disk (NFC or NFD preserved)
  name_key      TEXT NOT NULL,        -- fold_key(name)
  kind          INTEGER NOT NULL,     -- 0 other 1 document 2 photo 3 video 4 audio
  size          INTEGER NOT NULL,
  mtime         INTEGER NOT NULL,     -- unix seconds
  flags         INTEGER NOT NULL DEFAULT 0,  -- bit 0: name was not valid Unicode (stored lossily)
  content_state INTEGER NOT NULL DEFAULT 0,  -- phase 2 (content::ContentState); always 0 in v1
  UNIQUE(dir_id, name)
);

CREATE VIRTUAL TABLE files_fts USING fts5(
  name_key, content='', contentless_delete=1, tokenize='trigram case_sensitive 1');
-- Column filters: 'path_key : "t"' (directory-driven file search), 'name_key : "t"' (folder hits).
CREATE VIRTUAL TABLE dirs_fts USING fts5(
  path_key, name_key, content='', contentless_delete=1, tokenize='trigram case_sensitive 1');
