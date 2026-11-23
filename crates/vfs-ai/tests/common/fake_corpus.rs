//! A scripted [`Corpus`] for the ask-flow tests: documents of numbered chunks, retrieval rounds
//! answered from a script, and every call recorded.

use std::cell::RefCell;
use std::collections::VecDeque;

use vfs_ai::ask::Corpus;
use vfs_content::{ContentHit, Passage, PassageText, Query};
use vfs_core::{FileKind, SearchFilters};

pub struct FakeDoc {
    pub id: i64,
    pub name: String,
    pub dir: String,
    /// Chunk texts, by ordinal; chunk `i` is on page `i + 1`.
    pub chunks: Vec<String>,
    /// Ordinals of the matching chunks, best first (at most 3, as `retrieve` returns).
    pub best: Vec<u32>,
}

impl FakeDoc {
    pub fn new(id: i64, dir: &str, name: &str, chunks: usize, best: &[u32]) -> FakeDoc {
        FakeDoc {
            id,
            name: name.into(),
            dir: dir.into(),
            chunks: (0..chunks).map(|i| format!("{name} trozo {i}.")).collect(),
            best: best.to_vec(),
        }
    }

    fn hit(&self) -> ContentHit {
        let passages: Vec<Passage> = self
            .best
            .iter()
            .map(|&o| Passage {
                file_id: self.id,
                ordinal: o,
                page: Some(o + 1),
                start_char: 0,
                end_char: 0,
                score: 1.0,
                snippet: String::new(),
                highlights: Vec::new(),
            })
            .collect();
        ContentHit {
            id: self.id,
            name: self.name.clone(),
            dir: self.dir.clone(),
            kind: FileKind::Document,
            size: 1,
            modified: 0,
            score: 1.0,
            snippet: String::new(),
            highlights: Vec::new(),
            passages,
            title: None,
            authors: Vec::new(),
            created: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetrieveCall {
    pub query: Query,
    pub filters: SearchFilters,
    pub k: u32,
}

pub struct FakeCorpus {
    pub docs: Vec<FakeDoc>,
    /// Document ids each successive `retrieve` returns, before the folder filter. Once the
    /// script runs out, every document is returned.
    pub script: RefCell<VecDeque<Vec<i64>>>,
    /// What `folders_named` answers, per hint.
    pub folders: Vec<(String, Vec<String>)>,
    pub calls: RefCell<Vec<RetrieveCall>>,
}

impl FakeCorpus {
    pub fn new(docs: Vec<FakeDoc>) -> FakeCorpus {
        FakeCorpus {
            docs,
            script: RefCell::new(VecDeque::new()),
            folders: Vec::new(),
            calls: RefCell::new(Vec::new()),
        }
    }

    pub fn scripted(mut self, rounds: &[&[i64]]) -> FakeCorpus {
        self.script = RefCell::new(rounds.iter().map(|r| r.to_vec()).collect());
        self
    }

    pub fn with_folder(mut self, hint: &str, found: &[&str]) -> FakeCorpus {
        self.folders
            .push((hint.into(), found.iter().map(|s| s.to_string()).collect()));
        self
    }

    /// Every document as a hit, in order, without recording a call.
    pub fn retrieve_all(&self) -> Vec<ContentHit> {
        self.docs.iter().map(FakeDoc::hit).collect()
    }

    pub fn doc(&self, id: i64) -> &FakeDoc {
        self.docs.iter().find(|d| d.id == id).unwrap()
    }
}

/// `dir` is `folder` or below it (plain `/` paths in these tests).
pub fn inside(dir: &str, folder: &str) -> bool {
    let f = folder.trim_end_matches('/');
    dir == f || dir.starts_with(&format!("{f}/"))
}

impl Corpus for FakeCorpus {
    fn retrieve(
        &self,
        query: &Query,
        filters: &SearchFilters,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>> {
        self.calls.borrow_mut().push(RetrieveCall {
            query: query.clone(),
            filters: filters.clone(),
            k,
        });
        let ids: Vec<i64> = self
            .script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| self.docs.iter().map(|d| d.id).collect());
        Ok(ids
            .iter()
            .map(|id| self.doc(*id))
            .filter(|d| filters.folder.as_deref().is_none_or(|f| inside(&d.dir, f)))
            .take(k as usize)
            .map(FakeDoc::hit)
            .collect())
    }

    fn passage_text(
        &self,
        file_id: i64,
        ordinal: u32,
        before: u32,
        after: u32,
    ) -> vfs_content::Result<Option<PassageText>> {
        let d = self.doc(file_id);
        if ordinal as usize >= d.chunks.len() {
            return Ok(None);
        }
        let first = ordinal.saturating_sub(before);
        let last = (ordinal + after).min(d.chunks.len() as u32 - 1);
        let text = d.chunks[first as usize..=last as usize].join(" ");
        Ok(Some(PassageText {
            file_id,
            first_ordinal: first,
            last_ordinal: last,
            start_char: 0,
            end_char: text.chars().count(),
            text,
        }))
    }

    fn folders_named(&self, hint: &str) -> vfs_content::Result<Vec<String>> {
        Ok(self
            .folders
            .iter()
            .find(|(h, _)| h == hint)
            .map(|(_, f)| f.clone())
            .unwrap_or_default())
    }
}
