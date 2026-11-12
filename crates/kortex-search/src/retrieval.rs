//! Explicit local-only retrieval with native-provider vectors and weighted RRF.
use crate::{
    Filters, Hit,
    hybrid::{EmbeddingCache, LocalEmbedder, RankingWeights, reciprocal_rank_fusion_with_control},
    query_filtered_with_control, sort_with_control,
};
use kortex_core::{Error, RequestControl, Result, Snapshot};
use kortex_storage::Store;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub struct HybridOptions<'a> {
    pub filters: Filters<'a>,
    pub limit: usize,
    pub offset: usize,
    pub max_candidates: usize,
    pub weights: RankingWeights,
    pub deadline: Instant,
    pub cancelled: Option<&'a AtomicBool>,
}
impl Default for HybridOptions<'_> {
    fn default() -> Self {
        Self {
            filters: Filters::default(),
            limit: 20,
            offset: 0,
            max_candidates: 500,
            weights: RankingWeights::default(),
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct HybridResult {
    pub items: Vec<Hit>,
    pub more: bool,
    pub partial: bool,
    pub candidates_examined: usize,
    pub algorithm: &'static str,
    pub model_fingerprint: String,
    pub model_id: String,
    pub model_revision: String,
}
fn checkpoint(options: &HybridOptions<'_>) -> Result<()> {
    if options
        .cancelled
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        return Err(Error::new(
            "QUERY_CANCELLED",
            "local retrieval cancelled",
            9,
        ));
    }
    if Instant::now() >= options.deadline {
        return Err(Error::new(
            "QUERY_TIMEOUT",
            "local retrieval exceeded deadline",
            9,
        ));
    }
    Ok(())
}
/// Explicit library opt-in: callers must construct the native local provider from
/// separately approved artifacts. No repository setting creates a provider here.
pub fn query_hybrid(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    provider: &mut dyn LocalEmbedder,
    cache: &mut EmbeddingCache,
    options: &HybridOptions<'_>,
) -> Result<HybridResult> {
    let control = RequestControl::from_deadline(options.deadline);
    query_hybrid_with_control(store, snapshot, text, provider, cache, options, &control)
}
#[allow(clippy::too_many_arguments)]
pub fn query_hybrid_with_control(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    provider: &mut dyn LocalEmbedder,
    cache: &mut EmbeddingCache,
    options: &HybridOptions<'_>,
    control: &RequestControl,
) -> Result<HybridResult> {
    let control = control.restrict_deadline(options.deadline);
    let checkpoint = || {
        control.check()?;
        checkpoint(options)
    };
    checkpoint()?;
    if !(1..=10000).contains(&options.max_candidates)
        || !(1..=500).contains(&options.limit)
        || options.offset > 100000
    {
        return Err(Error::new(
            "QUERY_LIMIT",
            "hybrid candidates 1..10000, limit 1..500 and offset <=100000 required",
            2,
        ));
    }
    reciprocal_rank_fusion_with_control(&[], &[], options.weights, &control)?;
    let (lexical, _) =
        query_filtered_with_control(store, snapshot, text, 500, 0, &options.filters, &control)?;
    checkpoint()?;
    let query = cache.get_or_embed(provider, text)?;
    checkpoint()?;
    let mut documents = vec![];
    for symbol in snapshot.symbols() {
        checkpoint()?;
        if options.filters.matches(symbol) {
            documents.push(symbol);
        }
    }
    sort_with_control(
        &mut documents,
        |a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.span.start_byte.cmp(&b.span.start_byte))
                .then(a.symbol_id.cmp(&b.symbol_id))
        },
        &control,
    )?;
    let partial = documents.len() > options.max_candidates;
    documents.truncate(options.max_candidates);
    let mut vector = vec![];
    for symbol in &documents {
        checkpoint()?;
        let content = format!(
            "{} {}\n{}\n{}\n{}",
            symbol.language,
            symbol.qualified_name,
            symbol.signature,
            symbol.doc_comment.as_deref().unwrap_or(""),
            symbol.attributes["source_snippet"].as_str().unwrap_or("")
        )
        .chars()
        .take(8192)
        .collect::<String>();
        let embedding = cache.get_or_embed(provider, &content)?;
        let similarity = query
            .iter()
            .zip(&embedding)
            .map(|(q, d)| (*q as f64) * (*d as f64))
            .sum::<f64>();
        vector.push((symbol.symbol_id.clone(), similarity));
    }
    checkpoint()?;
    sort_with_control(
        &mut vector,
        |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)),
        &control,
    )?;
    let lexical_ids = lexical
        .iter()
        .map(|h| h.symbol.symbol_id.clone())
        .collect::<Vec<_>>();
    let vector_ids = vector.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let fused =
        reciprocal_rank_fusion_with_control(&lexical_ids, &vector_ids, options.weights, &control)?;
    let more = fused.len() > options.offset.saturating_add(options.limit);
    let lexical_by_id = lexical
        .into_iter()
        .map(|h| (h.symbol.symbol_id.clone(), h))
        .collect::<BTreeMap<_, _>>();
    let symbols = snapshot
        .symbols()
        .map(|s| (s.symbol_id.as_str(), s))
        .collect::<BTreeMap<_, _>>();
    let items = fused
        .into_iter()
        .skip(options.offset)
        .take(options.limit)
        .filter_map(|fused| {
            let symbol = symbols.get(fused.symbol_id.as_str())?;
            let mut why = lexical_by_id
                .get(&fused.symbol_id)
                .map(|h| h.why_matched.clone())
                .unwrap_or_default();
            if fused.vector_rank.is_some() {
                why.push("local_model_vector".into());
            }
            why.push("weighted_rrf_v1".into());
            Some(Hit {
                symbol: (*symbol).clone(),
                score: fused.score,
                why_matched: why,
            })
        })
        .collect();
    checkpoint()?;
    Ok(HybridResult {
        items,
        more,
        partial,
        candidates_examined: documents.len(),
        algorithm: "weighted_rrf_v1",
        model_fingerprint: provider.model().fingerprint().into(),
        model_id: provider.model().manifest().model_id.clone(),
        model_revision: provider.model().manifest().revision.clone(),
    })
}
