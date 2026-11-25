//! `vfs ask` (VFS-28 28c): ask the archive a question with a real provider, for the manual
//! `--live` check. Built with `--features content`. The key is read from the `VFS_AI_KEY`
//! environment variable and never printed.

use std::io::Write;
use std::path::Path;

use vfs_ai::ask::{self, AskOptions, Lang, LocalCorpus};
use vfs_ai::cost::NANO_PER_USD;
use vfs_ai::{chat_model, AiDb, AnswerEvent, ApiKey, CancelToken, Endpoint, Provider};
use vfs_core::{SearchFilters, Vfs};

use crate::gen::AnyResult;

/// The environment variable holding the provider key for `vfs ask`.
pub const KEY_VAR: &str = "VFS_AI_KEY";

pub struct AskArgs<'a> {
    pub db: &'a Path,
    pub ai_db: &'a Path,
    pub provider: Provider,
    pub lang: Lang,
    pub cheap: bool,
    pub yes: bool,
    pub filters: SearchFilters,
    pub question: String,
}

pub fn run(a: AskArgs) -> AnyResult<()> {
    let key = std::env::var(KEY_VAR).map_err(|_| format!("set {KEY_VAR} to the provider key"))?;
    let key = ApiKey::new(key).map_err(|_| format!("{KEY_VAR} does not hold a valid key"))?;
    let model = chat_model(a.provider, key, Endpoint::public(a.provider))?;
    let searcher = match vfs_content::Searcher::open(a.db) {
        Err(vfs_content::Error::Off) => {
            return Err("content search is off for this index (run `vfs content-index`)".into())
        }
        r => r?,
    };
    let vfs = Vfs::open(a.db)?;
    let corpus = LocalCorpus {
        searcher: &searcher,
        vfs: &vfs,
    };
    let mut db = AiDb::open(a.ai_db)?;
    let opts = AskOptions {
        provider: a.provider,
        lang: a.lang,
        cheap: a.cheap,
        answer_model: None,
        over_cap_confirmed: a.yes,
        now: chrono::Local::now(),
    };
    let mut stdout = std::io::stdout();
    let asked = match ask::ask(
        model.as_ref(),
        &corpus,
        &mut db,
        &a.question,
        &a.filters,
        &opts,
        &CancelToken::new(),
        &mut |AnswerEvent::Text(t)| {
            let _ = write!(stdout, "{t}");
            let _ = stdout.flush();
        },
    ) {
        Err(ask::AskError::CapReached(s)) => {
            return Err(format!(
                "this month has spent {} $ of the {} $ cap; pass --yes to ask anyway",
                usd(s.spent_nano),
                usd(s.cap_nano)
            )
            .into())
        }
        r => r?,
    };
    println!("\n");
    let r = &asked.reply;
    if r.not_found {
        println!("No he encontrado la respuesta en sus documentos. Los más cercanos:");
        for d in asked.thread.documents.iter().take(5) {
            println!("  {}", d.name);
        }
    } else {
        for b in &r.blocks {
            print!("{}", b.text);
            let mut markers: Vec<u32> = b.citations.iter().map(|c| c.marker).collect();
            markers.dedup();
            for m in markers {
                print!(" [{m}]");
            }
        }
        println!("\n\nFuentes:");
        let mut seen = Vec::new();
        for c in r.blocks.iter().flat_map(|b| &b.citations) {
            if !seen.contains(&c.marker) {
                seen.push(c.marker);
                println!(
                    "  [{}] {}",
                    c.marker,
                    asked.thread.sources[c.passage].title()
                );
            }
        }
    }
    println!(
        "\nrounds {:?}, {} passages sent, {} citations dropped, model {}, cost {} $, month {} $",
        asked.rounds,
        asked.thread.sources.len(),
        r.dropped,
        r.model,
        usd(r.cost_nano),
        usd(db.month_total(&opts.now)?)
    );
    Ok(())
}

fn usd(nano: i64) -> String {
    format!("{:.4}", nano as f64 / NANO_PER_USD as f64)
}
