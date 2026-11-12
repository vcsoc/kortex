use super::*;
#[test]
fn actual_sort_work_observes_shared_cancellation_without_finishing_remaining_runs() {
    use std::{cell::Cell, time::Duration};
    let parent = RequestControl::new(Duration::from_secs(10)).unwrap();
    let child = parent.restrict_deadline(parent.deadline() + Duration::from_secs(10));
    let comparisons = Cell::new(0);
    let mut values = (0..100_000).rev().collect::<Vec<_>>();
    let error = sort_with_control(
        &mut values,
        |a, b| {
            comparisons.set(comparisons.get() + 1);
            if comparisons.get() == 300 {
                parent.cancel();
            }
            a.cmp(b)
        },
        &child,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert!(
        (300..1024).contains(&comparisons.get()),
        "cancellation must stop actual sort work, not wait for all 100k values"
    );
    let active = RequestControl::new(Duration::from_secs(10)).unwrap();
    for len in [0, 1, 255, 256, 257, 511, 513, 100_000] {
        let mut actual = (0..len)
            .map(|n| ((n * 37) % 101, n))
            .rev()
            .collect::<Vec<_>>();
        let mut expected = actual.clone();
        expected.sort_by_key(|v| v.0);
        sort_with_control(&mut actual, |a, b| a.0.cmp(&b.0), &active).unwrap();
        assert_eq!(actual, expected, "stable ordering, length {len}");
    }
}
// Independent retained linear/reference ranking, before borrowed page materialization.
pub(super) fn reference(
    store: &Store,
    snapshot: &Snapshot,
    text: &str,
    filters: &Filters<'_>,
) -> Vec<Hit> {
    let mut ranked = BTreeMap::new();
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
    for (id, score) in store.keyword(&snapshot.generation_id, text, 500).unwrap() {
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
    hits
}
#[test]
fn borrowed_rank_pages_equal_retained_linear_reference_with_real_fts_and_filters() {
    let temp = tempfile::tempdir().unwrap();
    for file in 0..4 {
        let mut source = String::new();
        for index in 0..40 {
            source.push_str(&format!("def item_{index}():\n    \"\"\"searchable documentation code {index}\"\"\"\n    pass\n"));
        }
        std::fs::write(temp.path().join(format!("f{file}.py")), source).unwrap();
    }
    let report = kortex_indexer::index_at_root(
        temp.path(),
        &kortex_config::Config::default(),
        &Default::default(),
    )
    .unwrap();
    let store = Store::open(temp.path(), false).unwrap();
    for filters in [
        Filters::default(),
        Filters {
            kind: Some("function"),
            ..Default::default()
        },
        Filters {
            language: Some("rust"),
            ..Default::default()
        },
        Filters {
            path_prefix: Some("f1"),
            ..Default::default()
        },
    ] {
        for query in [
            "item_1",
            "ITEM_1",
            "f2.py",
            "searchable",
            "documentation",
            "missing",
            "code",
        ] {
            let expected = reference(&store, &report.snapshot, query, &filters);
            for offset in [0, 1, 20, 150, 100000] {
                let (page, more) =
                    query_filtered(&store, &report.snapshot, query, 7, offset, &filters).unwrap();
                let expected_page: Vec<_> = expected.iter().skip(offset).take(7).collect();
                assert_eq!(
                    serde_json::to_value(page).unwrap(),
                    serde_json::to_value(expected_page).unwrap(),
                    "query={query} offset={offset}"
                );
                assert_eq!(more, expected.len() > offset + 7);
            }
        }
    }
}
