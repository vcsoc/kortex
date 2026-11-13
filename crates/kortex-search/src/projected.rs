//! Exact lexical/FTS ranking over lightweight pinned inventory; eager search is retained.
use crate::{Filters, Hit, sort_with_control};
use kortex_core::{Error, RequestControl, Result};
use kortex_storage::{QueryCandidateFilter, ReadSession, SymbolInventoryEntry};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};
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

#[derive(Clone, Copy)]
struct Ranked<'a> {
    entry: &'a SymbolInventoryEntry,
    score: f64,
    lexical: Option<&'static str>,
    fts_count: usize,
}
impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Ranked<'_> {}
impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.entry.file_path.cmp(&other.entry.file_path))
            .then(self.entry.start_byte.cmp(&other.entry.start_byte))
            .then(self.entry.symbol_id.cmp(&other.entry.symbol_id))
    }
}

fn lower_prefix(value: &str, lower: &str) -> bool {
    if value.is_ascii() && lower.is_ascii() {
        value
            .as_bytes()
            .get(..lower.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(lower.as_bytes()))
    } else {
        value.to_lowercase().starts_with(lower)
    }
}
fn lower_contains(value: &str, lower: &str) -> bool {
    if value.is_ascii() && lower.is_ascii() {
        lower.is_empty()
            || value
                .as_bytes()
                .windows(lower.len())
                .any(|part| part.eq_ignore_ascii_case(lower.as_bytes()))
    } else {
        // str::to_lowercase includes contextual Unicode rules (e.g. final sigma).
        value.to_lowercase().contains(lower)
    }
}
fn lexical_score(
    entry: &SymbolInventoryEntry,
    text: &str,
    lower: &str,
) -> Option<(f64, &'static str)> {
    if entry.name == text {
        Some((1000., "exact_symbol"))
    } else if entry.qualified_name == text {
        Some((950., "qualified_name"))
    } else if lower_prefix(&entry.name, lower) {
        Some((700., "prefix"))
    } else if lower_contains(&entry.qualified_name, lower) {
        Some((500., "qualified_substring"))
    } else if lower_contains(&entry.file_path, lower) {
        Some((300., "path"))
    } else {
        None
    }
}

/// Lightweight ranking retains only offset+limit entries, decoding only the returned page.
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
    // The global FTS cap precedes filters. Keep first-score and repeated-why semantics.
    let candidates = session.query_scoped_candidate_inventory_with_control(
        text,
        &QueryCandidateFilter {
            language: filters.language,
            kind: filters.kind,
            path_prefix: filters.path_prefix,
        },
        control,
    )?;
    control.check()?;
    let mut fts_by_id = HashMap::with_capacity(candidates.fts.len());
    for (id, score) in &candidates.fts {
        let row = fts_by_id.entry(id.as_str()).or_insert((*score, 0usize));
        row.1 += 1;
    }
    let inventory = candidates.items;
    control.check()?;
    let lower = text.to_lowercase();
    let end = offset + limit;
    let mut best = BinaryHeap::with_capacity(end.min(inventory.len()));
    let mut count = 0usize;
    for entry in &inventory {
        control.check()?;
        if !matches(filters, entry) {
            continue;
        }
        let lexical = lexical_score(entry, text, &lower);
        let fts = fts_by_id.get(entry.symbol_id.as_str());
        let (score, why) = match (lexical, fts) {
            (Some((score, why)), _) => (score, Some(why)),
            (None, Some((score, _))) => (100. + (-score).min(99.), None),
            (None, None) => continue,
        };
        let ranked = Ranked {
            entry,
            score,
            lexical: why,
            fts_count: fts.map_or(0, |row| row.1),
        };
        count += 1;
        // The maximum heap entry is the WORST retained rank, not the best result.
        if best.len() < end {
            best.push(ranked);
        } else if let Some(mut worst) = best.peek_mut()
            && ranked < *worst
        {
            *worst = ranked;
        }
    }
    let mut hits = best.into_vec();
    sort_with_control(&mut hits, Ord::cmp, control)?;
    let more = count > end;
    let mut page = Vec::with_capacity(limit.min(hits.len()));
    for ranked in hits.into_iter().skip(offset).take(limit) {
        control.check()?;
        let entry = ranked.entry;
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
        let mut why_matched =
            Vec::with_capacity(usize::from(ranked.lexical.is_some()) + ranked.fts_count);
        if let Some(why) = ranked.lexical {
            why_matched.push(why.into());
        }
        for _ in 0..ranked.fts_count {
            why_matched.push("fts5_bm25".into());
        }
        page.push(Hit {
            symbol,
            score: ranked.score,
            why_matched,
        });
    }
    control.check()?;
    Ok((page, more))
}
