//! VFS-49 49a: a chat message runs the whole ask flow again with the chat's earlier turns
//! (R4), over the chat's sources (R6, R7), and saved chats live in `ai.db` apart from the
//! record (R10). AC-3, AC-4, AC-5 (store part), AC-6.

mod common;

use chrono::{Local, TimeZone};
use common::fake_corpus::{FakeCorpus, FakeDoc};
use common::*;
use vfs_ai::ask::{self, AskOptions, Lang, Scope, FIRST_CHUNKS};
use vfs_ai::mock::{anthropic_answer, anthropic_rewrite, MockResponse, MockServer};
use vfs_ai::{
    AiDb, AnswerBlock, CancelToken, ChatTurnRow, Provider, RawCitation, Rewrite, SentRecord, Turn,
};

fn opts() -> AskOptions {
    AskOptions {
        provider: Provider::Anthropic,
        lang: Lang::Es,
        cheap: true,
        answer_model: None,
        over_cap_confirmed: false,
        now: Local.with_ymd_and_hms(2026, 11, 26, 12, 0, 0).unwrap(),
    }
}

fn rewrite(keywords: &[&str]) -> Rewrite {
    Rewrite {
        keywords: keywords.iter().map(|s| s.to_string()).collect(),
        ..Rewrite::default()
    }
}

fn cited(text: &str, passage: usize, quote: &str) -> AnswerBlock {
    AnswerBlock {
        text: text.into(),
        citations: vec![RawCitation {
            passage,
            quote: quote.into(),
        }],
    }
}

fn db() -> (tempfile::TempDir, AiDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    (dir, db)
}

fn docs() -> Vec<FakeDoc> {
    vec![
        FakeDoc::new(1, "/Archivo/Casa", "Contrato.pdf", 6, &[0]),
        FakeDoc::new(2, "/Archivo/Casa", "Correo.eml", 3, &[1]),
        FakeDoc::new(3, "/Archivo/Trabajo", "Nómina.pdf", 3, &[1]),
    ]
}

fn turn(
    corpus: &FakeCorpus,
    db: &mut AiDb,
    question: &str,
    earlier: &[Turn],
    scope: &Scope,
    responses: Vec<MockResponse>,
) -> (ask::Asked, MockServer) {
    let mock = MockServer::start(responses);
    let model = model_at(Provider::Anthropic, &mock);
    let asked = ask::chat_turn(
        model.as_ref(),
        corpus,
        db,
        question,
        earlier,
        scope,
        &opts(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    (asked, mock)
}

/// The request bodies the mock received, in order: the rewrite, then the streamed answer.
fn bodies(mock: &MockServer) -> (serde_json::Value, serde_json::Value) {
    let reqs = mock.requests();
    let answer = reqs.iter().find(|r| r.json()["stream"] == true).unwrap();
    let rewrite = reqs.iter().find(|r| r.json()["stream"] != true).unwrap();
    (rewrite.json(), answer.json())
}

/// VFS-49 AC-3 (R4): a second message rewrites with the earlier question, retrieves anew, and
/// answers with the earlier turn and only its own passages; each message is a priced record.
#[test]
fn a_follow_up_message_retrieves_again_with_the_chat_as_context() {
    let corpus = FakeCorpus::new(docs()).scripted(&[&[1], &[1], &[2], &[2]]);
    let (_d, mut db) = db();
    let first = "¿Cuánto pagué de fianza por el piso de la calle Mayor?";
    let (a1, _m1) = turn(
        &corpus,
        &mut db,
        first,
        &[],
        &Scope::All,
        vec![
            anthropic_rewrite(&rewrite(&["fianza", "Mayor"])),
            anthropic_answer(&[cited("Pagó 1.300 €.", 0, "Contrato.pdf trozo 0.")]),
        ],
    );
    assert_eq!(a1.thread.turns.len(), 1);
    let before = corpus.calls.borrow().len();

    let (a2, m2) = turn(
        &corpus,
        &mut db,
        "¿Y me la devolvieron?",
        &a1.thread.turns,
        &Scope::All,
        vec![
            anthropic_rewrite(&rewrite(&["devolución", "fianza"])),
            anthropic_answer(&[cited("Sí.", 0, "Correo.eml trozo 1.")]),
        ],
    );
    let (rw, answer) = bodies(&m2);
    assert!(
        rw.to_string().contains(first),
        "the rewrite saw the earlier question"
    );
    assert!(rw.to_string().contains("¿Y me la devolvieron?"));
    assert!(
        corpus.calls.borrow().len() > before,
        "the message retrieved anew"
    );
    let text = answer.to_string();
    assert!(
        text.contains(first) && text.contains("Pagó 1.300 €."),
        "earlier turn sent"
    );
    assert!(
        text.contains("Correo.eml") && !text.contains("Contrato.pdf"),
        "{text}"
    );
    assert!(!a2.reply.not_found);
    assert_eq!(a2.thread.turns.len(), 2);
    assert_eq!(db.records().unwrap().len(), 2);
    assert!(db.records().unwrap().iter().all(|r| r.cost_nano > 0));
}

/// VFS-49 AC-4 (R6, R7): a chat limited to one document searches only it; when nothing in it
/// matches, its first chunks are sent instead of nothing.
#[test]
fn a_chat_on_one_document_searches_it_alone() {
    let corpus = FakeCorpus::new(docs());
    let (_d, mut db) = db();
    let scope = Scope::Document {
        file_id: 2,
        name: "Correo.eml".into(),
    };
    let (a, m) = turn(
        &corpus,
        &mut db,
        "¿Qué dice del salón?",
        &[],
        &scope,
        vec![
            anthropic_rewrite(&rewrite(&["salón"])),
            anthropic_answer(&[cited("Pintura.", 0, "Correo.eml trozo 1.")]),
        ],
    );
    assert!(corpus
        .calls
        .borrow()
        .iter()
        .all(|c| c.filters.folder.is_none()));
    assert!(!corpus.file_calls.borrow().is_empty());
    assert!(corpus.file_calls.borrow().iter().all(|&id| id == 2));
    assert!(a.thread.sources.iter().all(|s| s.file_id == 2));
    assert!(!bodies(&m).1.to_string().contains("Contrato.pdf"));

    // Nothing in the document matches: its first chunks go instead.
    let corpus = FakeCorpus::new(docs()).scripted(&[&[1], &[1]]);
    let (a, _m) = turn(
        &corpus,
        &mut db,
        "Resúmelo",
        &[],
        &scope,
        vec![
            anthropic_rewrite(&rewrite(&["resumen"])),
            anthropic_answer(&[cited("Un correo.", 0, "Correo.eml trozo 0.")]),
        ],
    );
    assert_eq!(a.thread.documents.len(), 1);
    assert_eq!(
        a.thread.sources.len(),
        1,
        "the first chunks merge into one passage"
    );
    let s = &a.thread.sources[0];
    assert_eq!((s.file_id, s.first_ordinal), (2, 0));
    assert!(s.last_ordinal < FIRST_CHUNKS + 1);
    assert!(!a.reply.not_found);
}

/// VFS-49 AC-4 (R6, D2): «Todos sus documentos» retrieves with no filter at all; a folder
/// scope retrieves inside that folder.
#[test]
fn the_scope_sets_the_filters() {
    let corpus = FakeCorpus::new(docs());
    let (_d, mut db) = db();
    let responses = || {
        vec![
            anthropic_rewrite(&rewrite(&["fianza"])),
            anthropic_answer(&[cited("x", 0, "Contrato.pdf trozo 0.")]),
        ]
    };
    turn(&corpus, &mut db, "fianza", &[], &Scope::All, responses());
    let c = corpus.calls.borrow()[0].clone();
    assert!(c.filters.folder.is_none() && c.filters.kinds.is_empty());
    assert!(c.filters.modified_from.is_none() && c.filters.size_min.is_none());

    let corpus = FakeCorpus::new(docs());
    let scope = Scope::Folder {
        path: "/Archivo/Trabajo".into(),
    };
    let (a, _m) = turn(&corpus, &mut db, "nómina", &[], &scope, responses());
    assert_eq!(
        corpus.calls.borrow()[0].filters.folder.as_deref(),
        Some("/Archivo/Trabajo")
    );
    assert!(a.thread.sources.iter().all(|s| s.dir == "/Archivo/Trabajo"));
}

/// The scope's JSON, as the shell and the screen exchange it.
#[test]
fn scopes_serialize_with_a_kind() {
    let doc = Scope::Document {
        file_id: 7,
        name: "Acta.pdf".into(),
    };
    let json = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "document", "fileId": 7, "name": "Acta.pdf"})
    );
    assert_eq!(serde_json::from_value::<Scope>(json).unwrap(), doc);
    assert_eq!(
        serde_json::to_value(Scope::All).unwrap(),
        serde_json::json!({"kind": "all"})
    );
}

fn row(q: &str) -> ChatTurnRow {
    ChatTurnRow {
        ord: 0,
        question: q.into(),
        answer: format!("respuesta a {q}"),
        view: "{}".into(),
    }
}

/// VFS-49 AC-5 (store part, R10, R11) and AC-2 (store part): chats and their turns are kept,
/// listed latest first, renamed and deleted; a new chat exists only with its first turn.
#[test]
fn chats_are_kept_in_ai_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ai.db");
    let at = |d| Local.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap();
    let (old, new) = {
        let mut db = AiDb::open(&path).unwrap();
        let old = db
            .create_chat("Vacunas", "{\"kind\":\"all\"}", &row("a"), &at(10))
            .unwrap();
        let new = db
            .create_chat("Fianza", "{\"kind\":\"all\"}", &row("b"), &at(20))
            .unwrap();
        assert_eq!(
            db.add_turn(new, "{\"kind\":\"all\"}", &row("c"), &at(21))
                .unwrap(),
            1
        );
        (old, new)
    };
    // Reopened, as after a restart.
    let mut db = AiDb::open(&path).unwrap();
    let ids: Vec<i64> = db.chats().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(ids, [new, old], "latest activity first");
    let turns = db.chat_turns(new).unwrap();
    assert_eq!(
        turns
            .iter()
            .map(|t| t.question.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert_eq!(turns[1].answer, "respuesta a c");

    // Touching the old chat moves it first; the scope is saved with the turn.
    let folder = "{\"kind\":\"folder\",\"path\":\"/Archivo\"}";
    db.add_turn(old, folder, &row("d"), &at(25)).unwrap();
    assert_eq!(db.chats().unwrap()[0].id, old);
    assert_eq!(db.chat(old).unwrap().unwrap().scope, folder);

    db.rename_chat(old, "Perro").unwrap();
    assert_eq!(db.chat(old).unwrap().unwrap().title, "Perro");
    db.delete_chat(old).unwrap();
    assert!(db.chat(old).unwrap().is_none());
    assert!(
        db.chat_turns(old).unwrap().is_empty(),
        "turns go with the chat"
    );
    assert!(db.add_turn(old, folder, &row("e"), &at(26)).is_err());
}

/// VFS-49 AC-6 (R10): «Borrar todos los chats» leaves the record and the month's spend;
/// «Borrar todo» in the record leaves the chats.
#[test]
fn chats_and_the_record_are_cleared_apart() {
    let (_d, mut db) = db();
    let now = opts().now;
    db.create_chat("Fianza", "{\"kind\":\"all\"}", &row("a"), &now)
        .unwrap();
    db.record(&SentRecord {
        asked_at: now,
        provider: Provider::Anthropic,
        question: "a".into(),
        rewrite: None,
        passages: Vec::new(),
        calls: vec![vfs_ai::CallRecord {
            kind: vfs_ai::CallKind::Answer,
            model: "claude-haiku-5-5".into(),
            usage: vfs_ai::Usage::default(),
            cost_nano: 1_000,
        }],
    })
    .unwrap();

    db.clear_records().unwrap();
    assert_eq!(db.chats().unwrap().len(), 1);
    assert_eq!(db.month_total(&now).unwrap(), 1_000);

    db.record(&SentRecord {
        asked_at: now,
        provider: Provider::Anthropic,
        question: "b".into(),
        rewrite: None,
        passages: Vec::new(),
        calls: Vec::new(),
    })
    .unwrap();
    db.clear_chats().unwrap();
    assert!(db.chats().unwrap().is_empty());
    assert_eq!(db.records().unwrap().len(), 1);
    assert_eq!(db.month_total(&now).unwrap(), 1_000);
}
