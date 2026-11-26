//! The ask flow (VFS-28 R6–R10, R14), PR 28c: rewrite the question, retrieve under the checked
//! filters, pick and merge passages, answer from them, check the citations, then record what
//! was sent and what it cost. A chat message ([`chat_turn`], VFS-49 R4) runs the whole flow
//! again with the chat's earlier turns as context.
//!
//! Every call blocks; run it off the UI thread.

mod citations;
mod prompts;
mod retrieval;

use std::time::Instant;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use vfs_content::{ContentHit, PassageText, Query, Searcher};
use vfs_core::{FileKind, SearchFilters, SearchQuery, Vfs};

pub use citations::{check as check_citations, CheckedBlock, Citation};
pub use prompts::{answer_prompt, rewrite_prompt, Lang};
pub use retrieval::{
    pick_passages, question_filters, retrieve_rounds, Corpus, Retrieved, Source, MAX_PASSAGES,
    MIN_DOCS, ROUND_K,
};

use crate::cost::{cost_nano, models_for, Usage};
use crate::db::{AiDb, CallKind, CallRecord, CapStatus, SentPassage, SentRecord};
use crate::error::AiError;
use crate::provider::{
    Answer, AnswerEvent, AnswerRequest, CancelToken, ChatModel, Passage, Provider, Rewrite,
    RewriteRequest, Turn, ANSWER_MAX_TOKENS,
};

/// Why a question or follow-up did not produce an answer.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// The month's spend has reached the cap and the user has not confirmed (R10).
    #[error("monthly cap reached")]
    CapReached(CapStatus),
    #[error(transparent)]
    Ai(#[from] AiError),
    #[error(transparent)]
    Content(#[from] vfs_content::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

impl AskError {
    /// The label of the R14 log line.
    pub fn kind(&self) -> &'static str {
        match self {
            AskError::CapReached(_) => "cap_reached",
            AskError::Ai(e) => e.kind(),
            AskError::Content(_) => "content",
            AskError::Db(_) => "db",
        }
    }
}

/// How a question is asked.
#[derive(Debug, Clone)]
pub struct AskOptions {
    pub provider: Provider,
    pub lang: Lang,
    /// The cheap tier answers (D7; picked beside the question, VFS-47).
    pub cheap: bool,
    /// A model to answer with instead of the tier's; `None` in the app (VFS-46).
    pub answer_model: Option<&'static str>,
    /// The user confirmed asking with the month's cap reached (R10).
    pub over_cap_confirmed: bool,
    /// The time of asking: the record's date, the month of the total, «today» for the rewrite.
    pub now: DateTime<Local>,
}

/// What a chat asks over (VFS-49 R6): every document, one of the user's folders, or one
/// document. The search screen's filters never apply to a chat (D2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scope {
    #[default]
    All,
    Folder {
        path: String,
    },
    Document {
        #[serde(rename = "fileId")]
        file_id: i64,
        name: String,
    },
}

/// Chunks sent from the start of a document when a chat limited to it asks something that
/// matches none of it, such as «Resúmelo» (VFS-49 R7): about the passage budget of a question.
pub const FIRST_CHUNKS: u32 = MAX_PASSAGES as u32;

/// A document retrieved for a question: the closest documents of a not-found answer (R8) and
/// «todos los encontrados» of the bibliography (R12). Kept with a saved chat (VFS-49 R10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub file_id: i64,
    pub name: String,
    pub dir: String,
    pub modified: i64,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub created: Option<String>,
}

impl From<&ContentHit> for Document {
    fn from(h: &ContentHit) -> Document {
        Document {
            file_id: h.id,
            name: h.name.clone(),
            dir: h.dir.clone(),
            modified: h.modified,
            title: h.title.clone(),
            authors: h.authors.clone(),
            created: h.created.clone(),
        }
    }
}

/// One answer, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub blocks: Vec<CheckedBlock>,
    /// No citation survived the check: show «No he encontrado la respuesta en sus documentos»
    /// and the closest documents (R8).
    pub not_found: bool,
    /// Citations dropped by the check (R7).
    pub dropped: usize,
    /// The model that answered.
    pub model: String,
    /// The cost of this question or follow-up, every call included (R10).
    pub cost_nano: i64,
}

impl Reply {
    /// The answer's whole text.
    pub fn text(&self) -> String {
        self.blocks.iter().map(|b| b.text.as_str()).collect()
    }
}

/// A question's passages and documents, and the chat's turns so far with this one (VFS-49 R4).
#[derive(Debug, Clone)]
pub struct Thread {
    pub provider: Provider,
    /// The passages sent, in the order the model saw them; citations index into it.
    pub sources: Vec<Source>,
    /// The documents of the last retrieval round, best first.
    pub documents: Vec<Document>,
    /// Earlier questions and answers, oldest first.
    pub turns: Vec<Turn>,
}

impl Thread {
    fn passages(&self) -> Vec<Passage> {
        self.sources
            .iter()
            .enumerate()
            .map(|(i, s)| Passage {
                source: format!("vfs://passage/{i}"),
                title: s.title(),
                text: s.text.clone(),
            })
            .collect()
    }

    fn sent(&self) -> Vec<SentPassage> {
        self.sources
            .iter()
            .map(|s| SentPassage {
                file_name: s.name.clone(),
                page: s.pages.first().copied(),
                text: s.text.clone(),
            })
            .collect()
    }
}

/// A question's result: its passages and documents with the chat's turns, and its reply.
#[derive(Debug, Clone)]
pub struct Asked {
    pub thread: Thread,
    pub reply: Reply,
    /// Documents found by each retrieval round (R14).
    pub rounds: Vec<usize>,
}

/// What one question leaves for the R14 log line. Counts only: no question, passage or name.
#[derive(Default)]
struct Diag {
    rounds: Vec<usize>,
    passages: usize,
    usage: Usage,
    cost_nano: i64,
    dropped: usize,
}

fn log_diag(what: &str, started: Instant, d: &Diag, error: Option<&str>) {
    log::info!(
        "vfs-ai: {what} rounds={:?} passages={} latency_ms={} input_tokens={} output_tokens={} \
         cost_nano={} citations_dropped={} error={}",
        d.rounds,
        d.passages,
        started.elapsed().as_millis(),
        d.usage.input_tokens + d.usage.cache_read_tokens + d.usage.cache_write_tokens,
        d.usage.output_tokens,
        d.cost_nano,
        d.dropped,
        error.unwrap_or("none"),
    );
}

fn check_cap(db: &AiDb, opts: &AskOptions) -> Result<(), AskError> {
    let status = db.cap_status(&opts.now)?;
    if status.reached() && !opts.over_cap_confirmed {
        return Err(AskError::CapReached(status));
    }
    Ok(())
}

fn priced(kind: CallKind, model: &str, usage: Usage) -> CallRecord {
    CallRecord {
        kind,
        model: model.to_owned(),
        usage,
        cost_nano: cost_nano(model, &usage),
    }
}

/// The answer call, priced: the answer, or the error with the cost of a cancelled stream.
fn answer_call(
    model: &dyn ChatModel,
    request: &AnswerRequest,
    kind: CallKind,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(AnswerEvent),
) -> (Result<Answer, AiError>, Option<CallRecord>) {
    let id = request.answer_model(model.provider());
    match model.answer(request, cancel, on_event) {
        Ok(a) => {
            let call = priced(kind, &a.model, a.usage);
            (Ok(a), Some(call))
        }
        Err(AiError::Cancelled { usage }) => {
            let call = priced(kind, id, Usage::for_cancelled(usage, ANSWER_MAX_TOKENS));
            (Err(AiError::Cancelled { usage }), Some(call))
        }
        Err(e) => (Err(e), None),
    }
}

fn add(diag: &mut Diag, call: &CallRecord) {
    let u = &mut diag.usage;
    u.input_tokens += call.usage.input_tokens;
    u.output_tokens += call.usage.output_tokens;
    u.cache_read_tokens += call.usage.cache_read_tokens;
    u.cache_write_tokens += call.usage.cache_write_tokens;
    diag.cost_nano += call.cost_nano;
}

fn reply_of(answer: &Answer, sources: &[Source], calls: &[CallRecord]) -> Reply {
    let texts: Vec<&str> = sources.iter().map(|s| s.text.as_str()).collect();
    let (blocks, dropped) = check_citations(&answer.blocks, &texts);
    let not_found = blocks.iter().all(|b| b.citations.is_empty());
    Reply {
        blocks,
        not_found,
        dropped,
        model: answer.model.clone(),
        cost_nano: calls.iter().map(|c| c.cost_nano).sum(),
    }
}

/// Asks a first question (R6, R7, R10, R11, R14). `screen` holds the filters on screen.
/// Everything sent is recorded in `db`, also when the answer fails after the rewrite.
#[allow(clippy::too_many_arguments)]
pub fn ask(
    model: &dyn ChatModel,
    corpus: &dyn Corpus,
    db: &mut AiDb,
    question: &str,
    screen: &SearchFilters,
    opts: &AskOptions,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(AnswerEvent),
) -> Result<Asked, AskError> {
    let started = Instant::now();
    let mut diag = Diag::default();
    let result = ask_inner(
        model,
        corpus,
        db,
        question,
        &[],
        screen,
        None,
        opts,
        cancel,
        on_event,
        &mut diag,
    );
    log_diag(
        "ask",
        started,
        &diag,
        result.as_ref().err().map(AskError::kind),
    );
    result
}

/// Asks one message of a chat (VFS-49 R4, R7): the whole flow of [`ask`] again, over `scope`,
/// with the chat's `earlier` turns. The rewrite sees the earlier questions, so a message such
/// as «¿Y me la devolvieron?» searches for what the chat is about; the answer sees the earlier
/// questions and answers as text and only this message's passages. Priced, recorded and
/// capped like a question. The returned thread holds `earlier` plus this turn.
#[allow(clippy::too_many_arguments)]
pub fn chat_turn(
    model: &dyn ChatModel,
    corpus: &dyn Corpus,
    db: &mut AiDb,
    question: &str,
    earlier: &[Turn],
    scope: &Scope,
    opts: &AskOptions,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(AnswerEvent),
) -> Result<Asked, AskError> {
    let started = Instant::now();
    let mut diag = Diag::default();
    let (filters, file) = match scope {
        Scope::All => (SearchFilters::default(), None),
        Scope::Folder { path } => (
            SearchFilters {
                folder: Some(path.clone()),
                ..SearchFilters::default()
            },
            None,
        ),
        Scope::Document { file_id, .. } => (SearchFilters::default(), Some(*file_id)),
    };
    let result = ask_inner(
        model, corpus, db, question, earlier, &filters, file, opts, cancel, on_event, &mut diag,
    );
    log_diag(
        "chat",
        started,
        &diag,
        result.as_ref().err().map(AskError::kind),
    );
    result
}

/// The rewrite call's input (VFS-49 R4): the question alone, or after a chat's earlier
/// questions, which tell the model what a short follow-up refers to.
fn rewrite_input(question: &str, earlier: &[Turn]) -> String {
    if earlier.is_empty() {
        return question.to_owned();
    }
    let before: Vec<String> = earlier
        .iter()
        .map(|t| format!("- {}", t.question))
        .collect();
    format!(
        "Earlier questions in this conversation, for context only:\n{}\n\nQuestion: {question}",
        before.join("\n")
    )
}

#[allow(clippy::too_many_arguments)]
fn ask_inner(
    model: &dyn ChatModel,
    corpus: &dyn Corpus,
    db: &mut AiDb,
    question: &str,
    earlier: &[Turn],
    screen: &SearchFilters,
    file: Option<i64>,
    opts: &AskOptions,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(AnswerEvent),
    diag: &mut Diag,
) -> Result<Asked, AskError> {
    check_cap(db, opts)?;
    let models = models_for(opts.provider);
    let rq = RewriteRequest {
        system_prompt: rewrite_prompt(opts.now.date_naive()),
        question: rewrite_input(question, earlier),
    };
    let (rewrite, raw, usage) = model.rewrite(&rq, cancel)?;
    let mut calls = vec![priced(CallKind::Rewrite, models.rewrite, usage)];
    add(diag, &calls[0]);
    let mut record = SentRecord {
        asked_at: opts.now,
        provider: opts.provider,
        question: question.to_owned(),
        rewrite: Some(raw),
        passages: Vec::new(),
        calls: Vec::new(),
    };

    let thread = match gather(corpus, &rewrite, screen, file, opts.provider, cancel, diag) {
        Ok(t) => t,
        Err(e) => {
            record.calls = calls;
            db.record(&record)?;
            return Err(e);
        }
    };
    record.passages = thread.sent();

    let request = AnswerRequest {
        system_prompt: answer_prompt(opts.lang),
        passages: thread.passages(),
        earlier: earlier.to_vec(),
        question: question.to_owned(),
        cheap: opts.cheap,
        model: opts.answer_model,
    };
    let (answer, call) = answer_call(model, &request, CallKind::Answer, cancel, on_event);
    if let Some(call) = call {
        add(diag, &call);
        calls.push(call);
    }
    record.calls = calls.clone();
    db.record(&record)?;
    let answer = answer?;
    let reply = reply_of(&answer, &thread.sources, &calls);
    diag.dropped = reply.dropped;
    let mut thread = thread;
    thread.turns = earlier.to_vec();
    thread.turns.push(Turn {
        question: question.to_owned(),
        answer: reply.text(),
    });
    Ok(Asked {
        thread,
        reply,
        rounds: diag.rounds.clone(),
    })
}

/// Retrieval and passage choice for a rewritten question (R6.2–R6.4). With `file`, only that
/// document is searched, and its first chunks stand in when nothing in it matches (VFS-49 R7).
fn gather(
    corpus: &dyn Corpus,
    rewrite: &Rewrite,
    screen: &SearchFilters,
    file: Option<i64>,
    provider: Provider,
    cancel: &CancelToken,
    diag: &mut Diag,
) -> Result<Thread, AskError> {
    let cancelled = || AskError::Ai(AiError::Cancelled { usage: None });
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let hits = match file {
        None => {
            let (filters, narrowed) = question_filters(rewrite, screen, corpus)?;
            let found = retrieve_rounds(corpus, rewrite, screen, &filters, narrowed)?;
            diag.rounds = found.rounds;
            found.hits
        }
        Some(file_id) => {
            let in_file = InFile { corpus, file_id };
            let none = SearchFilters::default();
            let found = retrieve_rounds(&in_file, rewrite, &none, &none, false)?;
            diag.rounds = found.rounds;
            if found.hits.is_empty() {
                corpus
                    .document(file_id, FIRST_CHUNKS)?
                    .into_iter()
                    .collect()
            } else {
                found.hits
            }
        }
    };
    let sources = pick_passages(corpus, &hits)?;
    diag.passages = sources.len();
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    Ok(Thread {
        provider,
        sources,
        documents: hits.iter().map(Document::from).collect(),
        turns: Vec::new(),
    })
}

/// A [`Corpus`] whose retrieval searches one document only (VFS-49 R7). It names no folder,
/// so a rewrite's folder hint never narrows it.
struct InFile<'a> {
    corpus: &'a dyn Corpus,
    file_id: i64,
}

impl Corpus for InFile<'_> {
    fn retrieve(
        &self,
        query: &Query,
        _filters: &SearchFilters,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        self.corpus.retrieve_in_file(query, self.file_id, k)
    }

    fn passage_text(
        &self,
        file_id: i64,
        ordinal: u32,
        before: u32,
        after: u32,
    ) -> vfs_content::Result<Option<PassageText>> {
        self.corpus.passage_text(file_id, ordinal, before, after)
    }

    fn folders_named(&self, _hint: &str) -> vfs_content::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn retrieve_in_file(
        &self,
        query: &Query,
        file_id: i64,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        self.corpus.retrieve_in_file(query, file_id, k)
    }

    fn document(&self, file_id: i64, n: u32) -> vfs_content::Result<Option<ContentHit>> {
        self.corpus.document(file_id, n)
    }
}

/// The app's [`Corpus`]: content search's [`Searcher`] and the name index's [`Vfs`].
pub struct LocalCorpus<'a> {
    pub searcher: &'a Searcher,
    pub vfs: &'a Vfs,
}

impl Corpus for LocalCorpus<'_> {
    fn retrieve(
        &self,
        query: &Query,
        filters: &SearchFilters,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        Ok(self.searcher.retrieve(query, filters, k, 0)?.hits)
    }

    fn passage_text(
        &self,
        file_id: i64,
        ordinal: u32,
        before: u32,
        after: u32,
    ) -> vfs_content::Result<Option<PassageText>> {
        self.searcher.passage_text(file_id, ordinal, before, after)
    }

    fn retrieve_in_file(
        &self,
        query: &Query,
        file_id: i64,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        Ok(self.searcher.retrieve_file(query, file_id, k)?.hits)
    }

    fn document(&self, file_id: i64, n: u32) -> vfs_content::Result<Option<ContentHit>> {
        self.searcher.document(file_id, n)
    }

    fn folders_named(&self, hint: &str) -> vfs_content::Result<Vec<String>> {
        let hint = hint.trim_end_matches(['/', '\\']);
        let last = hint.rsplit(['/', '\\']).next().unwrap_or(hint);
        if last.is_empty() {
            return Ok(Vec::new());
        }
        let r = self.vfs.search(&SearchQuery {
            text: last.to_owned(),
            filters: SearchFilters {
                kinds: vec![FileKind::Folder],
                ..SearchFilters::default()
            },
            limit: 50,
            offset: 0,
            sort: Default::default(),
            scope: Default::default(),
        })?;
        let mut found: Vec<String> = Vec::new();
        for h in r.hits.iter().filter(|h| h.is_dir) {
            let path = std::path::Path::new(&h.dir).join(&h.name);
            let path = path.to_string_lossy().into_owned();
            if ends_with_path(&path, hint) && !found.contains(&path) {
                found.push(path);
            }
        }
        Ok(found)
    }
}

/// `path` ends with the folder path `tail`, on a separator, after the accent fold.
fn ends_with_path(path: &str, tail: &str) -> bool {
    let key = |s: &str| {
        vfs_core::fold_key(&s.replace('\\', "/"))
            .trim_matches('/')
            .to_owned()
    };
    let (p, t) = (key(path), key(tail));
    !t.is_empty() && (p == t || p.ends_with(&format!("/{t}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_tails_match_on_separators() {
        assert!(ends_with_path("/Docs/Casa/Facturas", "facturas"));
        assert!(ends_with_path("C:\\Docs\\Casa\\Facturas", "Casa/Facturas"));
        assert!(ends_with_path("/Docs/Casa", "/Docs/Casa/"));
        assert!(!ends_with_path("/Docs/MisFacturas", "Facturas"));
    }
}
