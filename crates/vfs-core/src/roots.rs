//! Adding, removing and listing root folders (VFS-1 AC-4). Owned by task T2a.

use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::db::ingest;
use crate::error::{Error, OverlapRelation, Result};
use crate::fold::fold_key;
use crate::platform;
use crate::types::{Root, RootStatus, UnreadableDir, UnreadableReason};

/// Column 6 counts the root's unreadable folders on the partial index `dirs_unreadable`
/// (its condition is spelled out so SQLite uses it, VFS-10 R4); column 7 is its pause key
/// (VFS-32 R1).
const ROOT_COLS: &str = concat!(
    "id, path, status, files_count, dirs_count, last_full_scan_at, \
    (SELECT COUNT(*) FROM dirs d WHERE d.root_id = roots.id AND d.error IS NOT NULL), EXISTS ",
    paused_key_sql!("roots.id")
);

/// Prefix of a root's pause key in `meta`, `paused/<root id>` (VFS-32 R1). Every key and SQL
/// condition is built from it.
macro_rules! paused_prefix {
    () => {
        "paused/"
    };
}

/// A subquery that has a row when the root whose id is the SQL expression `$id` is paused.
macro_rules! paused_key_sql {
    ($id:literal) => {
        concat!(
            "(SELECT 1 FROM meta WHERE key = '",
            $crate::roots::paused_prefix!(),
            "' || ",
            $id,
            ")"
        )
    };
}
pub(crate) use {paused_key_sql, paused_prefix};

fn read_root(r: &Row<'_>) -> rusqlite::Result<Root> {
    let path: String = r.get(1)?;
    Ok(Root {
        id: r.get(0)?,
        drive: platform::drive_kind(Path::new(&path)),
        path,
        status: RootStatus::from_code(r.get(2)?),
        files: r.get::<_, i64>(3)?.max(0) as u64,
        dirs: r.get::<_, i64>(4)?.max(0) as u64,
        last_full_scan: r.get(5)?,
        unreadable: r.get::<_, i64>(6)?.max(0) as u64,
        paused: r.get(7)?,
    })
}

/// Every root, in the order added; roots being removed are listed with status `removing`.
pub(crate) fn list_roots(conn: &Connection) -> Result<Vec<Root>> {
    let mut stmt = conn.prepare(&format!("SELECT {ROOT_COLS} FROM roots ORDER BY id"))?;
    let roots = stmt
        .query_map([], read_root)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(roots)
}

/// Most folders [`unreadable_dirs`] returns (VFS-10 R4).
pub(crate) const UNREADABLE_LIST_CAP: usize = 500;

/// The folders of `root_id` that could not be listed, in path order, at most
/// [`UNREADABLE_LIST_CAP`] (VFS-10 R4); `Root::unreadable` has the full count.
pub(crate) fn unreadable_dirs(conn: &Connection, root_id: i64) -> Result<Vec<UnreadableDir>> {
    let mut stmt = conn.prepare(
        "SELECT d.id, r.path, d.rel_path, d.error FROM dirs d JOIN roots r ON r.id = d.root_id \
         WHERE d.root_id = ?1 AND d.error IS NOT NULL ORDER BY d.rel_path LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![root_id, UNREADABLE_LIST_CAP as i64],
            |r| {
                let root: String = r.get(1)?;
                let rel: String = r.get(2)?;
                Ok(UnreadableDir {
                    id: r.get(0)?,
                    path: crate::db::join_rel(&root, &rel)
                        .to_string_lossy()
                        .into_owned(),
                    reason: UnreadableReason::from_code(r.get(3)?),
                })
            },
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// True when the folded path `path` lies strictly below the folded path `ancestor`.
fn is_below(path: &str, ancestor: &str) -> bool {
    path.strip_prefix(ancestor.trim_end_matches('/'))
        .is_some_and(|rest| rest.starts_with('/'))
}

/// What `add_root` took in from the roots inside the new one (VFS-31 R2), for its
/// diagnostic record.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TakenIn {
    pub roots: u64,
    pub dirs: u64,
    pub files: u64,
}

/// The diagnostic record of a root added (VFS-31 R5): ids, counts and milliseconds only.
pub(crate) fn add_record(id: i64, taken: TakenIn, took: Duration) -> String {
    format!(
        "root_add id={id} taken_roots={} taken_dirs={} taken_files={} ms={}",
        taken.roots,
        taken.dirs,
        taken.files,
        took.as_millis()
    )
}

/// The diagnostic record of a root marked for removal (VFS-31 R5).
pub(crate) fn remove_record(id: i64, took: Duration) -> String {
    format!("root_remove id={id} ms={}", took.as_millis())
}

/// The diagnostic record of a removal the indexer finished (VFS-31 R5).
pub(crate) fn removed_record(id: i64, dirs: u64, files: u64, took: Duration) -> String {
    format!(
        "root_removed id={id} dirs={dirs} files={files} ms={}",
        took.as_millis()
    )
}

/// Canonicalizes `path` (`dunce`), rejects a missing path (`NotFound`), a file
/// (`NotADirectory`) and a path equal to or inside an existing root (`OverlappingRoot`),
/// then inserts it with `db::ingest::insert_root` and `platform::volume_key`. The facade
/// notifies the indexer afterwards. The returned root carries its `drive`, so the UI can
/// warn about a network or removable drive.
///
/// Roots inside the new one are taken in, in the same transaction (VFS-31 R2): their
/// folders and files move to the new root with their ids (`take_in`), so they stay
/// searchable and are not inserted again; the new root's first walk re-lists them.
///
/// Roots being removed do not count as overlapping: search and the watcher already ignore
/// them. Re-adding the same folder while it is being removed supersedes the removal: the
/// old root's remaining rows are deleted in this transaction and the folder is added anew.
///
/// The `OverlappingRoot` error carries the [`OverlapRelation`] and the existing root path;
/// its message reads `"<relation> <existing root path>"`, so the UI can word each case
/// (U4). Paths are compared on folded keys, so case, accents and NFC/NFD differences still
/// count as the same folder.
pub(crate) fn add_root(conn: &mut Connection, path: &Path) -> Result<(Root, TakenIn)> {
    let canon = dunce::canonicalize(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => Error::NotFound(path.display().to_string()),
        _ => Error::Io(e),
    })?;
    if !canon.is_dir() {
        return Err(Error::NotADirectory(canon.display().to_string()));
    }
    let path_str = canon
        .to_str()
        .ok_or_else(|| {
            Error::Unavailable(format!("name is not valid Unicode: {}", canon.display()))
        })?
        .to_string();
    let key = fold_key(&path_str.replace('\\', "/"));

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut inner = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id, path, path_key, status FROM roots ORDER BY id")?;
        let mut rows = stmt.query([])?;
        let mut superseded = Vec::new();
        while let Some(row) = rows.next()? {
            let (id, existing, existing_key): (i64, String, String) =
                (row.get(0)?, row.get(1)?, row.get(2)?);
            let same = key.trim_end_matches('/') == existing_key.trim_end_matches('/');
            if RootStatus::from_code(row.get(3)?) == RootStatus::Removing {
                if same {
                    superseded.push(id);
                }
                continue;
            }
            let relation = if same {
                OverlapRelation::Same
            } else if is_below(&key, &existing_key) {
                OverlapRelation::Inside
            } else {
                if is_below(&existing_key, &key) {
                    inner.push((id, existing));
                }
                continue;
            };
            return Err(Error::OverlappingRoot { relation, existing });
        }
        drop(rows);
        for id in superseded {
            ingest::delete_root(&tx, id)?;
            // Root ids are reused: the old root's change cursor and pause go with it
            // (VFS-9 R8, VFS-32 R1).
            crate::indexer::forget_cursor(&tx, id)?;
            set_paused_key(&tx, id, false)?;
        }
    }
    let id = ingest::insert_root(&tx, &platform::volume_key(&canon), &path_str)?;
    let mut taken = TakenIn::default();
    for (inner_id, inner_path) in inner {
        let rel = rel_below(&canon, Path::new(&inner_path));
        take_in(&tx, id, inner_id, &rel, &mut taken)?;
    }
    // The taken-in files count from the start; the first walk's tally adds what it finds.
    tx.execute(
        "UPDATE roots SET files_count = ?2, dirs_count = ?3 WHERE id = ?1",
        params![id, taken.files as i64, taken.dirs as i64],
    )?;
    tx.commit()?;
    let root = conn
        .query_row(
            &format!("SELECT {ROOT_COLS} FROM roots WHERE id = ?1"),
            [id],
            read_root,
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("root {id}")))?;
    Ok((root, taken))
}

/// `inner`'s components below `outer`, '/'-separated with `inner`'s own spelling. `inner`
/// lies below `outer` by folded keys, which may differ from the stored spelling in case or
/// normalization, so the components are counted rather than the prefix stripped.
fn rel_below(outer: &Path, inner: &Path) -> String {
    let depth = outer.components().count();
    inner
        .components()
        .skip(depth)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Moves root `inner_id`'s folders and files into root `root_id`, below `rel` (VFS-31 R2):
/// the folders between the new root and `rel` are created pending deep where missing,
/// every moved folder's `rel_path`, `path_key`, `dirs_fts` row and its files'
/// `file_paths_fts` rows are rewritten (the latter deleted by their old value first, schema
/// v3), and the inner root's own folder hangs from its new parent. Ids, file rows, links,
/// unreadable reasons and scan states are kept. The inner root row and its change cursor
/// are deleted.
fn take_in(
    tx: &Transaction<'_>,
    root_id: i64,
    inner_id: i64,
    rel: &str,
    taken: &mut TakenIn,
) -> Result<()> {
    let find = |rel: &str| -> Result<Option<i64>> {
        Ok(tx
            .prepare_cached("SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ?2")?
            .query_row(params![root_id, rel], |r| r.get(0))
            .optional()?)
    };
    let mut parent = find("")?.ok_or_else(|| Error::NotFound(format!("root {root_id}")))?;
    let parts: Vec<&str> = rel.split('/').collect();
    for i in 1..parts.len() {
        let prefix = parts[..i].join("/");
        parent = match find(&prefix)? {
            Some(id) => id,
            None => ingest::insert_dir(tx, root_id, parent, &prefix, 0)?,
        };
    }
    let moved: Vec<(i64, String, String)> = {
        let mut stmt = tx.prepare("SELECT id, rel_path, name_key FROM dirs WHERE root_id = ?1")?;
        let rows = stmt.query_map([inner_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // The files' path rows are deleted by their old value, before the folders change.
    ingest::delete_file_paths_fts(tx, "d.root_id = ?1", [inner_id])?;
    {
        let mut update = tx.prepare_cached(
            "UPDATE dirs SET root_id = ?2, rel_path = ?3, path_key = ?4, \
             parent_id = COALESCE(parent_id, ?5) WHERE id = ?1",
        )?;
        let mut fts_delete = tx.prepare_cached("DELETE FROM dirs_fts WHERE rowid = ?1")?;
        let mut fts_insert = tx.prepare_cached(
            "INSERT INTO dirs_fts(rowid, path_key, name_key) VALUES (?1, ?2, ?3)",
        )?;
        // Built after the folder's row is updated, from the same value `ingest` indexes.
        let mut file_paths_insert = tx.prepare_cached(&format!(
            "INSERT INTO file_paths_fts(rowid, path) SELECT f.id, {} \
             FROM files f JOIN dirs d ON d.id = f.dir_id WHERE f.dir_id = ?1",
            ingest::FILE_PATH_SQL
        ))?;
        for (id, old_rel, name_key) in moved {
            let new_rel = if old_rel.is_empty() {
                rel.to_string()
            } else {
                format!("{rel}/{old_rel}")
            };
            let path_key = fold_key(&new_rel);
            update.execute(params![id, root_id, new_rel, path_key, parent])?;
            fts_delete.execute([id])?;
            fts_insert.execute(params![id, path_key, name_key])?;
            taken.files += file_paths_insert.execute([id])? as u64;
            taken.dirs += 1;
        }
    }
    tx.execute("DELETE FROM roots WHERE id = ?1", [inner_id])?;
    crate::indexer::forget_cursor(tx, inner_id)?;
    // Root ids are reused: the inner root's pause goes with it (VFS-32 R1); its folders,
    // pending ones included, are the parent's now.
    set_paused_key(tx, inner_id, false)?;
    taken.roots += 1;
    Ok(())
}

/// Sets the root's status to `removing` (search ignores it at once); the indexer deletes its
/// rows. `NotFound` for an unknown id. The facade notifies the indexer afterwards.
pub(crate) fn remove_root(conn: &mut Connection, root_id: i64) -> Result<()> {
    let changed = conn.execute(
        "UPDATE roots SET status = ?2 WHERE id = ?1",
        [root_id, RootStatus::Removing.code()],
    )?;
    if changed == 0 {
        return Err(Error::NotFound(format!("root {root_id}")));
    }
    Ok(())
}

/// Writes or deletes `root_id`'s pause key (VFS-32 R1).
pub(crate) fn set_paused_key(conn: &Connection, root_id: i64, paused: bool) -> Result<()> {
    let key = format!(concat!(paused_prefix!(), "{}"), root_id);
    if paused {
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, '1')",
            [key],
        )?;
    } else {
        conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    }
    Ok(())
}

/// Pauses or resumes indexing of a root (VFS-32 R1). `NotFound` for an unknown root or one
/// being removed. The facade notifies the indexer afterwards.
pub(crate) fn set_root_paused(conn: &Connection, root_id: i64, paused: bool) -> Result<()> {
    let live: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM roots WHERE id = ?1 AND status <> ?2)",
        [root_id, RootStatus::Removing.code()],
        |r| r.get(0),
    )?;
    if !live {
        return Err(Error::NotFound(format!("root {root_id}")));
    }
    set_paused_key(conn, root_id, paused)
}

#[cfg(test)]
mod tests {
    use super::is_below;

    /// VFS-1 AC-4: containment is by whole path components; a shared name prefix is not.
    #[test]
    fn below_is_component_wise() {
        assert!(is_below("/a/fotos/2019", "/a/fotos"));
        assert!(!is_below("/a/fotos2", "/a/fotos"));
        assert!(!is_below("/a/fotos", "/a/fotos"));
        assert!(is_below("/a", "/"));
        assert!(is_below("c:/datos", "c:/"));
    }

    /// VFS-31 AC-5: a take-in reports what it moved, and the add, remove and removed records
    /// are `key=number` pairs with no folder or file name.
    #[test]
    fn root_records_hold_numbers_only() {
        use super::*;
        let tmp = tempfile::tempdir().unwrap();
        let secreto = dunce::canonicalize(tmp.path()).unwrap().join("Secreto");
        std::fs::create_dir_all(secreto.join("Fotos")).unwrap();
        let mut conn = crate::db::open_rw(&tmp.path().join("index.db")).unwrap();
        let (inner, _) = add_root(&mut conn, &secreto.join("Fotos")).unwrap();
        let tx = conn.transaction().unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [inner.id], |r| {
                r.get(0)
            })
            .unwrap();
        let boda = ingest::NewFile {
            name: "boda.jpg",
            size: 1,
            mtime: 1,
            lossy: false,
        };
        ingest::insert_file(&tx, top, &boda).unwrap();
        tx.commit().unwrap();
        let (root, taken) = add_root(&mut conn, &secreto).unwrap();
        assert_eq!(
            taken,
            TakenIn {
                roots: 1,
                dirs: 1,
                files: 1
            }
        );
        let ms = Duration::from_millis(7);
        let lines = [
            add_record(root.id, taken, ms),
            remove_record(root.id, ms),
            removed_record(root.id, 2, 1, ms),
        ];
        assert_eq!(
            lines[0],
            format!(
                "root_add id={} taken_roots=1 taken_dirs=1 taken_files=1 ms=7",
                root.id
            )
        );
        for line in &lines {
            let mut words = line.split(' ');
            assert!(words.next().unwrap().starts_with("root_"));
            for pair in words {
                let (key, value) = pair.split_once('=').expect(line);
                assert!(!key.is_empty() && value.parse::<u64>().is_ok(), "{line}");
            }
        }
    }
}
