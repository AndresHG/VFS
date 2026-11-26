//! VFS-28 28d-2, VFS-49: asking in a chat at the app layer, over a two-document corpus and the
//! mock provider: the turn the screen draws, the cap prompt, saved chats, the bibliography and
//! the cancel. No test needs a real key or the network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vfs_ai::ask::Corpus;
use vfs_ai::mock::{anthropic_answer, anthropic_rewrite, MockServer};
use vfs_ai::{
    AnswerBlock, ApiKey, CredentialBackend, Keys, Provider, RawCitation, Rewrite, StoreError,
};
use vfs_content::{ContentHit, Passage, PassageText, Query};
use vfs_core::{FileKind, SearchFilters};
use vfs_desktop_lib::ai::{AiState, DB_FILE};

#[derive(Default)]
struct MemStore(Arc<Mutex<HashMap<String, String>>>);

impl CredentialBackend for MemStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn get(&self, account: &str) -> Result<String, StoreError> {
        self.0
            .lock()
            .unwrap()
            .get(account)
            .cloned()
            .ok_or(StoreError::NoEntry)
    }
    fn delete(&self, account: &str) -> Result<(), StoreError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// Two documents of one chunk each: a PDF with metadata (page 4) and a DOCX without.
struct TwoDocs;

const PDF_TEXT: &str = "La fianza asciende a 1.200 euros.";
const DOCX_TEXT: &str = "Recibí 1.200 euros por transferencia.";

fn hit(id: i64, name: &str, page: Option<u32>, meta: bool) -> ContentHit {
    ContentHit {
        id,
        name: name.into(),
        dir: "/Archivo".into(),
        kind: FileKind::Document,
        size: 1,
        modified: 1_560_000_000,
        score: 1.0,
        snippet: String::new(),
        highlights: Vec::new(),
        passages: vec![Passage {
            file_id: id,
            ordinal: 0,
            page,
            start_char: 0,
            end_char: 0,
            score: 1.0,
            snippet: String::new(),
            highlights: Vec::new(),
        }],
        title: meta.then(|| "Contrato de arrendamiento".to_owned()),
        authors: if meta {
            vec!["Ana Ruiz".into()]
        } else {
            Vec::new()
        },
        created: meta.then(|| "D:20210304".to_owned()),
        by_meaning: false,
    }
}

impl Corpus for TwoDocs {
    fn retrieve(
        &self,
        _: &Query,
        _: &SearchFilters,
        _: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        Ok(vec![
            hit(1, "Contrato.pdf", Some(4), true),
            hit(2, "Recibo.docx", None, false),
        ])
    }
    fn passage_text(
        &self,
        file_id: i64,
        _: u32,
        _: u32,
        _: u32,
    ) -> vfs_content::Result<Option<PassageText>> {
        let text = if file_id == 1 { PDF_TEXT } else { DOCX_TEXT };
        Ok(Some(PassageText {
            file_id,
            first_ordinal: 0,
            last_ordinal: 0,
            start_char: 0,
            end_char: text.len(),
            text: text.into(),
        }))
    }
    fn folders_named(&self, _: &str) -> vfs_content::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn retrieve_in_file(
        &self,
        q: &Query,
        file_id: i64,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        let hits = self.retrieve(q, &SearchFilters::default(), k)?;
        Ok(hits.into_iter().filter(|h| h.id == file_id).collect())
    }
    fn document(&self, file_id: i64, _: u32) -> vfs_content::Result<Option<ContentHit>> {
        Ok(match file_id {
            1 => Some(hit(1, "Contrato.pdf", Some(4), true)),
            2 => Some(hit(2, "Recibo.docx", None, false)),
            _ => None,
        })
    }
}

fn state(mock: &MockServer) -> (tempfile::TempDir, AiState) {
    let dir = tempfile::tempdir().unwrap();
    let keys = Keys::with_backend(Box::new(MemStore::default()), dir.path().join("ai-keys"));
    keys.save(
        Provider::Anthropic,
        &ApiKey::new("sk-ant-test-NOT-REAL-0123").unwrap(),
    )
    .unwrap();
    let ai = AiState::with(dir.path(), keys, Some(mock.url()));
    (dir, ai)
}

fn block(text: &str, cites: &[(usize, &str)]) -> AnswerBlock {
    AnswerBlock {
        text: text.into(),
        citations: cites
            .iter()
            .map(|(p, q)| RawCitation {
                passage: *p,
                quote: (*q).into(),
            })
            .collect(),
    }
}

fn rewrite() -> Rewrite {
    Rewrite {
        keywords: vec!["fianza".into()],
        ..Rewrite::default()
    }
}

/// VFS-28 AC-8 (UI part, R10): with the cap reached, a chat message rejects with
/// `cap_reached` and sends nothing; confirmed, it asks, and no chat is kept for the refusal.
#[test]
fn cap_reached_then_confirmed() {
    use vfs_ai::ask::Scope;
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("x", &[])]),
    ]);
    let (dir, ai) = state(&mock);
    let mut no_text = |_: String| {};
    vfs_ai::AiDb::open(&dir.path().join(DB_FILE))
        .unwrap()
        .set_cap(0)
        .unwrap();
    let err = ai
        .chat_send_over(
            &TwoDocs,
            1,
            None,
            &Scope::All,
            "q",
            false,
            "es",
            &mut no_text,
        )
        .unwrap_err();
    assert_eq!(err.code, "cap_reached");
    assert!(
        mock.requests().is_empty(),
        "nothing is sent before confirmation"
    );
    assert!(ai.chats().unwrap().is_empty());
    let sent = ai
        .chat_send_over(
            &TwoDocs,
            2,
            None,
            &Scope::All,
            "q",
            true,
            "es",
            &mut no_text,
        )
        .unwrap();
    assert!(sent.turn.not_found);
}

/// VFS-28 R13 «Cancelar»: cancelling a streaming answer stops it with `cancelled`, the question
/// is still recorded and priced (R10, R11), and the id no longer names a running question.
#[test]
fn cancel_stops_a_streaming_answer() {
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("uno ", &[]), block("dos", &[])]),
    ]);
    let (_d, ai) = state(&mock);
    assert!(!ai.cancel(7), "nothing is running yet");
    let mut cancelled = false;
    let err = ai
        .chat_send_over(
            &TwoDocs,
            7,
            None,
            &vfs_ai::ask::Scope::All,
            "q",
            false,
            "es",
            &mut |_| {
                cancelled |= ai.cancel(7);
            },
        )
        .unwrap_err();
    assert!(cancelled);
    assert_eq!(err.code, "cancelled");
    assert!(!ai.cancel(7), "the finished question is forgotten");
    assert!(
        ai.chats().unwrap().is_empty(),
        "no chat is kept for a cancelled message"
    );
    assert!(
        ai.status().unwrap().month_nano > 0,
        "the cancelled answer is priced"
    );
}

/// VFS-49 AC-3, AC-5 (R4, R10, R11): a new chat is saved with its first answer; a second
/// message joins it; after a restart the chat opens from `ai.db` with both turns, markers and
/// sources, without a request, and «Bibliografía» of a turn lists what it cited.
#[test]
fn chats_are_saved_and_reopen_without_a_call() {
    use vfs_ai::ask::Scope;
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("La fianza fue de 1.200 €.", &[(0, PDF_TEXT)])]),
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("Por transferencia.", &[(1, "por transferencia")])]),
    ]);
    let (dir, ai) = state(&mock);
    let mut streamed = String::new();
    let first = ai
        .chat_send_over(
            &TwoDocs,
            1,
            None,
            &Scope::All,
            "¿Cuánto pagué de fianza?",
            false,
            "es",
            &mut |t| streamed.push_str(&t),
        )
        .unwrap();
    assert_eq!(streamed, "La fianza fue de 1.200 €.");
    assert_eq!(first.turn.blocks[0].citations[0].marker, 1);
    assert_eq!(first.month_nano, first.turn.cost_nano);
    let second = ai
        .chat_send_over(
            &TwoDocs,
            2,
            Some(first.chat_id),
            &Scope::All,
            "¿Cómo se pagó?",
            false,
            "es",
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(second.chat_id, first.chat_id);
    let rewrite_body = mock.requests()[2].json().to_string();
    assert!(
        rewrite_body.contains("¿Cuánto pagué de fianza?"),
        "{rewrite_body}"
    );

    // A restart: a new state over the same folder, and a provider that must not be called.
    let silent = MockServer::start(vec![]);
    let keys = Keys::with_backend(Box::new(MemStore::default()), dir.path().join("ai-keys"));
    let again = AiState::with(dir.path(), keys, Some(silent.url()));
    let list = again.chats().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "¿Cuánto pagué de fianza?");
    let chat = again.chat(first.chat_id).unwrap();
    assert_eq!(chat.turns.len(), 2);
    assert_eq!(chat.turns[0], first.turn);
    assert_eq!(chat.turns[1].sources[1].name, "Recibo.docx");
    assert!(silent.requests().is_empty());
    let bib = again.chat_bibliography(first.chat_id, 0, false).unwrap();
    assert_eq!(bib.len(), 1);
    assert_eq!(bib[0].pages, [4]);
    assert_eq!(
        again
            .chat_bibliography(first.chat_id, 1, true)
            .unwrap()
            .len(),
        2
    );

    again.rename_chat(first.chat_id, "  Fianza  ").unwrap();
    assert_eq!(again.chats().unwrap()[0].title, "Fianza");
    again.delete_chat(first.chat_id).unwrap();
    assert_eq!(again.chat(first.chat_id).unwrap_err().code, "no_chat");
    let err = again
        .chat_send_over(
            &TwoDocs,
            3,
            Some(first.chat_id),
            &Scope::All,
            "q",
            false,
            "es",
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(err.code, "no_chat");
    assert!(
        silent.requests().is_empty(),
        "nothing sent for a deleted chat"
    );
}

/// VFS-49 AC-4 (R6, R7) at the app layer: a chat limited to one document keeps its scope and
/// sends that document's passages only.
#[test]
fn a_document_chat_keeps_its_scope() {
    use vfs_ai::ask::Scope;
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("Por transferencia.", &[(0, "por transferencia")])]),
    ]);
    let (_d, ai) = state(&mock);
    let scope = Scope::Document {
        file_id: 2,
        name: "Recibo.docx".into(),
    };
    let sent = ai
        .chat_send_over(
            &TwoDocs,
            1,
            None,
            &scope,
            "¿Cómo?",
            false,
            "es",
            &mut |_| {},
        )
        .unwrap();
    assert!(sent.turn.sources.iter().all(|s| s.file_id == 2));
    assert_eq!(ai.chat(sent.chat_id).unwrap().scope, scope);
    assert_eq!(ai.chats().unwrap()[0].scope, scope);
}

/// VFS-49 AC-6 (R10): «Borrar todos los chats» leaves «Qué se ha enviado».
#[test]
fn clearing_chats_keeps_the_record() {
    use vfs_ai::ask::Scope;
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block("x", &[])]),
    ]);
    let (dir, ai) = state(&mock);
    ai.chat_send_over(
        &TwoDocs,
        1,
        None,
        &Scope::All,
        "q",
        false,
        "es",
        &mut |_| {},
    )
    .unwrap();
    ai.clear_chats().unwrap();
    assert!(ai.chats().unwrap().is_empty());
    let db = vfs_ai::AiDb::open(&dir.path().join(DB_FILE)).unwrap();
    assert_eq!(db.records().unwrap().len(), 1);
}
