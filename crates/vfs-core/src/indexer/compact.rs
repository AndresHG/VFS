//! Gives back the space that large deletions leave in the index (VFS-40).
//!
//! `files_fts` and `file_paths_fts` delete rows with FTS5's `'delete'` command, which writes
//! a tombstone next to the entry it cancels; both stay until their segments are merged, and
//! automerge never merges the small tombstone segments into the big old ones. Deletions are
//! counted in `meta`; once enough have piled up, the idle indexer merges every FTS table in
//! small slices, then vacuums when that is cheap.

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;

/// `meta` key: files whose FTS entries were deleted since the last compaction (R1).
pub(crate) const DELETED_KEY: &str = "fts_deleted";
/// Compaction is due once this many deletions, or a tenth of the indexed files if more,
/// have piled up (R2, D2).
pub(crate) const FLOOR: u64 = 50_000;
/// FTS5 merge argument of one slice: negative merges regardless of `automerge`; small, so a
/// slice holds the write lock briefly (VFS-43 R1, D1).
const MERGE_STEP: i64 = -16;
/// Tables merged, in this order: the largest first.
const TABLES: [&str; 3] = ["file_paths_fts", "files_fts", "dirs_fts"];
/// `VACUUM` limits (R3): at least this much free…
pub(crate) const VACUUM_MIN_FREE: u64 = 64 << 20;
/// …and at most this much in use.
const VACUUM_MAX_USED: u64 = 256 << 20;
/// Page cache of the `VACUUM`, KiB (D4).
const VACUUM_CACHE_KIB: i64 = 2048;
/// An index from 0.3.0 or earlier is taken as bloated when it uses more than this per file
/// plus `SEED_SLACK` (R4).
const SEED_BYTES_PER_FILE: u64 = 1024;
const SEED_SLACK: u64 = 64 << 20;

/// Adds `n` deleted files to the counter, in the caller's transaction (R1).
pub(crate) fn record_deleted(conn: &Connection, n: usize) -> Result<()> {
    if n == 0 {
        return Ok(());
    }
    conn.prepare_cached(
        "INSERT INTO meta(key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + excluded.value",
    )?
    .execute(rusqlite::params![DELETED_KEY, n as i64])?;
    Ok(())
}

fn deleted(conn: &Connection) -> Result<Option<u64>> {
    Ok(conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = ?1",
            [DELETED_KEY],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .map(|v| v.max(0) as u64))
}

fn indexed_files(conn: &Connection) -> Result<u64> {
    let n: i64 = conn.query_row("SELECT COALESCE(SUM(files_count), 0) FROM roots", [], |r| {
        r.get(0)
    })?;
    Ok(n.max(0) as u64)
}

/// (page size, page count, free pages).
fn pages(conn: &Connection) -> Result<(u64, u64, u64)> {
    let get = |p: &str| -> Result<u64> {
        let v: i64 = conn.query_row(&format!("PRAGMA {p}"), [], |r| r.get(0))?;
        Ok(v.max(0) as u64)
    };
    Ok((
        get("page_size")?,
        get("page_count")?,
        get("freelist_count")?,
    ))
}

fn threshold(files: u64, floor: u64) -> u64 {
    floor.max(files / 10)
}

/// The counter of an index from 0.3.0 or earlier, which has none (R4).
fn seed(used_bytes: u64, files: u64, floor: u64) -> u64 {
    if used_bytes > files.saturating_mul(SEED_BYTES_PER_FILE) + SEED_SLACK {
        threshold(files, floor)
    } else {
        0
    }
}

/// Once per indexer start, before any walk: an index that uses more than R4's bound is owed a
/// compaction even when its counter exists but is below the threshold. The first deletion of
/// an index from 0.3.0 or earlier created the counter before R4 could seed it, so such an
/// index was never compacted (VFS-51 R1). Never lowers the counter.
pub(crate) fn reseed(conn: &Connection, floor: u64) -> Result<()> {
    let files = indexed_files(conn)?;
    let (size, count, free) = pages(conn)?;
    let used = size * count.saturating_sub(free);
    if let Some(n) = reseeded(deleted(conn)?, used, files, floor) {
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            rusqlite::params![DELETED_KEY, n as i64],
        )?;
    }
    Ok(())
}

/// The counter `reseed` writes, if any: R4's seed when it is above the current counter.
fn reseeded(current: Option<u64>, used_bytes: u64, files: u64, floor: u64) -> Option<u64> {
    let owed = seed(used_bytes, files, floor);
    (current.is_none() || current.unwrap_or(0) < owed).then_some(owed.max(current.unwrap_or(0)))
}

/// Whether compaction is due: the counter, seeded first when missing, has reached the
/// threshold. Returns the counter when it is due.
pub(crate) fn due(conn: &Connection, floor: u64) -> Result<Option<u64>> {
    let files = indexed_files(conn)?;
    let n = match deleted(conn)? {
        Some(n) => n,
        None => {
            let (size, count, free) = pages(conn)?;
            let n = seed(size * count.saturating_sub(free), files, floor);
            conn.execute(
                "INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)",
                rusqlite::params![DELETED_KEY, n as i64],
            )?;
            n
        }
    };
    Ok((n >= threshold(files, floor)).then_some(n))
}

/// One merge slice in its own write transaction: the first table with work left gets one
/// `('merge', MERGE_STEP)`. Returns false once no table had any work.
pub(crate) fn merge_slice(conn: &Connection) -> Result<bool> {
    for table in TABLES {
        let tx = super::begin(conn)?;
        let before = tx.total_changes();
        tx.execute(
            &format!("INSERT INTO {table}({table}, rank) VALUES('merge', ?1)"),
            [MERGE_STEP],
        )?;
        // FTS5: fewer than 2 changes means the merge found nothing to do.
        let worked = tx.total_changes() - before >= 2;
        tx.commit()?;
        if worked {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a `VACUUM` is worth it for an index of `size`-byte pages, `count` pages and
/// `free` free pages, needing `min_free` bytes free (R3, AC-4).
fn vacuum_worth(size: u64, count: u64, free: u64, min_free: u64) -> bool {
    let (free, used) = (size * free, size * count.saturating_sub(free));
    free >= min_free && free >= used && used <= VACUUM_MAX_USED
}

/// Ends a compaction once no merge work is left: takes the deletions counted when it began
/// off the counter, vacuums when worth it (`min_free`: `VACUUM_MIN_FREE`, smaller in tests),
/// and returns `(vacuumed, bytes before, after)`. Only a failure to settle the counter is an
/// error: once it is settled, the compaction is over whatever the `VACUUM` does, so a retry
/// never takes the same deletions off twice.
pub(crate) fn finish(
    conn: &Connection,
    started_at: u64,
    min_free: u64,
) -> Result<(bool, u64, u64)> {
    let tx = super::begin(conn)?;
    let left = deleted(&tx)?.unwrap_or(0).saturating_sub(started_at);
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
        rusqlite::params![DELETED_KEY, left as i64],
    )?;
    tx.commit()?;
    match shrink(conn, min_free) {
        Ok(r) => Ok(r),
        Err(e) => {
            // The merge already removed the dead entries; the free pages are reused as the
            // index grows.
            log::warn!("compaction: vacuum: {e}");
            let size = pages(conn).map_or(0, |(size, count, _)| size * count);
            Ok((false, size, size))
        }
    }
}

/// `VACUUM` when worth it (R3); returns `(vacuumed, bytes before, after)`.
fn shrink(conn: &Connection, min_free: u64) -> Result<(bool, u64, u64)> {
    let (size, count, free) = pages(conn)?;
    let before = size * count;
    if !vacuum_worth(size, count, free, min_free) {
        return Ok((false, before, before));
    }
    // The temporary copy goes to a file, and both databases get a small page cache: with
    // the writer's 32 MB cache and memory temp storage, the `VACUUM` of a 62 MB index peaked
    // at 141 MB RSS; this way at 29 MB, no slower (D4).
    let cache: i64 = conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;
    let temp: i64 = conn.query_row("PRAGMA temp_store", [], |r| r.get(0))?;
    let vacuum = conn
        .execute_batch(&format!(
            "PRAGMA temp_store=FILE; PRAGMA cache_size=-{VACUUM_CACHE_KIB};"
        ))
        .and_then(|()| conn.execute_batch("VACUUM;"));
    conn.execute_batch(&format!(
        "PRAGMA temp_store={temp}; PRAGMA cache_size={cache};"
    ))?;
    vacuum?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    let (size, count, _) = pages(conn)?;
    Ok((true, before, size * count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, ingest};

    const MIB: u64 = 1 << 20;
    const PAGE: u64 = 16384;

    /// VFS-40 AC-4: the `VACUUM` decision follows R3's three limits.
    #[test]
    fn vacuum_only_when_much_is_free_and_little_is_used() {
        let pages = |mb: u64| mb * MIB / PAGE;
        // The owner's index after merging: 1,840 MB free, 62 MB used.
        assert!(vacuum_worth(
            PAGE,
            pages(1902),
            pages(1840),
            VACUUM_MIN_FREE
        ));
        // Less than 64 MiB free.
        assert!(!vacuum_worth(PAGE, pages(100), pages(60), VACUUM_MIN_FREE));
        // Fewer free than used pages.
        assert!(!vacuum_worth(PAGE, pages(250), pages(100), VACUUM_MIN_FREE));
        // More than 256 MiB used, however much is free.
        assert!(!vacuum_worth(
            PAGE,
            pages(2300),
            pages(2000),
            VACUUM_MIN_FREE
        ));
    }

    /// VFS-40 AC-5: an index without a counter is seeded at the threshold when it uses more
    /// than 1 KiB per file plus 64 MiB, and at 0 otherwise.
    #[test]
    fn seed_flags_bloated_indexes_only() {
        // The owner's index: 1.86 GB used for 228 k files.
        assert_eq!(seed(1860 * MIB, 228_000, FLOOR), FLOOR);
        // A healthy 5 M-file index: about 1.8 GB.
        assert_eq!(seed(1800 * MIB, 5_000_000, FLOOR), 0);
        // A bloated large index gets a tenth of its files.
        assert_eq!(seed(9000 * MIB, 5_000_000, FLOOR), 500_000);
        // An empty index.
        assert_eq!(seed(5 * MIB, 0, FLOOR), 0);
    }

    /// Adds a root of `files` files, 100 per folder, and records its count as a scan would.
    fn add_root(conn: &mut Connection, path: &str, files: usize) -> i64 {
        let tx = conn.transaction().unwrap();
        let root_id = ingest::insert_root(&tx, "test", path).unwrap();
        let top: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root_id],
                |r| r.get(0),
            )
            .unwrap();
        let mut dir = top;
        for i in 0..files {
            if i % 100 == 0 {
                let rel = format!("carpeta {} de trabajo/año {}", i / 100, i / 1000);
                dir = ingest::insert_dir(&tx, root_id, top, &rel, 0).unwrap();
            }
            let name = format!("informe {i} del proyecto número {}.pdf", i % 37);
            let f = ingest::NewFile {
                name: &name,
                size: 1,
                mtime: 0,
                lossy: false,
            };
            ingest::insert_file(&tx, dir, &f).unwrap();
        }
        tx.execute(
            "UPDATE roots SET files_count = ?2 WHERE id = ?1",
            rusqlite::params![root_id, files as i64],
        )
        .unwrap();
        tx.commit().unwrap();
        root_id
    }

    /// VFS-51 AC-1 (R1): an index from 0.3.0 or earlier whose counter a first deletion
    /// created small (the owner's Mac: 1.9 GB for 236 k files, counter 8,320) is owed a
    /// compaction; a compact index, or a counter already at or above the seed, is left alone.
    #[test]
    fn reseed_owes_compaction_to_a_bloated_index_with_a_small_counter() {
        assert_eq!(
            reseeded(Some(8_320), 1910 * MIB, 236_000, FLOOR),
            Some(FLOOR)
        );
        assert_eq!(reseeded(Some(8_320), 62 * MIB, 236_000, FLOOR), None);
        assert_eq!(reseeded(Some(80_000), 1910 * MIB, 236_000, FLOOR), None);
        assert_eq!(reseeded(None, 62 * MIB, 236_000, FLOOR), Some(0));
    }

    /// VFS-51 AC-2 (R1): `reseed` runs at indexer start and writes the counter.
    #[test]
    fn reseed_writes_the_counter() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db::open_rw(&dir.path().join("index.db")).unwrap();
        add_root(&mut conn, "/raíz", 200);
        reseed(&conn, FLOOR).unwrap();
        assert_eq!(deleted(&conn).unwrap(), Some(0));
        record_deleted(&conn, 30).unwrap();
        reseed(&conn, FLOOR).unwrap();
        assert_eq!(deleted(&conn).unwrap(), Some(30), "never lowered");
    }

    /// Bytes of the index's pages. On Windows the file itself grows and shrinks in 32 MB
    /// chunks (VFS-5 R10), so its length on disk is not comparable.
    fn page_bytes(conn: &Connection) -> u64 {
        let (size, count, _) = pages(conn).unwrap();
        size * count
    }

    fn data_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM file_paths_fts_data", [], |r| r.get(0))
            .unwrap()
    }

    /// Rowids each FTS table returns for a few trigram queries.
    fn answers(conn: &Connection) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        for (table, q) in [
            ("files_fts", "inf"),
            ("files_fts", "úme"),
            ("file_paths_fts", "arp AND año"),
            ("file_paths_fts", "pdf AND rpe"),
            ("dirs_fts", "tra"),
        ] {
            let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rowid");
            let mut stmt = conn.prepare(&sql).unwrap();
            let rows = stmt.query_map([q], |r| r.get(0)).unwrap();
            out.push(rows.collect::<rusqlite::Result<Vec<i64>>>().unwrap());
        }
        out
    }

    /// Merges slice by slice until no work is left; returns the slices run.
    fn merge_all(conn: &Connection) -> u32 {
        let mut slices = 0;
        while merge_slice(conn).unwrap() {
            slices += 1;
        }
        slices
    }

    /// VFS-40 AC-1: after a large root is removed, compaction merges the dead FTS entries
    /// away and vacuums the index to about the size of one built from the remaining root
    /// alone; searches answer as before and the counter is back to 0.
    #[test]
    fn removal_is_compacted_to_the_remaining_size() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");
        let mut conn = db::open_rw(&db_path).unwrap();
        let big = add_root(&mut conn, "/grande", 20_000);
        add_root(&mut conn, "/pequeña", 2_000);
        crate::roots::remove_root(&mut conn, big).unwrap();
        assert!(super::super::finish_removing(&conn, &|| false).unwrap());
        assert_eq!(deleted(&conn).unwrap(), Some(20_000));
        let before = answers(&conn);
        let rows_before = data_rows(&conn);

        let started = due(&conn, 1).unwrap().expect("compaction due");
        assert!(merge_all(&conn) > 0);
        let (vacuumed, _, after) = finish(&conn, started, 0).unwrap();
        assert!(vacuumed);
        assert!(
            data_rows(&conn) * 3 < rows_before,
            "dead entries merged away"
        );
        assert_eq!(answers(&conn), before);
        assert_eq!(deleted(&conn).unwrap(), Some(0));

        let fresh_path = dir.path().join("fresh.db");
        let mut fresh = db::open_rw(&fresh_path).unwrap();
        add_root(&mut fresh, "/pequeña", 2_000);
        fresh
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        let (got, want) = (page_bytes(&conn), page_bytes(&fresh));
        assert_eq!(got, after);
        assert!(
            got * 4 <= want * 5,
            "{got} bytes against {want} for a fresh index"
        );
    }

    /// VFS-40 AC-2: below the threshold (the floor, or a tenth of the files if more) nothing
    /// is due, and no merge runs.
    #[test]
    fn few_deletions_are_not_compacted() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db::open_rw(&dir.path().join("index.db")).unwrap();
        let root = add_root(&mut conn, "/raíz", 3_000);
        assert_eq!(due(&conn, 1).unwrap(), None, "a fresh index seeds 0");
        let tx = conn.transaction().unwrap();
        let n = ingest::delete_files_fts(&tx, "f.id <= 299", []).unwrap();
        tx.commit().unwrap();
        assert_eq!(n, 299);
        // A tenth of 3,000 files is 300: one deletion short.
        assert_eq!(due(&conn, 1).unwrap(), None);
        assert_eq!(due(&conn, FLOOR).unwrap(), None);
        let tx = conn.transaction().unwrap();
        ingest::delete_files_fts(&tx, "f.id = 300", []).unwrap();
        tx.commit().unwrap();
        assert_eq!(due(&conn, 1).unwrap(), Some(300));
        assert_eq!(due(&conn, FLOOR).unwrap(), None);
        let _ = root;
    }

    /// VFS-40 AC-3: compaction stopped between slices (the connection closed, as on a stop)
    /// is finished by a later run on a new connection, with the same answers.
    #[test]
    fn interrupted_merge_finishes_later() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");
        let mut conn = db::open_rw(&db_path).unwrap();
        let big = add_root(&mut conn, "/grande", 10_000);
        add_root(&mut conn, "/pequeña", 1_000);
        crate::roots::remove_root(&mut conn, big).unwrap();
        assert!(super::super::finish_removing(&conn, &|| false).unwrap());
        let before = answers(&conn);
        let started = due(&conn, 1).unwrap().unwrap();
        assert!(merge_slice(&conn).unwrap());
        drop(conn);

        let conn = db::open_rw(&db_path).unwrap();
        assert_eq!(answers(&conn), before);
        assert_eq!(
            due(&conn, 1).unwrap(),
            Some(started),
            "still due after the stop"
        );
        merge_all(&conn);
        finish(&conn, started, 0).unwrap();
        assert_eq!(answers(&conn), before);
        assert_eq!(due(&conn, 1).unwrap(), None);
    }
}
