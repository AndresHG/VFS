//! The user's file filter (VFS-34): name patterns such as `*.tmp`, global for every root,
//! that keep matching files out of the index. A pattern matches a file's name only, never a
//! folder or a path (R2); the walker and the folder lister apply it per listed file with no
//! file-system call (R3). Adding a pattern purges the files already indexed in resumable
//! batches, with no walk (R4); removing one bumps the generation in VFS-33's per-root stamp,
//! so every root owes a full walk (R5).

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use unicode_normalization::UnicodeNormalization;

use crate::db::ingest;
use crate::error::{Error, PatternError, Result};
use crate::fold::fold_key;

/// `meta` key of the list, one JSON array of patterns as the user typed them (R1).
const LIST_KEY: &str = "user_exclusions";
/// `meta` key of the list's generation: bumped when a pattern is removed or the list is
/// restored, never when one is added (R5). Absent reads as 0.
const GEN_KEY: &str = "user_exclusions_gen";
/// `meta` key set while a purge is owed (R4).
const PURGE_KEY: &str = "user_exclusions_purge";
/// `meta` key of the last `files.id` the owed purge has done (R4).
const CURSOR_KEY: &str = "user_exclusions_cursor";
/// Files scanned per purge batch (R4).
pub(crate) const PURGE_BATCH: i64 = 10_000;

/// The list of a fresh install and of «Restablecer la lista» (R7, D3): temporary and
/// partial-download files that the built-in list (`walk::is_excluded`) does not cover. No
/// pattern matches a shortcut (`*.lnk`, `*.url`; VFS-18).
pub const DEFAULT_PATTERNS: &[&str] = &["*.tmp", "*.temp", "*.swp", "*.crdownload", "*.part"];

/// The form both a pattern and a name are compared in: NFC, then Unicode lowercase (R2).
fn normal(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.nfc().flat_map(char::to_lowercase).collect()
}

/// Whether `name` matches the glob `pat` (both in [`normal`] form, as chars): `*` is any run
/// of characters, `?` exactly one, anything else is literal.
fn glob(pat: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    // Where the last `*` was, and the name position it currently stands for.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pat.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

/// The user's patterns, compiled for matching file names.
#[derive(Default)]
pub(crate) struct FilePatterns {
    pats: Vec<Vec<char>>,
}

impl FilePatterns {
    pub(crate) fn new<S: AsRef<str>>(list: &[S]) -> FilePatterns {
        FilePatterns {
            pats: list
                .iter()
                .map(|p| normal(p.as_ref()).chars().collect())
                .collect(),
        }
    }

    /// Whether the file `name` matches a pattern: one pass over the list, no I/O (R3).
    pub(crate) fn matches(&self, name: &str) -> bool {
        if self.pats.is_empty() {
            return false;
        }
        let name: Vec<char> = normal(name).chars().collect();
        self.pats.iter().any(|p| glob(p, &name))
    }
}

/// Checks a pattern the user is adding against `list` (R2). A pattern needs a literal
/// character other than `*`, `?` and `.`, so neither `*` nor `*.*` can hide every file.
/// A duplicate is one equal to a listed pattern after NFC and case folding.
pub(crate) fn validate(pattern: &str, list: &[String]) -> std::result::Result<(), PatternError> {
    if pattern.is_empty() {
        return Err(PatternError::Empty);
    }
    if pattern.contains(['/', '\\']) {
        return Err(PatternError::Separator);
    }
    if pattern.chars().all(|c| matches!(c, '*' | '?' | '.')) {
        return Err(PatternError::NoLiteral);
    }
    let n = normal(pattern);
    if list.iter().any(|p| normal(p) == n) {
        return Err(PatternError::Duplicate);
    }
    Ok(())
}

fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.prepare_cached("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?
        .execute([key, value])?;
    Ok(())
}

fn delete_meta(conn: &Connection, key: &str) -> Result<()> {
    conn.prepare_cached("DELETE FROM meta WHERE key = ?1")?
        .execute([key])?;
    Ok(())
}

/// The stored list, or [`DEFAULT_PATTERNS`] when none is stored (a fresh install, R7). An
/// unreadable value also reads as the defaults.
pub(crate) fn list(conn: &Connection) -> Result<Vec<String>> {
    Ok(match meta(conn, LIST_KEY)? {
        Some(v) => serde_json::from_str(&v).unwrap_or_else(|e| {
            log::warn!("{LIST_KEY}: {e}");
            defaults()
        }),
        None => defaults(),
    })
}

fn defaults() -> Vec<String> {
    DEFAULT_PATTERNS.iter().map(|p| p.to_string()).collect()
}

fn store(conn: &Connection, list: &[String]) -> Result<()> {
    let json = serde_json::to_string(list).expect("a list of strings serializes");
    set_meta(conn, LIST_KEY, &json)
}

/// The compiled list in force.
pub(crate) fn load(conn: &Connection) -> Result<FilePatterns> {
    Ok(FilePatterns::new(&list(conn)?))
}

/// The list's generation (R5); 0 for a fresh list.
pub(crate) fn generation(conn: &Connection) -> Result<i64> {
    Ok(meta(conn, GEN_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

fn bump_generation(conn: &Connection) -> Result<()> {
    let next = generation(conn)? + 1;
    set_meta(conn, GEN_KEY, &next.to_string())
}

/// Owes a purge from the first file: a purge already running restarts, so the files it
/// passed are checked against the new pattern too (R4).
fn owe_purge(conn: &Connection) -> Result<()> {
    set_meta(conn, PURGE_KEY, "1")?;
    set_meta(conn, CURSOR_KEY, "0")
}

/// Adds `pattern` (trimmed) after [`validate`], and owes the purge of the files it matches
/// (R2, R4). Returns the stored pattern.
pub(crate) fn add(conn: &Connection, pattern: &str) -> Result<String> {
    let pattern = pattern.trim();
    let mut all = list(conn)?;
    validate(pattern, &all).map_err(Error::InvalidPattern)?;
    all.push(pattern.to_owned());
    store(conn, &all)?;
    owe_purge(conn)?;
    Ok(pattern.to_owned())
}

/// Removes `pattern` (exact text); bumps the generation when it was listed (R5). Returns
/// whether it was.
pub(crate) fn remove(conn: &Connection, pattern: &str) -> Result<bool> {
    let mut all = list(conn)?;
    let before = all.len();
    all.retain(|p| p != pattern);
    if all.len() == before {
        return Ok(false);
    }
    store(conn, &all)?;
    bump_generation(conn)?;
    Ok(true)
}

/// Restores [`DEFAULT_PATTERNS`] and bumps the generation: the walk it owes both brings
/// back files no longer excluded and drops files a restored default matches (R5, R7).
pub(crate) fn reset(conn: &Connection) -> Result<()> {
    store(conn, &defaults())?;
    bump_generation(conn)
}

/// On an index that never stored a list (a fresh install, or one from before VFS-34),
/// stores the defaults and owes their purge, so files indexed before they applied leave the
/// index with no walk (R4, R7). The generation stays 0, so no root goes stale (R5). Run in
/// a write transaction, as [`add`] may run at the same time.
pub(crate) fn init(conn: &Connection) -> Result<()> {
    if meta(conn, LIST_KEY)?.is_none() {
        store(conn, &defaults())?;
        owe_purge(conn)?;
    }
    Ok(())
}

/// Indexed files `pattern` matches, for the confirmation before adding it (R2). SQLite
/// narrows the rows with a `LIKE` on the folded name, which every true match passes, and
/// the exact match runs on those only.
pub(crate) fn count_matches(conn: &Connection, pattern: &str) -> Result<u64> {
    let pattern = pattern.trim();
    validate(pattern, &list(conn)?).map_err(Error::InvalidPattern)?;
    let pats = FilePatterns::new(&[pattern]);
    let mut like = String::new();
    for c in pattern.chars() {
        match c {
            '*' | '?' => like.push('%'),
            c => {
                for f in fold_key(c.encode_utf8(&mut [0; 4])).chars() {
                    if matches!(f, '%' | '_' | '\\') {
                        like.push('\\');
                    }
                    like.push(f);
                }
            }
        }
    }
    let mut st = conn
        .prepare("SELECT name FROM files WHERE name_key LIKE ?1 ESCAPE '\\' AND flags & ?2 = 0")?;
    let mut rows = st.query(params![like, ingest::FLAG_LINK])?;
    let mut n = 0;
    while let Some(row) = rows.next()? {
        if pats.matches(&row.get::<_, String>(0)?) {
            n += 1;
        }
    }
    Ok(n)
}

/// Whether a purge is owed (R4).
pub(crate) fn purge_owed(conn: &Connection) -> Result<bool> {
    Ok(meta(conn, PURGE_KEY)?.is_some())
}

/// One batch of the owed purge: the next [`PURGE_BATCH`] files after the cursor, in `id`
/// order, whose names match the list are deleted, and the cursor moves past them. At the
/// end of `files` both keys are removed (R4). Link rows are left alone, as the lister never
/// matches them. Returns the files deleted, or `None` when no purge is owed.
pub(crate) fn purge_batch(tx: &Transaction) -> Result<Option<usize>> {
    if !purge_owed(tx)? {
        return Ok(None);
    }
    let pats = load(tx)?;
    let cursor: i64 = meta(tx, CURSOR_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut last = cursor;
    let mut scanned = 0;
    let mut doomed: Vec<(i64, i64)> = Vec::new();
    {
        let mut st = tx.prepare_cached(
            "SELECT f.id, f.name, f.flags, d.root_id FROM files f JOIN dirs d ON d.id = f.dir_id
             WHERE f.id > ?1 ORDER BY f.id LIMIT ?2",
        )?;
        let mut rows = st.query(params![cursor, PURGE_BATCH])?;
        while let Some(row) = rows.next()? {
            scanned += 1;
            last = row.get(0)?;
            let flags: i64 = row.get(2)?;
            if flags & ingest::FLAG_LINK == 0 && pats.matches(&row.get::<_, String>(1)?) {
                doomed.push((last, row.get(3)?));
            }
        }
    }
    for &(id, root_id) in &doomed {
        ingest::delete_file(tx, id)?;
        tx.prepare_cached("UPDATE roots SET files_count = max(files_count - 1, 0) WHERE id = ?1")?
            .execute([root_id])?;
    }
    if scanned < PURGE_BATCH {
        delete_meta(tx, PURGE_KEY)?;
        delete_meta(tx, CURSOR_KEY)?;
    } else {
        set_meta(tx, CURSOR_KEY, &last.to_string())?;
    }
    Ok(Some(doomed.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, name: &str) -> bool {
        FilePatterns::new(&[pat]).matches(name)
    }

    /// VFS-34 AC-1: `*` and `?` are the only wildcards; case is ignored; the whole name
    /// must match.
    #[test]
    fn wildcards_and_case() {
        assert!(m("*.swp", "A.SWP"));
        assert!(!m("*.swp", "a.swp.txt"));
        assert!(m("?.txt", "a.txt"));
        assert!(!m("?.txt", "ab.txt"));
        assert!(!m("?.txt", ".txt"));
        assert!(m("in*e*.pdf", "informe final.pdf"));
        assert!(!m("in*e*.pdf", "informe final.doc"));
        assert!(m("[1].txt", "[1].txt"), "brackets are literal");
        assert!(!m("[1].txt", "1.txt"));
        assert!(m("*", "anything"));
        assert!(m("~*", "~borrador"));
    }

    /// VFS-34 AC-1: a pattern written in NFC matches a name stored in NFD, both ways, and
    /// case folding covers non-ASCII letters.
    #[test]
    fn nfc_and_nfd_match() {
        let nfd = "Informe e\u{301}xito.DOC"; // "é" as e + combining acute
        assert!(m("*ÉXITO.doc", nfd));
        assert!(m("*e\u{301}xito.doc", "INFORME ÉXITO.doc"));
        assert!(!m("*exito.doc", nfd), "accents are not folded away");
    }

    /// VFS-34 AC-1: the add field rejects empty, separator, wildcard-only and duplicate
    /// patterns.
    #[test]
    fn validation() {
        let list = vec!["*.tmp".to_string()];
        assert_eq!(validate("", &list), Err(PatternError::Empty));
        assert_eq!(validate("a/b", &list), Err(PatternError::Separator));
        assert_eq!(validate("a\\b", &list), Err(PatternError::Separator));
        for p in ["*", "*.*", "?*", "**", "."] {
            assert_eq!(validate(p, &list), Err(PatternError::NoLiteral), "{p}");
        }
        assert_eq!(validate("*.tmp", &list), Err(PatternError::Duplicate));
        assert_eq!(validate("*.TMP", &list), Err(PatternError::Duplicate));
        assert_eq!(validate("*.bak", &list), Ok(()));
    }

    /// VFS-34 AC-6, R7: no default matches a shortcut, and each default is valid.
    #[test]
    fn defaults_spare_shortcuts() {
        let d = FilePatterns::new(DEFAULT_PATTERNS);
        for name in ["Acceso.lnk", "web.url", "nota.txt", "fotos.part.jpg"] {
            assert!(!d.matches(name), "{name}");
        }
        for name in [
            "video.mp4.part",
            "x.TMP",
            "a.swp",
            "informe.pdf.crdownload",
            "b.temp",
        ] {
            assert!(d.matches(name), "{name}");
        }
        for (i, p) in DEFAULT_PATTERNS.iter().enumerate() {
            let others: Vec<String> = DEFAULT_PATTERNS[..i]
                .iter()
                .map(|s| s.to_string())
                .collect();
            assert_eq!(validate(p, &others), Ok(()), "{p}");
        }
    }
}
