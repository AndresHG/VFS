//! VFS-28 28d-2: asking at the app layer, over a two-document corpus and the mock provider: the
//! reply the screen draws, the cap prompt, follow-ups in the thread, the bibliography and the
//! cancel. No test needs a real key or the network.

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

/// VFS-28 AC-6 (UI part, R8), AC-8 (UI part, R10), AC-7 (R9), AC-10 (R12): the reply carries
/// numbered markers, the passages and the documents, its cost and the month's; a follow-up in
/// the thread is priced and counted; «Bibliografía» lists the cited documents, or all found,
/// with the pages cited.
#[test]
fn ask_follow_up_and_bibliography() {
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite()),
        anthropic_answer(&[block(
            "La fianza fue de 1.200 €.",
            &[(0, PDF_TEXT), (5, "x")],
        )]),
        anthropic_answer(&[block("Por transferencia.", &[(1, "por transferencia")])]),
    ]);
    let (_d, ai) = state(&mock);
    let mut streamed = String::new();
    let view = ai
        .ask_over(
            &TwoDocs,
            1,
            "¿Cuánto pagué de fianza?",
            &SearchFilters::default(),
            false,
            "es",
            &mut |t| streamed.push_str(&t),
        )
        .unwrap();
    assert_eq!(streamed, "La fianza fue de 1.200 €.");
    let v = serde_json::to_value(&view).unwrap();
    assert_eq!(
        v["reply"]["blocks"][0]["citations"],
        serde_json::json!([{"passage": 0, "marker": 1}]),
        "the citation to an unsent passage is dropped"
    );
    assert_eq!(v["reply"]["notFound"], false);
    assert_eq!(v["sources"][0]["pages"], serde_json::json!([4]));
    assert_eq!(v["documents"][1]["name"], "Recibo.docx");
    let first_cost = v["reply"]["costNano"].as_i64().unwrap();
    assert!(first_cost > 0);
    assert_eq!(v["reply"]["monthNano"].as_i64().unwrap(), first_cost);
    assert!(
        v.to_string().find("/Archivo").is_none(),
        "no folder reaches the view"
    );

    let cited = ai.bibliography(view.thread_id, false).unwrap();
    assert_eq!(cited.len(), 1);
    assert_eq!(cited[0].authors, ["Ana Ruiz"]);
    assert_eq!((cited[0].year, cited[0].year_from_file), (2021, false));
    assert_eq!(cited[0].pages, [4]);

    let follow = ai
        .follow_up_in(
            view.thread_id,
            2,
            "¿Cómo se pagó?",
            false,
            "es",
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(follow.blocks[0].citations[0].marker, 1);
    assert_eq!(follow.month_nano, first_cost + follow.cost_nano);
    let cited = ai.bibliography(view.thread_id, false).unwrap();
    assert_eq!(
        cited.len(),
        2,
        "the follow-up's citation joins the bibliography"
    );
    assert!(cited[1].authors.is_empty() && cited[1].year_from_file);
    assert_eq!(ai.bibliography(view.thread_id, true).unwrap().len(), 2);
}

/// VFS-28 AC-8 (UI part, R10): with the cap reached, asking rejects with `cap_reached` and sends
/// nothing; confirmed, it asks.
#[test]
fn cap_reached_then_confirmed() {
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
        .ask_over(
            &TwoDocs,
            1,
            "q",
            &SearchFilters::default(),
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
    let view = ai
        .ask_over(
            &TwoDocs,
            2,
            "q",
            &SearchFilters::default(),
            true,
            "es",
            &mut no_text,
        )
        .unwrap();
    assert!(view.reply.not_found);
}

/// VFS-28 R9, D3: a follow-up in an unknown thread says so; nothing is kept between runs.
#[test]
fn unknown_thread_is_reported() {
    let mock = MockServer::start(vec![]);
    let (_d, ai) = state(&mock);
    let err = ai
        .follow_up_in(42, 1, "q", false, "es", &mut |_| {})
        .unwrap_err();
    assert_eq!(err.code, "no_thread");
    assert_eq!(ai.bibliography(42, true).unwrap_err().code, "no_thread");
}
