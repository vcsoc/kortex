//! Stable lexical/FTS5 retrieval; exact names outrank incidental matches.
pub mod hybrid;
#[cfg(feature = "local-embeddings")]
pub mod local;
pub mod persistence;
pub mod projected;
#[cfg(test)]
mod ranking_tests;
pub mod retrieval;
use kortex_core::{Error, RequestControl, Result, Snapshot, Symbol};
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
    let control = RequestControl::new(std::time::Duration::from_secs(10))?;
    query_filtered_with_control(store, snapshot, text, limit, offset, filters, &control)
}
pub fn query_filtered_with_control(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    limit: usize,
    offset: usize,
    filters: &Filters<'_>,
    control: &RequestControl,
) -> Result<(Vec<Hit>, bool)> {
    control.check()?;
    if text.len() > 4096 || text.trim().is_empty() || limit == 0 || limit > 500 || offset > 100000 {
        return Err(Error::new(
            "QUERY_LIMIT",
            "query 1..4096 bytes, limit 1..500, offset <=100000 required",
            2,
        ));
    }
    // Borrow symbols until pagination. Cloning every ranked source snippet and doing a
    // linear Snapshot::symbol scan for each FTS row costs heavily at 100k symbols.
    let mut ranked: BTreeMap<&str, (&Symbol, f64, Vec<String>)> = BTreeMap::new();
    let lower = text.to_lowercase();
    for s in snapshot.symbols() {
        control.check()?;
        if !filters.matches(s) {
            continue;
        }
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
        ranked.insert(&s.symbol_id, (s, score, vec![why.into()]));
    }
    control.check()?;
    let fts = store.keyword(&snapshot.generation_id, text, 500)?;
    control.check()?;
    let needed: std::collections::BTreeSet<_> = fts.iter().map(|(id, _)| id.as_str()).collect();
    let mut fts_symbols = BTreeMap::new();
    for symbol in snapshot.symbols() {
        control.check()?;
        if !needed.contains(symbol.symbol_id.as_str()) {
            continue;
        }
        // Preserve Snapshot::symbol's first-ID lookup semantics even on unvalidated input.
        fts_symbols
            .entry(symbol.symbol_id.as_str())
            .or_insert(symbol);
    }
    for (id, score) in &fts {
        control.check()?;
        if let Some((_, _, why)) = ranked.get_mut(id.as_str()) {
            why.push("fts5_bm25".into());
        } else if let Some(s) = fts_symbols.get(id.as_str()).filter(|s| filters.matches(s)) {
            ranked.insert(
                s.symbol_id.as_str(),
                (*s, 100. + (-score).min(99.), vec!["fts5_bm25".into()]),
            );
        }
    }
    let mut hits: Vec<_> = ranked.into_values().collect();
    sort_with_control(
        &mut hits,
        |a, b| {
            b.1.total_cmp(&a.1)
                .then(a.0.file_path.cmp(&b.0.file_path))
                .then(a.0.span.start_byte.cmp(&b.0.span.start_byte))
                .then(a.0.symbol_id.cmp(&b.0.symbol_id))
        },
        control,
    )?;
    let more = hits.len() > offset.saturating_add(limit);
    let page = hits
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(symbol, score, why_matched)| Hit {
            symbol: symbol.clone(),
            score,
            why_matched,
        })
        .collect();
    control.check()?;
    Ok((page, more))
}

/// Stable bounded-run merge sort with cooperative checkpoints, including during merging.
/// A single 256-element run is indivisible; no request waits for an entire 100k sort.
pub fn sort_with_control<T: Clone>(
    values: &mut [T],
    compare: impl Fn(&T, &T) -> std::cmp::Ordering,
    control: &RequestControl,
) -> Result<()> {
    for run in values.chunks_mut(256) {
        control.check()?;
        run.sort_by(&compare);
    }
    let mut width = 256;
    while width < values.len() {
        for run in values.chunks_mut(width * 2) {
            control.check()?;
            let mid = width.min(run.len());
            let mut left = 0;
            let mut right = mid;
            let mut merged = Vec::with_capacity(run.len());
            while left < mid || right < run.len() {
                control.check()?;
                if right == run.len() || (left < mid && !compare(&run[left], &run[right]).is_gt()) {
                    merged.push(run[left].clone());
                    left += 1;
                } else {
                    merged.push(run[right].clone());
                    right += 1;
                }
            }
            run.clone_from_slice(&merged);
        }
        width *= 2;
    }
    control.check()
}
