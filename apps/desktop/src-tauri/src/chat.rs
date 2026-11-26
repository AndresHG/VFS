//! The Chat tab's commands (VFS-49): send a message in a chat, list, open, rename and delete
//! chats, and the bibliography of one answer. Chats are kept in `ai.db` (R10); each message
//! runs the whole ask flow again over the chat's sources with its earlier turns (R4, R6, R7).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use vfs_ai::ask::{self, Document, LocalCorpus, Scope};
use vfs_ai::{chat_model, AiDb, AnswerEvent, ChatRow, ChatTurnRow, Turn};
use vfs_core::api::ApiError;

use crate::ai::{
    ai_error, api_error, ask_error, off_runtime, storage_error, AiErrorCode, AiState, BlockView,
    CitationView, SourceView, TextEvent, EVENT_ANSWER_TEXT,
};
use crate::bibliography::{self, BibEntry};
use crate::AppState;

/// A chat's title: its first question, cut at this many characters (R3).
pub const TITLE_CHARS: usize = 60;

/// One answered message as the screen draws it, kept as JSON in `ai.db` so a saved chat opens
/// without a model call (R8, R11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub question: String,
    /// Plain text runs with their markers (VFS-28 R7a, R8).
    pub blocks: Vec<BlockView>,
    /// No valid citation: the not-found sentence and the closest documents.
    pub not_found: bool,
    /// The model that answered.
    pub model: String,
    pub cost_nano: i64,
    /// The passages sent, which the markers index.
    pub sources: Vec<SourceView>,
    /// The documents found, best first: the not-found list and «todos los encontrados».
    pub documents: Vec<Document>,
}

/// A chat in the list (R2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    pub id: i64,
    pub title: String,
    /// RFC 3339 in UTC: when the chat last got a message.
    pub updated_at: String,
    pub scope: Scope,
}

/// A saved chat, opened (R11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatView {
    pub id: i64,
    pub title: String,
    pub scope: Scope,
    pub turns: Vec<TurnView>,
}

/// A message answered: the chat it is in (new when the message started one), the turn, and the
/// month's spend after it (VFS-28 R10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSent {
    pub chat_id: i64,
    pub turn: TurnView,
    pub month_nano: i64,
    pub cap_nano: i64,
}

/// The first question as a title: one line, at most [`TITLE_CHARS`] characters (R3).
pub fn title_of(question: &str) -> String {
    let line = question.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    let cut: String = line.chars().take(TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

fn scope_json(scope: &Scope) -> String {
    serde_json::to_string(scope).unwrap_or_else(|_| "{\"kind\":\"all\"}".into())
}

/// A saved scope; one this build cannot read falls back to every document.
fn scope_of(json: &str) -> Scope {
    serde_json::from_str(json).unwrap_or_default()
}

fn summary(row: ChatRow) -> ChatSummary {
    ChatSummary {
        id: row.id,
        title: row.title,
        updated_at: row.updated_at,
        scope: scope_of(&row.scope),
    }
}

fn turn_view(question: &str, asked: &ask::Asked) -> TurnView {
    let reply = &asked.reply;
    TurnView {
        question: question.to_owned(),
        blocks: reply
            .blocks
            .iter()
            .map(|b| BlockView {
                text: b.text.clone(),
                citations: b
                    .citations
                    .iter()
                    .map(|c| CitationView {
                        passage: c.passage,
                        marker: c.marker,
                    })
                    .collect(),
            })
            .collect(),
        not_found: reply.not_found,
        model: reply.model.clone(),
        cost_nano: reply.cost_nano,
        sources: asked
            .thread
            .sources
            .iter()
            .map(|s| SourceView {
                file_id: s.file_id,
                name: s.name.clone(),
                pages: s.pages.clone(),
                text: s.text.clone(),
            })
            .collect(),
        documents: asked.thread.documents.clone(),
    }
}

/// The saved turns of a chat, read back; a row this build cannot read is skipped.
fn turns_of(db: &AiDb, id: i64) -> Result<Vec<(ChatTurnRow, TurnView)>, ApiError> {
    Ok(db
        .chat_turns(id)
        .map_err(storage_error)?
        .into_iter()
        .filter_map(|row| {
            let view = serde_json::from_str(&row.view).ok()?;
            Some((row, view))
        })
        .collect())
}

impl AiState {
    /// Sends `question` in chat `chat_id`, or in a new chat when `None` (R4, R6, R7, R10): the
    /// work of `ai_chat_send`, blocking. The chat and the turn are saved only once the answer
    /// is complete. `on_text` hears the answer as it streams.
    #[allow(clippy::too_many_arguments)]
    pub fn chat_send_over(
        &self,
        corpus: &dyn ask::Corpus,
        ask_id: u64,
        chat_id: Option<i64>,
        scope: &Scope,
        question: &str,
        confirmed: bool,
        lang: &str,
        on_text: &mut dyn FnMut(String),
    ) -> Result<ChatSent, ApiError> {
        let earlier: Vec<Turn> = match chat_id {
            None => Vec::new(),
            Some(id) => {
                let db = self.db()?;
                if db.get().chat(id).map_err(storage_error)?.is_none() {
                    return Err(api_error(AiErrorCode::NoChat, "chat not found"));
                }
                turns_of(db.get(), id)?
                    .into_iter()
                    .map(|(row, _)| Turn {
                        question: row.question,
                        answer: row.answer,
                    })
                    .collect()
            }
        };
        let opts = self.options(confirmed, lang);
        let key = self.key(opts.provider)?;
        let model =
            chat_model(opts.provider, key, self.endpoint(opts.provider)).map_err(ai_error)?;
        let cancel = self.start(ask_id);
        let result = {
            let mut db = self.db()?;
            ask::chat_turn(
                model.as_ref(),
                corpus,
                db.get_mut(),
                question,
                &earlier,
                scope,
                &opts,
                &cancel,
                &mut |AnswerEvent::Text(t)| on_text(t),
            )
        };
        self.finish(ask_id);
        let asked = result.map_err(ask_error)?;
        let turn = turn_view(question, &asked);
        let row = ChatTurnRow {
            ord: 0,
            question: question.to_owned(),
            answer: asked.reply.text(),
            view: serde_json::to_string(&turn).map_err(storage_error)?,
        };
        let now = chrono::Local::now();
        let mut db = self.db()?;
        let db = db.get_mut();
        let chat_id = match chat_id {
            None => db
                .create_chat(&title_of(question), &scope_json(scope), &row, &now)
                .map_err(storage_error)?,
            Some(id) => {
                db.add_turn(id, &scope_json(scope), &row, &now)
                    .map_err(storage_error)?
                    .ok_or_else(|| api_error(AiErrorCode::NoChat, "chat not found"))?;
                id
            }
        };
        let cap = db.cap_status(&now).map_err(storage_error)?;
        Ok(ChatSent {
            chat_id,
            turn,
            month_nano: cap.spent_nano,
            cap_nano: cap.cap_nano,
        })
    }

    /// Every chat, latest activity first (R2).
    pub fn chats(&self) -> Result<Vec<ChatSummary>, ApiError> {
        Ok(self
            .db()?
            .get()
            .chats()
            .map_err(storage_error)?
            .into_iter()
            .map(summary)
            .collect())
    }

    /// Chat `id` with its turns, from `ai.db` only (R11).
    pub fn chat(&self, id: i64) -> Result<ChatView, ApiError> {
        let db = self.db()?;
        let row = db
            .get()
            .chat(id)
            .map_err(storage_error)?
            .ok_or_else(|| api_error(AiErrorCode::NoChat, "chat not found"))?;
        let turns = turns_of(db.get(), id)?
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        Ok(ChatView {
            id: row.id,
            title: row.title,
            scope: scope_of(&row.scope),
            turns,
        })
    }

    /// «Renombrar» (R2): a blank title keeps the old one.
    pub fn rename_chat(&self, id: i64, title: &str) -> Result<(), ApiError> {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return Ok(());
        }
        self.db()?
            .get()
            .rename_chat(id, &title)
            .map_err(storage_error)
    }

    /// «Borrar» on one chat (R10).
    pub fn delete_chat(&self, id: i64) -> Result<(), ApiError> {
        self.db()?.get().delete_chat(id).map_err(storage_error)
    }

    /// «Borrar todos los chats» (R10).
    pub fn clear_chats(&self) -> Result<(), ApiError> {
        self.db()?.get_mut().clear_chats().map_err(storage_error)
    }

    /// The bibliography of turn `ord` of chat `id` (VFS-28 R12): the documents its answer cited,
    /// with the pages cited, or with `all` every document found.
    pub fn chat_bibliography(
        &self,
        id: i64,
        ord: i64,
        all: bool,
    ) -> Result<Vec<BibEntry>, ApiError> {
        let db = self.db()?;
        let turn = turns_of(db.get(), id)?
            .into_iter()
            .find(|(row, _)| row.ord == ord)
            .map(|(_, v)| v)
            .ok_or_else(|| api_error(AiErrorCode::NoChat, "turn not found"))?;
        let mut cited: BTreeMap<i64, BTreeSet<u32>> = BTreeMap::new();
        for c in turn.blocks.iter().flat_map(|b| &b.citations) {
            if let Some(s) = turn.sources.get(c.passage) {
                cited
                    .entry(s.file_id)
                    .or_default()
                    .extend(s.pages.iter().copied());
            }
        }
        Ok(turn
            .documents
            .iter()
            .filter(|d| all || cited.contains_key(&d.file_id))
            .map(|d| {
                let pages = cited
                    .get(&d.file_id)
                    .map(|p| p.iter().copied().collect())
                    .unwrap_or_default();
                bibliography::entry(d, pages)
            })
            .collect())
    }
}

/// Sends a message (R4–R7): in chat `chat_id`, or in a new chat when null, over `scope`. The
/// answer streams as `ai-answer-text` events tagged with `ask_id`; `ai_cancel(ask_id)` stops
/// it. Rejects with `cap_reached` when the month's cap is reached and `confirmed` is false,
/// with `no_store` without a content store, and with `no_chat` when the chat was deleted.
#[allow(clippy::too_many_arguments)]
#[tauri::command(async)]
pub fn ai_chat_send<R: Runtime>(
    app: AppHandle<R>,
    ai: State<'_, AiState>,
    core: State<'_, AppState>,
    ask_id: u64,
    chat_id: Option<i64>,
    scope: Scope,
    question: String,
    confirmed: bool,
    lang: String,
) -> Result<ChatSent, ApiError> {
    let vfs = core.vfs()?;
    let ai: &AiState = &ai;
    off_runtime(|| {
        let searcher = vfs_content::Searcher::open(&ai.index_db)
            .map_err(|e| ask_error(ask::AskError::Content(e)))?;
        let corpus = LocalCorpus {
            searcher: &searcher,
            vfs: &vfs,
        };
        ai.chat_send_over(
            &corpus,
            ask_id,
            chat_id,
            &scope,
            &question,
            confirmed,
            &lang,
            &mut |text| {
                let _ = app.emit(EVENT_ANSWER_TEXT, TextEvent { ask_id, text });
            },
        )
    })
}

/// The chat list (R2).
#[tauri::command(async)]
pub fn ai_chats(ai: State<'_, AiState>) -> Result<Vec<ChatSummary>, ApiError> {
    ai.chats()
}

/// Opens a saved chat (R11).
#[tauri::command(async)]
pub fn ai_chat(ai: State<'_, AiState>, chat_id: i64) -> Result<ChatView, ApiError> {
    ai.chat(chat_id)
}

/// «Renombrar» (R2).
#[tauri::command(async)]
pub fn ai_chat_rename(ai: State<'_, AiState>, chat_id: i64, title: String) -> Result<(), ApiError> {
    ai.rename_chat(chat_id, &title)
}

/// «Borrar» (R10).
#[tauri::command(async)]
pub fn ai_chat_delete(ai: State<'_, AiState>, chat_id: i64) -> Result<(), ApiError> {
    ai.delete_chat(chat_id)
}

/// «Borrar todos los chats» (R10).
#[tauri::command(async)]
pub fn ai_chats_clear(ai: State<'_, AiState>) -> Result<(), ApiError> {
    ai.clear_chats()
}

/// «Bibliografía» of one answer (VFS-28 R12).
#[tauri::command(async)]
pub fn ai_chat_bibliography(
    ai: State<'_, AiState>,
    chat_id: i64,
    ord: i64,
    all: bool,
) -> Result<Vec<BibEntry>, ApiError> {
    ai.chat_bibliography(chat_id, ord, all)
}

/// «Exportar RIS» of one answer (VFS-28 R12): asks where to save and writes the file.
/// Resolves with the path written, or null when the user cancelled the dialog.
#[tauri::command(async)]
pub fn ai_chat_export_ris<R: Runtime>(
    app: AppHandle<R>,
    ai: State<'_, AiState>,
    chat_id: i64,
    ord: i64,
    all: bool,
) -> Result<Option<String>, ApiError> {
    let text = bibliography::ris(&ai.chat_bibliography(chat_id, ord, all)?);
    let picked = app
        .dialog()
        .file()
        .set_file_name(crate::i18n::current(&app).strings().bibliography_file)
        .add_filter("RIS", &["ris"])
        .blocking_save_file();
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(storage_error)?;
    std::fs::write(&path, text).map_err(storage_error)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-49 R3: the title is the first question on one line, cut at 60 characters.
    #[test]
    fn titles_are_the_first_question_cut() {
        assert_eq!(title_of("  ¿Cuánto\npagué?  "), "¿Cuánto pagué?");
        let long = "palabra ".repeat(20);
        let t = title_of(&long);
        assert_eq!(t.chars().count(), TITLE_CHARS);
        assert!(t.ends_with('…'));
    }

    /// A saved scope this build cannot read opens as every document.
    #[test]
    fn unknown_scopes_fall_back_to_all() {
        assert_eq!(scope_of("{\"kind\":\"tag\"}"), Scope::All);
        let doc = Scope::Document {
            file_id: 3,
            name: "a.pdf".into(),
        };
        assert_eq!(scope_of(&scope_json(&doc)), doc);
    }
}
