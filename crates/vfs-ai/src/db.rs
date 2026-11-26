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
-- Saved chats (VFS-49 R10). `scope` and `view` are the shell's JSON; `answer` is the plain
-- answer text a later message sends back as an earlier turn (R4).
CREATE TABLE IF NOT EXISTS chats (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    scope TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_turns (
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    view TEXT NOT NULL,
    PRIMARY KEY (chat_id, ord)
);
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

/// A saved chat, as the chat list shows it (VFS-49 R2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub scope: String,
}

/// One saved message of a chat and its answer (VFS-49 R10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurnRow {
    pub ord: i64,
    pub question: String,
    pub answer: String,
    pub view: String,
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
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
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

    /// Saves a chat's first answered message as a new chat (VFS-49 R10): no chat is kept for a
    /// message that failed. Returns the chat's id.
    pub fn create_chat(
        &mut self,
        title: &str,
        scope: &str,
        turn: &ChatTurnRow,
        at: &DateTime<Local>,
    ) -> rusqlite::Result<i64> {
        let tx = self.conn.transaction()?;
        let now = utc(at);
        tx.execute(
            "INSERT INTO chats (title, created_at, updated_at, scope) VALUES (?1, ?2, ?2, ?3)",
            params![title, now, scope],
        )?;
        let id = tx.last_insert_rowid();
        insert_turn(&tx, id, 0, turn)?;
        tx.commit()?;
        Ok(id)
    }

    /// Appends an answered message to chat `id` and saves the chat's sources choice with it,
    /// in one transaction (VFS-49 R6, R10). Returns the message's position, or `None` when the
    /// chat was deleted.
    pub fn add_turn(
        &mut self,
        id: i64,
        scope: &str,
        turn: &ChatTurnRow,
        at: &DateTime<Local>,
    ) -> rusqlite::Result<Option<i64>> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE chats SET updated_at = ?2, scope = ?3 WHERE id = ?1",
            params![id, utc(at), scope],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let ord: i64 = tx.query_row(
            "SELECT coalesce(max(ord) + 1, 0) FROM chat_turns WHERE chat_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        insert_turn(&tx, id, ord, turn)?;
        tx.commit()?;
        Ok(Some(ord))
    }

    /// Every chat, latest activity first (VFS-49 R2).
    pub fn chats(&self) -> rusqlite::Result<Vec<ChatRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, title, created_at, updated_at, scope FROM chats
             ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(ChatRow {
                id: r.get(0)?,
                title: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
                scope: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Chat `id`, or `None` when it was deleted.
    pub fn chat(&self, id: i64) -> rusqlite::Result<Option<ChatRow>> {
        self.conn
            .query_row(
                "SELECT id, title, created_at, updated_at, scope FROM chats WHERE id = ?1",
                [id],
                |r| {
                    Ok(ChatRow {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        created_at: r.get(2)?,
                        updated_at: r.get(3)?,
                        scope: r.get(4)?,
                    })
                },
            )
            .optional()
    }

    /// The messages of chat `id`, oldest first.
    pub fn chat_turns(&self, id: i64) -> rusqlite::Result<Vec<ChatTurnRow>> {
        let mut st = self.conn.prepare(
            "SELECT ord, question, answer, view FROM chat_turns WHERE chat_id = ?1 ORDER BY ord",
        )?;
        let rows = st.query_map([id], |r| {
            Ok(ChatTurnRow {
                ord: r.get(0)?,
                question: r.get(1)?,
                answer: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// «Renombrar» (VFS-49 R2).
    pub fn rename_chat(&self, id: i64, title: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE chats SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    /// «Borrar» on one chat (VFS-49 R10). The record and the month's spend stay; the space
    /// of its text is reclaimed, so what it quoted does not linger in free pages.
    pub fn delete_chat(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM chats WHERE id = ?1", [id])?;
        self.conn
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
    }

    /// «Borrar todos los chats» (VFS-49 R10). The record and the month's spend stay.
    pub fn clear_chats(&mut self) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM chat_turns; DELETE FROM chats;")?;
        tx.commit()?;
        self.conn
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
    }
}

/// A chat's time as RFC 3339 in UTC, so the text order is the time order across a daylight
/// saving change (VFS-49 R2).
fn utc(at: &DateTime<Local>) -> String {
    at.with_timezone(&chrono::Utc).to_rfc3339()
}

fn insert_turn(
    tx: &rusqlite::Transaction<'_>,
    chat_id: i64,
    ord: i64,
    turn: &ChatTurnRow,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO chat_turns (chat_id, ord, question, answer, view) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![chat_id, ord, turn.question, turn.answer, turn.view],
    )?;
    Ok(())
}
