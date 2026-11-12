//! Exact lexical/FTS ranking over lightweight pinned inventory; eager search is retained.
use crate::{Filters, Hit, sort_with_control};
use kortex_core::{Error, RequestControl, Result};
use kortex_storage::{ReadSession, SymbolInventoryEntry};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "projected_tests.rs"]
mod tests;

fn matches(filters: &Filters<'_>, symbol: &SymbolInventoryEntry) -> bool {
    filters.language.is_none_or(|s| symbol.language == s)
        && filters.kind.is_none_or(|s| symbol.kind == s)
        && filters
            .path_prefix
            .is_none_or(|s| symbol.file_path.starts_with(s))
}

/// O(symbols) lightweight ranking, with only the requested page decoded as full Symbols.
/// The caller must choose an explicit same-Store eager fallback for legacy projections.
pub fn query_projected_with_control(
    session: &ReadSession<'_>,
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
    let inventory = session.symbol_inventory()?;
    control.check()?;
    let mut ranked: BTreeMap<&str, (&SymbolInventoryEntry, f64, Vec<String>)> = BTreeMap::new();
    let lower = text.to_lowercase();
    for s in &inventory {
        control.check()?;
        if !matches(filters, s) {
            continue;
        }
        // Deliberately Rust Unicode lowercase, not SQLite's ASCII-only lower/LIKE.
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
    // Preserve the original global 500-candidate FTS cap BEFORE application filters.
    let fts = session.keyword(text, 500)?;
    control.check()?;
    let needed: BTreeSet<_> = fts.iter().map(|(id, _)| id.as_str()).collect();
    let mut fts_symbols = BTreeMap::new();
    for symbol in &inventory {
        control.check()?;
        if needed.contains(symbol.symbol_id.as_str()) {
            fts_symbols
                .entry(symbol.symbol_id.as_str())
                .or_insert(symbol);
        }
    }
    for (id, score) in &fts {
        control.check()?;
        if let Some((_, _, why)) = ranked.get_mut(id.as_str()) {
            why.push("fts5_bm25".into());
        } else if let Some(s) = fts_symbols.get(id.as_str()).filter(|s| matches(filters, s)) {
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
                .then(a.0.start_byte.cmp(&b.0.start_byte))
                .then(a.0.symbol_id.cmp(&b.0.symbol_id))
        },
        control,
    )?;
    let more = hits.len() > offset.saturating_add(limit);
    let mut page = Vec::with_capacity(limit.min(hits.len()));
    for (entry, score, why_matched) in hits.into_iter().skip(offset).take(limit) {
        control.check()?;
        let symbol = session.get_symbol(&entry.symbol_id)?.ok_or_else(|| {
            Error::new(
                "CORRUPT_INDEX",
                "ranked symbol is missing from pinned facts",
                6,
            )
        })?;
        if symbol.symbol_id != entry.symbol_id
            || symbol.name != entry.name
            || symbol.qualified_name != entry.qualified_name
            || symbol.file_path != entry.file_path
            || symbol.language != entry.language
            || symbol.kind != entry.kind
            || symbol.span.start_byte != entry.start_byte
        {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "ranked inventory differs from pinned symbol fact",
                6,
            ));
        }
        page.push(Hit {
            symbol,
            score,
            why_matched,
        });
    }
    control.check()?;
    Ok((page, more))
}
