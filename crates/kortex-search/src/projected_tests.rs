use super::*;
use crate::ranking_tests::reference;

#[test]
fn allocation_free_ascii_checks_equal_rust_unicode_lowercase_including_literal_bytes() {
    let parts = [
        "", "A", "z", "_", "%", "\0", ".", "Ä", "İ", "ΟΣ", "Σ", "ς", "σ", "i\u{307}",
    ];
    for left in parts {
        for right in parts {
            let value = format!("{left}{right}");
            for query in parts {
                let lower = query.to_lowercase();
                assert_eq!(
                    lower_prefix(&value, &lower),
                    value.to_lowercase().starts_with(&lower),
                    "value={value:?} query={query:?}"
                );
                assert_eq!(
                    lower_contains(&value, &lower),
                    value.to_lowercase().contains(&lower),
                    "value={value:?} query={query:?}"
                );
            }
        }
    }
}

#[test]
fn bounded_heap_pages_equal_full_sort_for_large_tie_sets_and_all_score_classes() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = "def needle():\n    pass\ndef NEEDLE():\n    pass\n".to_owned();
    for index in 0..1500 {
        source.push_str(&format!(
            "def needle_{index}():\n    \"\"\"needle searchable text\"\"\"\n    pass\n"
        ));
    }
    std::fs::write(temp.path().join("needle_%_.py"), source).unwrap();
    let report = kortex_indexer::index_at_root(
        temp.path(),
        &kortex_config::Config::default(),
        &Default::default(),
    )
    .unwrap();
    let store = kortex_storage::Store::open(temp.path(), false).unwrap();
    let session = store.read_session().unwrap();
    let control = RequestControl::new(std::time::Duration::from_secs(60)).unwrap();
    for query in [
        "needle",
        "NEED",
        "searchable",
        "needle_%_",
        "%",
        "_",
        "not-present",
    ] {
        let expected = reference(&store, &report.snapshot, query, &Filters::default());
        for (offset, limit) in [
            (0, 1),
            (0, 20),
            (1, 7),
            (255, 1),
            (256, 257),
            (499, 500),
            (1000, 500),
            (1500, 500),
            (100000, 500),
        ] {
            let (actual, more) = query_projected_with_control(
                &session,
                query,
                limit,
                offset,
                &Filters::default(),
                &control,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected.iter().skip(offset).take(limit).collect::<Vec<_>>())
                    .unwrap(),
                "query={query} offset={offset} limit={limit}"
            );
            assert_eq!(more, expected.len() > offset + limit);
        }
    }
}

#[test]
fn projected_pages_equal_retained_reference_for_unicode_filters_ties_and_500_fts_cap() {
    let temp = tempfile::tempdir().unwrap();
    for file in 0..4 {
        let mut source = "def ÄLPHA():\n    pass\ndef İstanbul():\n    pass\n".to_owned();
        for index in 0..165 {
            source.push_str(&format!("def item_{index}():\n    \"\"\"searchable documentation code {index}\"\"\"\n    pass\n"));
        }
        std::fs::write(temp.path().join(format!("Ärea_{file}.py")), source).unwrap();
    }
    let report = kortex_indexer::index_at_root(
        temp.path(),
        &kortex_config::Config::default(),
        &Default::default(),
    )
    .unwrap();
    let store = kortex_storage::Store::open(temp.path(), false).unwrap();
    let session = store.read_session().unwrap();
    assert_eq!(
        session.read_projection_source().unwrap(),
        kortex_storage::MetadataSource::Projection
    );
    let control = RequestControl::new(std::time::Duration::from_secs(60)).unwrap();
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
            path_prefix: Some("Ärea_1"),
            ..Default::default()
        },
        Filters {
            language: Some("python"),
            kind: Some("function"),
            path_prefix: Some("Ärea_3"),
        },
    ] {
        for query in [
            "item_1",
            "ITEM_1",
            "ÄLPHA",
            "älp",
            "İstanbul",
            "i\u{307}",
            "ärea",
            "area",
            "documentation",
            "missing",
            "code",
        ] {
            let expected = reference(&store, &report.snapshot, query, &filters);
            for offset in [0, 1, 7, 20, 160, 499, 500, 660, 100000] {
                let (page, more) =
                    query_projected_with_control(&session, query, 7, offset, &filters, &control)
                        .unwrap();
                assert_eq!(
                    serde_json::to_value(page).unwrap(),
                    serde_json::to_value(expected.iter().skip(offset).take(7).collect::<Vec<_>>())
                        .unwrap(),
                    "query={query} offset={offset} filters={filters:?}"
                );
                assert_eq!(more, expected.len() > offset + 7);
            }
        }
    }
    control.cancel();
    assert_eq!(
        query_projected_with_control(&session, "item", 7, 0, &Filters::default(), &control)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
}
