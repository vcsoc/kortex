//! Stable lexical/FTS5 retrieval; exact names outrank incidental matches.
pub mod hybrid;
#[cfg(feature = "local-embeddings")]
pub mod local;
pub mod retrieval;
use kortex_core::{Error, Result, Snapshot, Symbol};
use kortex_storage::Store;
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Debug, Serialize)]
pub struct Hit {
    pub symbol: Symbol,
    pub score: f64,
    pub why_matched: Vec<String>,
}
#[derive(Debug, Default, Clone)]
pub struct Filters<'a> {
    pub language: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub path_prefix: Option<&'a str>,
}
impl Filters<'_> {
    fn matches(&self, symbol: &Symbol) -> bool {
        self.language.is_none_or(|s| symbol.language == s)
            && self.kind.is_none_or(|s| symbol.kind == s)
            && self
                .path_prefix
                .is_none_or(|s| symbol.file_path.starts_with(s))
    }
}
pub fn query(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    limit: usize,
    offset: usize,
) -> Result<(Vec<Hit>, bool)> {
    query_filtered(store, snapshot, text, limit, offset, &Filters::default())
}
pub fn query_filtered(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    limit: usize,
    offset: usize,
    filters: &Filters<'_>,
) -> Result<(Vec<Hit>, bool)> {
    if text.len() > 4096 || text.trim().is_empty() || limit == 0 || limit > 500 || offset > 100000 {
        return Err(Error::new(
            "QUERY_LIMIT",
            "query 1..4096 bytes, limit 1..500, offset <=100000 required",
            2,
        ));
    }
    let mut ranked: BTreeMap<String, Hit> = BTreeMap::new();
    let lower = text.to_lowercase();
    for s in snapshot.symbols().filter(|s| filters.matches(s)) {
        let (score, why) = if s.name == text {
            (1000., "exact_symbol")
        } else if s.qualified_name == text {
            (950., "qualified_name")
        } else if s.name.to_lowercase().starts_with(&lower) {
            (700., "prefix")
        } else if s.qualified_name.to_lowercase().contains(&lower) {
            (500., "qualified_substring")
        } else if s.file_path.to_lowercase().contains(&lower) {
            (300., "path")
        } else {
            continue;
        };
        ranked.insert(
            s.symbol_id.clone(),
            Hit {
                symbol: s.clone(),
                score,
                why_matched: vec![why.into()],
            },
        );
    }
    for (id, score) in store.keyword(&snapshot.generation_id, text, 500)? {
        if let Some(hit) = ranked.get_mut(&id) {
            hit.why_matched.push("fts5_bm25".into());
        } else if let Some(s) = snapshot.symbol(&id).filter(|s| filters.matches(s)) {
            ranked.insert(
                id,
                Hit {
                    symbol: s.clone(),
                    score: 100. + (-score).min(99.),
                    why_matched: vec!["fts5_bm25".into()],
                },
            );
        }
    }
    let mut hits: Vec<_> = ranked.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.symbol.file_path.cmp(&b.symbol.file_path))
            .then(a.symbol.span.start_byte.cmp(&b.symbol.span.start_byte))
            .then(a.symbol.symbol_id.cmp(&b.symbol.symbol_id))
    });
    let more = hits.len() > offset.saturating_add(limit);
    Ok((hits.into_iter().skip(offset).take(limit).collect(), more))
}
