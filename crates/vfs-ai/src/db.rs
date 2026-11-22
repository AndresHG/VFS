//! `ai.db`: the record of what each question sent, the monthly totals and the cap (VFS-28 R10,
//! R11). It lives in the app data folder, never in `index.db` or `content.db`, and never holds
//! a key (R2).

use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::{params, Connection, OptionalExtension};

use crate::cost::{Usage, NANO_PER_USD};
use crate::provider::Provider;

/// The default monthly cap: 5 $ (D4).
pub const DEFAULT_CAP_NANO: i64 = 5 * NANO_PER_USD;

/// The `settings` key of the monthly cap.
const CAP_KEY: &str = "monthly_cap_nano";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS questions (
    id INTEGER PRIMARY KEY,
    asked_at TEXT NOT NULL,
    provider TEXT NOT NULL,
    question TEXT NOT NULL,
    rewrite TEXT
);
CREATE TABLE IF NOT EXISTS calls (
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    kind TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cost_nano INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS passages (
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    file_name TEXT NOT NULL,
    page INTEGER,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS calls_by_question ON calls(question_id);
CREATE INDEX IF NOT EXISTS passages_by_question ON passages(question_id);
-- Kept apart from the record so «Borrar todo» does not reset the month's spend (R10).
CREATE TABLE IF NOT EXISTS month_totals (month TEXT PRIMARY KEY, cost_nano INTEGER NOT NULL);
";

/// Which call of a question a cost belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Rewrite,
    Answer,
    FollowUp,
}

impl CallKind {
    fn id(self) -> &'static str {
        match self {
            CallKind::Rewrite => "rewrite",
            CallKind::Answer => "answer",
            CallKind::FollowUp => "follow_up",
        }
    }

    fn from_id(id: &str) -> CallKind {
        match id {
            "rewrite" => CallKind::Rewrite,
            "follow_up" => CallKind::FollowUp,
            _ => CallKind::Answer,
        }
    }
}

/// One priced call (R10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub kind: CallKind,
    pub model: String,
    pub usage: Usage,
    pub cost_nano: i64,
}

/// A passage exactly as sent (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentPassage {
    pub file_name: String,
    pub page: Option<u32>,
    pub text: String,
}

/// What one question or follow-up sent (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentRecord {
    pub asked_at: DateTime<Local>,
    pub provider: Provider,
    pub question: String,
    /// The rewrite's output as returned; `None` for a follow-up, which has no rewrite (R9).
    pub rewrite: Option<String>,
    pub passages: Vec<SentPassage>,
    pub calls: Vec<CallRecord>,
}

/// A record read back, with its total cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    pub id: i64,
    /// RFC 3339 with the local offset at the time of asking.
    pub asked_at: String,
    pub provider: Provider,
    pub question: String,
    pub rewrite: Option<String>,
    pub passages: Vec<SentPassage>,
    pub calls: Vec<CallRecord>,
    pub cost_nano: i64,
}

/// The month's spend against the cap (R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapStatus {
    pub spent_nano: i64,
    pub cap_nano: i64,
}

impl CapStatus {
    /// True once the month's total has reached the cap: ask before each further question.
    pub fn reached(&self) -> bool {
        self.spent_nano >= self.cap_nano
    }
}

/// The calendar month of `at` in local time, `YYYY-MM` (R10).
fn month_of(at: &DateTime<Local>) -> String {
    at.format("%Y-%m").to_string()
}

pub struct AiDb {
    conn: Connection,
}

impl AiDb {
    /// Opens or creates `ai.db` at `path`, in WAL mode.
    pub fn open(path: &Path) -> rusqlite::Result<AiDb> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(SCHEMA)?;
        Ok(AiDb { conn })
    }

    /// Stores what a question sent and adds its cost to its month, in one transaction, so a
    /// question cut halfway leaves no record (R11).
    pub fn record(&mut self, record: &SentRecord) -> rusqlite::Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO questions (asked_at, provider, question, rewrite) VALUES (?1, ?2, ?3, ?4)",
            params![
                record.asked_at.to_rfc3339(),
                record.provider.id(),
                record.question,
                record.rewrite
            ],
        )?;
        let id = tx.last_insert_rowid();
        let mut total = 0i64;
        for (ord, call) in record.calls.iter().enumerate() {
            let u = &call.usage;
            tx.execute(
                "INSERT INTO calls (question_id, ord, kind, model, input_tokens, output_tokens,
                 cache_read_tokens, cache_write_tokens, cost_nano)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    ord as i64,
                    call.kind.id(),
                    call.model,
                    u.input_tokens as i64,
                    u.output_tokens as i64,
                    u.cache_read_tokens as i64,
                    u.cache_write_tokens as i64,
                    call.cost_nano
                ],
            )?;
            total = total.saturating_add(call.cost_nano);
        }
        for (ord, p) in record.passages.iter().enumerate() {
            tx.execute(
                "INSERT INTO passages (question_id, ord, file_name, page, text)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, ord as i64, p.file_name, p.page, p.text],
            )?;
        }
        tx.execute(
            "INSERT INTO month_totals (month, cost_nano) VALUES (?1, ?2)
             ON CONFLICT(month) DO UPDATE SET cost_nano = cost_nano + excluded.cost_nano",
            params![month_of(&record.asked_at), total],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Every record, oldest first (R11, «Qué se ha enviado»).
    pub fn records(&self) -> rusqlite::Result<Vec<StoredRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, asked_at, provider, question, rewrite FROM questions ORDER BY id",
        )?;
        let heads = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut calls_stmt = self.conn.prepare(
            "SELECT kind, model, input_tokens, output_tokens, cache_read_tokens,
             cache_write_tokens, cost_nano FROM calls WHERE question_id = ?1 ORDER BY ord",
        )?;
        let mut passages_stmt = self.conn.prepare(
            "SELECT file_name, page, text FROM passages WHERE question_id = ?1 ORDER BY ord",
        )?;
        let mut out = Vec::with_capacity(heads.len());
        for (id, asked_at, provider, question, rewrite) in heads {
            let calls = calls_stmt
                .query_map([id], |r| {
                    Ok(CallRecord {
                        kind: CallKind::from_id(&r.get::<_, String>(0)?),
                        model: r.get(1)?,
                        usage: Usage {
                            input_tokens: r.get::<_, i64>(2)? as u64,
                            output_tokens: r.get::<_, i64>(3)? as u64,
                            cache_read_tokens: r.get::<_, i64>(4)? as u64,
                            cache_write_tokens: r.get::<_, i64>(5)? as u64,
                        },
                        cost_nano: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let passages = passages_stmt
                .query_map([id], |r| {
                    Ok(SentPassage {
                        file_name: r.get(0)?,
                        page: r.get(1)?,
                        text: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push(StoredRecord {
                id,
                asked_at,
                provider: Provider::from_id(&provider).unwrap_or(Provider::Anthropic),
                question,
                rewrite,
                passages,
                cost_nano: calls.iter().map(|c| c.cost_nano).sum(),
                calls,
            });
        }
        Ok(out)
    }

    /// «Borrar todo» (R11): empties the record. The month totals stay, so the cap still holds.
    pub fn clear_records(&mut self) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM passages; DELETE FROM calls; DELETE FROM questions;")?;
        tx.commit()?;
        // Reclaim the space of the deleted text on disk.
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
    }

    /// The total spent in the calendar month of `at`, local time (R10).
    pub fn month_total(&self, at: &DateTime<Local>) -> rusqlite::Result<i64> {
        self.conn
            .query_row(
                "SELECT cost_nano FROM month_totals WHERE month = ?1",
                [month_of(at)],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.unwrap_or(0))
    }

    /// The monthly cap in nano-dollars; 5 $ until the user edits it (R10).
    pub fn cap(&self) -> rusqlite::Result<i64> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [CAP_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_CAP_NANO))
    }

    /// Sets the monthly cap from Ajustes (R10). Negative values are stored as zero.
    pub fn set_cap(&self, cap_nano: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![CAP_KEY, cap_nano.max(0).to_string()],
        )?;
        Ok(())
    }

    /// The month's spend against the cap, for the confirmation before a question (R10).
    pub fn cap_status(&self, at: &DateTime<Local>) -> rusqlite::Result<CapStatus> {
        Ok(CapStatus {
            spent_nano: self.month_total(at)?,
            cap_nano: self.cap()?,
        })
    }
}
