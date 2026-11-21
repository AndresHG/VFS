-- VFS index schema, version 5: applied by db::open_rw on top of version 4 (VFS-18).
-- One row per shortcut (`.lnk`), Internet Shortcut (`.url`) or listed symbolic link or
-- junction (files.flags bit 1): where it points, read from the link itself by the deferred
-- link pass, never from the target. No full-text index: links are a few thousand rows and
-- the search scans them (search::link_pass).
CREATE TABLE links(
  file_id     INTEGER PRIMARY KEY,    -- files.id
  kind        INTEGER NOT NULL,       -- 0 shortcut (.lnk) 1 internet (.url) 2 symlink/junction
  state       INTEGER NOT NULL DEFAULT 0,  -- 0 pending 1 read 2 unreadable
  target      TEXT,                   -- path (native separators) or web address; NULL = none
  target_key  TEXT                    -- fold_key(target with '/' separators)
);
CREATE INDEX links_pending ON links(file_id) WHERE state = 0;
-- Existing shortcuts are queued for the deferred pass; no rescan is needed (R10).
INSERT INTO links(file_id, kind)
  SELECT id, CASE WHEN lower(name) LIKE '%.url' THEN 1 ELSE 0 END FROM files
  WHERE (lower(name) LIKE '%.lnk' OR lower(name) LIKE '%.url')
    AND name NOT LIKE '.lnk' AND name NOT LIKE '.url';
