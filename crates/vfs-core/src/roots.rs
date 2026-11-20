//! Adding, removing and listing root folders (VFS-1 AC-4). Owned by task T2a.

use std::path::Path;

use rusqlite::Connection;

use crate::error::Result;
use crate::types::Root;

pub(crate) fn list_roots(_conn: &Connection) -> Result<Vec<Root>> {
    todo!("T2a")
}

/// Canonicalizes `path` (`dunce`), rejects a missing path (`NotFound`), a file
/// (`NotADirectory`) and a path equal to, inside or containing an existing root
/// (`OverlappingRoot`), then inserts it with `db::ingest::insert_root` and
/// `platform::volume_key`. The facade notifies the indexer afterwards.
pub(crate) fn add_root(_conn: &mut Connection, _path: &Path) -> Result<Root> {
    todo!("T2a")
}

/// Sets the root's status to `removing` (search ignores it at once); the indexer deletes its
/// rows. `NotFound` for an unknown id. The facade notifies the indexer afterwards.
pub(crate) fn remove_root(_conn: &mut Connection, _root_id: i64) -> Result<()> {
    todo!("T2a")
}
