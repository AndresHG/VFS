//! Adding, removing and listing root folders (VFS-1 AC-4). Owned by task T2a.

use std::io::ErrorKind;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior};

use crate::db::ingest;
use crate::error::{Error, Result};
use crate::fold::fold_key;
use crate::platform;
use crate::types::{Root, RootStatus};

const ROOT_COLS: &str = "id, path, status, files_count, dirs_count, last_full_scan_at";

fn read_root(r: &Row<'_>) -> rusqlite::Result<Root> {
    Ok(Root {
        id: r.get(0)?,
        path: r.get(1)?,
        status: RootStatus::from_code(r.get(2)?),
        files: r.get::<_, i64>(3)?.max(0) as u64,
        dirs: r.get::<_, i64>(4)?.max(0) as u64,
        last_full_scan: r.get(5)?,
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

/// True when the folded path `path` lies strictly below the folded path `ancestor`.
fn is_below(path: &str, ancestor: &str) -> bool {
    path.strip_prefix(ancestor.trim_end_matches('/'))
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Canonicalizes `path` (`dunce`), rejects a missing path (`NotFound`), a file
/// (`NotADirectory`) and a path equal to, inside or containing an existing root
/// (`OverlappingRoot`), then inserts it with `db::ingest::insert_root` and
/// `platform::volume_key`. The facade notifies the indexer afterwards.
///
/// The `OverlappingRoot` payload is `"<relation> <existing root path>"`, the relation being
/// `same`, `inside` (the new folder is inside that root) or `contains` (the new folder
/// contains that root), so the UI can word each case (U4). Paths are compared on folded
/// keys, so case, accents and NFC/NFD differences still count as the same folder.
pub(crate) fn add_root(conn: &mut Connection, path: &Path) -> Result<Root> {
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
    {
        let mut stmt = tx.prepare("SELECT path, path_key FROM roots ORDER BY id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let (existing, existing_key): (String, String) = (row.get(0)?, row.get(1)?);
            let relation = if key.trim_end_matches('/') == existing_key.trim_end_matches('/') {
                "same"
            } else if is_below(&key, &existing_key) {
                "inside"
            } else if is_below(&existing_key, &key) {
                "contains"
            } else {
                continue;
            };
            return Err(Error::OverlappingRoot(format!("{relation} {existing}")));
        }
    }
    let id = ingest::insert_root(&tx, &platform::volume_key(&canon), &path_str)?;
    tx.commit()?;
    conn.query_row(
        &format!("SELECT {ROOT_COLS} FROM roots WHERE id = ?1"),
        [id],
        read_root,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("root {id}")))
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
}
