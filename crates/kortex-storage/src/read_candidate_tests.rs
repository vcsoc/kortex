use super::*;
use crate::read_tests::fixture;

#[test]
fn exact_candidate_union_preserves_unicode_literals_filters_fts500_and_order() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let mut s = fixture("candidates");
    let template = s.files[0].symbols[0].clone();
    s.files[0].symbols = (0..660)
        .rev()
        .map(|i| {
            let mut symbol = template.clone();
            symbol.symbol_id = format!("candidate{i:04}");
            symbol.name = match i {
                0 => "İTEM".into(),
                1 => "Éclair".into(),
                2 => "Σcope".into(),
                3 => "a\0B%_literal".into(),
                _ => format!("f{i}"),
            };
            symbol.qualified_name = format!("MÓDULO.{}", symbol.name);
            symbol.file_path = format!("DÍR/{i}%_\0.py");
            symbol.language = if i % 2 == 0 { "python" } else { "rust" }.into();
            symbol.kind = if i % 3 == 0 { "Function" } else { "Method" }.into();
            symbol.attributes = serde_json::json!({"source_snippet": "globalneedle"});
            symbol.span.start_byte = i;
            symbol
        })
        .collect();
    s.edges.clear();
    writer.commit(&s, &lock).unwrap();
    // Full duplicate JSON is not required by the projected candidate operation.
    writer.conn.execute_batch("UPDATE generations SET snapshot='invalid'; UPDATE files SET facts_json='invalid'; UPDATE nodes SET data_json='invalid'").unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    for text in [
        "i\u{307}t",
        "écl",
        "σc",
        "módulo",
        "dír",
        "a",
        "%_",
        "literal",
        "globalneedle",
        "missing",
        "Function",
    ] {
        for filter in [
            QueryCandidateFilter::default(),
            QueryCandidateFilter {
                language: Some("python"),
                ..Default::default()
            },
            QueryCandidateFilter {
                language: Some("rust"),
                kind: Some("Method"),
                path_prefix: Some("DÍR/"),
            },
            QueryCandidateFilter {
                path_prefix: Some("dír/"),
                ..Default::default()
            },
            QueryCandidateFilter {
                path_prefix: Some("DÍR/0%_\0"),
                ..Default::default()
            },
        ] {
            let session = reader.read_session().unwrap();
            let actual = session.query_candidate_inventory(text, &filter).unwrap();
            assert_eq!(actual.source, MetadataSource::Projection);
            let fts = session.keyword(text, 500).unwrap();
            assert_eq!(actual.fts, fts);
            if text == "globalneedle" {
                assert_eq!(fts.len(), 500);
            }
            let lower = text.to_lowercase();
            let expected: Vec<_> = s
                .symbols()
                .filter(|symbol| {
                    filter.language.is_none_or(|v| symbol.language == v)
                        && filter.kind.is_none_or(|v| symbol.kind == v)
                        && filter
                            .path_prefix
                            .is_none_or(|v| symbol.file_path.starts_with(v))
                        && (symbol.name == text
                            || symbol.qualified_name == text
                            || symbol.name.to_lowercase().starts_with(&lower)
                            || symbol.qualified_name.to_lowercase().contains(&lower)
                            || symbol.file_path.to_lowercase().contains(&lower)
                            || fts.iter().any(|(id, _)| id == &symbol.symbol_id))
                })
                .collect();
            assert_eq!(actual.items.len(), expected.len(), "{text:?} {filter:?}");
            for (row, expected) in actual.items.iter().zip(expected) {
                assert_eq!(
                    (
                        &row.symbol_id,
                        &row.name,
                        &row.qualified_name,
                        &row.file_path,
                        &row.language,
                        &row.kind,
                        row.start_byte
                    ),
                    (
                        &expected.symbol_id,
                        &expected.name,
                        &expected.qualified_name,
                        &expected.file_path,
                        &expected.language,
                        &expected.kind,
                        expected.span.start_byte
                    )
                );
            }
        }
    }
    for text in ["a\0b%_", "\0.py"] {
        let session = reader.read_session().unwrap();
        // Existing FTS5 rejects embedded-NUL query strings. Preserve that error rather
        // than silently truncating the query or manufacturing lexical-only success.
        let original = session.keyword(text, 500).unwrap_err();
        let actual = session
            .query_candidate_inventory(text, &Default::default())
            .unwrap_err();
        assert_eq!(
            (actual.code, actual.message, actual.exit),
            (original.code, original.message, original.exit)
        );
    }
    let first = reader
        .read_session()
        .unwrap()
        .query_candidate_inventory("écl", &Default::default())
        .unwrap();
    writer.commit(&fixture("new"), &lock).unwrap();
    assert_eq!(
        reader
            .read_session()
            .unwrap()
            .query_candidate_inventory("écl", &Default::default())
            .unwrap()
            .items,
        first.items
    );
    let fresh = Store::open(d.path(), false).unwrap();
    assert!(
        fresh
            .read_session()
            .unwrap()
            .query_candidate_inventory("écl", &Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    for text in ["", " \n", &"a".repeat(4097)] {
        assert_eq!(
            reader
                .read_session()
                .unwrap()
                .query_candidate_inventory(text, &Default::default())
                .unwrap_err()
                .code,
            "QUERY_LIMIT"
        );
    }
}

#[test]
fn candidate_validation_is_shared_only_by_the_same_pinned_store_and_never_caches_failure() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture("old"), &lock).unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    assert!(reader.projection_source.get().is_none());
    reader
        .read_session()
        .unwrap()
        .read_projection_source()
        .unwrap();
    assert_eq!(
        reader.projection_source.get(),
        Some(&MetadataSource::Projection)
    );
    writer
        .conn
        .execute_batch("DELETE FROM read_edge_order WHERE ordinal=1")
        .unwrap();
    // New sessions on the old Store still read the exact old SQLite snapshot.
    assert_eq!(
        reader
            .read_session()
            .unwrap()
            .outgoing_edges_ordered("symbol0", None, 10)
            .unwrap()
            .items
            .len(),
        3
    );
    let fresh = Store::open(d.path(), false).unwrap();
    for _ in 0..2 {
        assert_eq!(
            fresh
                .read_session()
                .unwrap()
                .query_candidate_inventory("old", &Default::default())
                .unwrap_err()
                .code,
            "CORRUPT_INDEX"
        );
        assert!(fresh.projection_source.get().is_none());
    }
    writer.commit(&fixture("new"), &lock).unwrap();
    assert_eq!(
        fresh
            .read_session()
            .unwrap()
            .read_projection_source()
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    let newer = Store::open(d.path(), false).unwrap();
    assert_eq!(
        newer
            .read_session()
            .unwrap()
            .query_candidate_inventory("new", &Default::default())
            .unwrap()
            .items
            .len(),
        3
    );
}

#[test]
fn candidate_legacy_is_explicit_and_malformed_excluded_utf8_still_refuses() {
    for mutation in [
        "UPDATE nodes SET name=CAST(x'ff' AS TEXT) WHERE id='symbol1'",
        "UPDATE nodes SET file_path=CAST(x'ff' AS TEXT) WHERE id='symbol1'",
        "UPDATE read_symbol_inventory SET language=CAST(x'ff' AS TEXT) WHERE id='symbol1'",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_symbol_inventory SET start_byte='wrong' WHERE id='symbol1'",
    ] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        assert_eq!(
            reader
                .read_session()
                .unwrap()
                .query_candidate_inventory(
                    "absent",
                    &QueryCandidateFilter {
                        language: Some("excluded"),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .code,
            "CORRUPT_INDEX",
            "{mutation}"
        );
    }
    for mutation in [
        "DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_edge_order; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order",
        "UPDATE generation_read_manifest SET manifest_json=replace(manifest_json,'\"version\":2','\"version\":1'); DROP TABLE read_file_order",
    ] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let actual = reader
            .read_session()
            .unwrap()
            .query_candidate_inventory("old", &Default::default())
            .unwrap();
        assert_eq!(actual.source, MetadataSource::LegacySnapshot);
        assert_eq!(
            actual
                .items
                .iter()
                .map(|s| s.symbol_id.as_str())
                .collect::<Vec<_>>(),
            ["symbol2", "symbol1", "symbol0"]
        );
        writer
            .conn
            .execute_batch("UPDATE generations SET snapshot='invalid'")
            .unwrap();
        let corrupt = Store::open(d.path(), false).unwrap();
        assert_eq!(
            corrupt
                .read_session()
                .unwrap()
                .query_candidate_inventory("old", &Default::default())
                .unwrap_err()
                .code,
            "CORRUPT_INDEX"
        );
    }
}

#[test]
fn order_validation_refuses_duplicate_fractional_null_and_gap_ordinals_without_unique_constraints()
{
    for ordinals in ["0,0,2", "0,0.5,2", "0,NULL,2", "0,1,3"] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        // A malformed replacement schema must not turn uniqueness into an assumption.
        writer.conn.execute_batch("CREATE TABLE replacement(generation_id,id,ordinal); INSERT INTO replacement SELECT generation_id,id,ordinal FROM read_edge_order; DROP TABLE read_edge_order; ALTER TABLE replacement RENAME TO read_edge_order").unwrap();
        for (i, value) in ordinals.split(',').enumerate() {
            writer
                .conn
                .execute_batch(&format!(
                    "UPDATE read_edge_order SET ordinal={value} WHERE id='edge{i}'"
                ))
                .unwrap();
        }
        let reader = Store::open(d.path(), false).unwrap();
        assert_eq!(
            reader
                .read_session()
                .unwrap()
                .read_projection_source()
                .unwrap_err()
                .code,
            "CORRUPT_INDEX",
            "{ordinals}"
        );
    }
}
