use super::*;
use crate::read_tests::fixture;

const DROP_FAST: &str = "DROP TABLE query_symbol_inventory; DROP TABLE generation_query_manifest;";
const DROP_USED: &str = "DROP TABLE query_symbol_inventory; DROP TABLE generation_query_manifest; DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order;";

#[test]
fn query_approval_never_authorizes_unused_corrupt_edges_or_full_helpers() {
    for mutation in [
        "DELETE FROM read_edge_order WHERE ordinal=1",
        "PRAGMA foreign_keys=OFF; UPDATE read_edge_order SET id='orphan' WHERE ordinal=1",
        "DROP TABLE read_edge_order",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_edge_order SET ordinal='bad' WHERE ordinal=1",
    ] {
        for fast in [true, false] {
            let d = crate::test_support::tempdir().unwrap();
            let mut writer = Store::open(d.path(), true).unwrap();
            let lock = writer.writer_lock().unwrap();
            let snapshot = fixture("same");
            writer.commit(&snapshot, &lock).unwrap();
            if !fast {
                writer.conn.execute_batch(DROP_FAST).unwrap();
            }
            writer.conn.execute_batch(mutation).unwrap();
            let reader = Store::open(d.path(), false).unwrap();
            let session = reader.read_session().unwrap();
            assert_eq!(
                session.query_scope_source().unwrap(),
                MetadataSource::Projection
            );
            assert_eq!(
                session
                    .query_file_inventory_snapshot_order()
                    .unwrap()
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect::<Vec<_>>(),
                snapshot
                    .files
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                session.query_parser_diagnostic_presence().unwrap().len(),
                snapshot.files.len()
            );
            let candidates = session
                .query_scoped_candidate_inventory("same", &Default::default())
                .unwrap();
            assert_eq!(candidates.items.len(), 3);
            for entry in &candidates.items {
                session.query_symbol(entry).unwrap();
            }
            assert!(reader.query_projection.get().is_some());
            assert!(reader.projection_source.get().is_none());
            for error in [
                session.read_projection_source().unwrap_err(),
                session.file_inventory_snapshot_order().unwrap_err(),
                session.parser_diagnostic_presence().unwrap_err(),
                session
                    .query_candidate_inventory("same", &Default::default())
                    .unwrap_err(),
                session
                    .outgoing_edges_ordered("symbol0", None, 10)
                    .unwrap_err(),
            ] {
                assert_eq!(error.code, "CORRUPT_INDEX", "{mutation}");
            }
            assert!(reader.projection_source.get().is_none());
            assert_eq!(
                session.query_scope_source().unwrap(),
                MetadataSource::Projection
            );
        }
    }
}

#[test]
fn query_used_evidence_corruption_never_downgrades_to_legacy() {
    for mutation in [
        "DELETE FROM read_file_order WHERE ordinal=1",
        "DELETE FROM read_file_diagnostics WHERE id=(SELECT id FROM read_file_diagnostics LIMIT 1)",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_file_diagnostics SET has_diagnostics=2",
        "PRAGMA foreign_keys=OFF; UPDATE read_file_diagnostics SET id='orphan' WHERE id=(SELECT id FROM read_file_diagnostics LIMIT 1)",
        "PRAGMA foreign_keys=OFF; UPDATE read_file_order SET id='orphan' WHERE ordinal=1",
        "UPDATE generation_metadata SET metadata_json=replace(metadata_json,'same','wrong')",
        "UPDATE generation_read_manifest SET manifest_json='{}'",
        "UPDATE generation_query_manifest SET manifest_json='{}'",
        "UPDATE generation_query_manifest SET manifest_json=replace(manifest_json,'\"version\":1','\"version\":99')",
        "DELETE FROM generation_query_manifest",
        "DROP TABLE query_symbol_inventory",
        "DELETE FROM query_symbol_inventory WHERE ordinal=1",
        "PRAGMA foreign_keys=OFF; UPDATE query_symbol_inventory SET id='orphan' WHERE ordinal=1",
        "DELETE FROM generation_read_manifest",
    ] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("same"), &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        for _ in 0..2 {
            let err = reader
                .read_session()
                .and_then(|s| s.query_scoped_candidate_inventory("same", &Default::default()))
                .unwrap_err();
            assert_eq!(err.code, "CORRUPT_INDEX", "{mutation}");
            assert!(reader.query_projection.get().is_none());
            assert!(reader.projection_source.get().is_none());
        }
    }
}

#[test]
fn query_refuses_excluded_bad_utf8_types_and_all_order_defects() {
    for fast in [true, false] {
        let table = if fast {
            "query_symbol_inventory"
        } else {
            "nodes"
        };
        for column in [
            "id",
            "name",
            "qualified_name",
            "file_path",
            "language",
            "kind",
            "start_byte",
        ] {
            let target = if !fast && ["language", "start_byte"].contains(&column) {
                "read_symbol_inventory"
            } else {
                table
            };
            for value in ["CAST(x'ff' AS TEXT)", "x'31'", "NULL", "0.5"] {
                let d = crate::test_support::tempdir().unwrap();
                let mut writer = Store::open(d.path(), true).unwrap();
                let lock = writer.writer_lock().unwrap();
                writer.commit(&fixture("same"), &lock).unwrap();
                if !fast {
                    writer.conn.execute_batch(DROP_FAST).unwrap();
                }
                // A replacement schema tests validation independently of DDL constraints/affinity.
                writer.conn.execute_batch(&format!("PRAGMA foreign_keys=OFF; CREATE TABLE replacement AS SELECT * FROM {target}; DROP TABLE {target}; ALTER TABLE replacement RENAME TO {target}; ALTER TABLE {target} RENAME COLUMN {column} TO original_value; ALTER TABLE {target} ADD COLUMN {column}; UPDATE {target} SET {column}=original_value; UPDATE {target} SET {column}={value} WHERE rowid=(SELECT rowid FROM {target} LIMIT 1)")).unwrap();
                let reader = Store::open(d.path(), false).unwrap();
                assert_eq!(
                    reader
                        .read_session()
                        .unwrap()
                        .query_scoped_candidate_inventory(
                            "absent",
                            &QueryCandidateFilter {
                                language: Some("excluded"),
                                ..Default::default()
                            }
                        )
                        .unwrap_err()
                        .code,
                    "CORRUPT_INDEX",
                    "{fast} {column} {value}"
                );
            }
        }
    }
    for table in [
        "query_symbol_inventory",
        "read_symbol_inventory",
        "read_file_order",
    ] {
        for ordinals in [
            "0,0,2",
            "0,0.5,2",
            "0,NULL,2",
            "0,1,3",
            "0,'1',2",
            "0,x'31',2",
            "-1,1,2",
        ] {
            let d = crate::test_support::tempdir().unwrap();
            let mut writer = Store::open(d.path(), true).unwrap();
            let lock = writer.writer_lock().unwrap();
            writer.commit(&fixture("same"), &lock).unwrap();
            if table == "read_symbol_inventory" {
                writer.conn.execute_batch(DROP_FAST).unwrap();
            }
            writer.conn.execute_batch(&format!("CREATE TABLE replacement AS SELECT * FROM {table}; DROP TABLE {table}; ALTER TABLE replacement RENAME TO {table}; ALTER TABLE {table} RENAME COLUMN ordinal TO original_ordinal; ALTER TABLE {table} ADD COLUMN ordinal")).unwrap();
            for (i, value) in ordinals.split(',').enumerate() {
                writer
                    .conn
                    .execute_batch(&format!(
                        "UPDATE {table} SET ordinal={value} WHERE original_ordinal={i}"
                    ))
                    .unwrap();
            }
            let reader = Store::open(d.path(), false).unwrap();
            assert_eq!(
                reader
                    .read_session()
                    .unwrap()
                    .query_scope_source()
                    .unwrap_err()
                    .code,
                "CORRUPT_INDEX",
                "{table} {ordinals}"
            );
            assert!(reader.query_projection.get().is_none());
        }
    }
}

#[test]
fn query_selected_facts_compare_every_raw_field_not_folded_values() {
    for (column, value) in [
        ("id", "different"),
        ("name", "SAME"),
        ("qualified_name", "different"),
        ("file_path", "different"),
        ("language", "different"),
        ("kind", "different"),
        ("start_byte", "123"),
    ] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("same"), &lock).unwrap();
        if column == "id" {
            // A JSON fact identity mismatch is distinct from projection orphan validation.
            let raw: String = writer
                .conn
                .query_row("SELECT data_json FROM nodes WHERE id='symbol1'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            writer
                .conn
                .execute(
                    "UPDATE nodes SET data_json=?1 WHERE id='symbol1'",
                    [raw.replace("symbol1", "different")],
                )
                .unwrap();
        } else {
            writer
                .conn
                .execute(
                    &format!("UPDATE query_symbol_inventory SET {column}=?1 WHERE id='symbol1'"),
                    [value],
                )
                .unwrap();
        }
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        let candidates = session
            .query_scoped_candidate_inventory("same", &Default::default())
            .unwrap();
        let selected = candidates
            .items
            .iter()
            .find(|s| s.symbol_id == "symbol1")
            .unwrap();
        assert_eq!(
            session.query_symbol(selected).unwrap_err().code,
            "CORRUPT_INDEX",
            "{column}"
        );
    }
}

#[test]
fn query_scope_cache_is_pinned_store_local_with_truthful_old_writer_and_legacy_paths() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture("same"), &lock).unwrap();
    let old = Store::open(d.path(), false).unwrap();
    old.read_session().unwrap().query_scope_source().unwrap();
    assert!(old.projection_source.get().is_none());
    writer
        .conn
        .execute_batch("DELETE FROM read_file_order WHERE ordinal=1")
        .unwrap();
    let bad = Store::open(d.path(), false).unwrap();
    assert_eq!(
        bad.read_session()
            .unwrap()
            .query_scope_source()
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(bad.query_projection.get().is_none());
    writer.commit(&fixture("next"), &lock).unwrap();
    assert_eq!(
        old.read_session()
            .unwrap()
            .query_scoped_candidate_inventory("same", &Default::default())
            .unwrap()
            .items
            .len(),
        3
    );
    assert!(bad.read_session().unwrap().query_scope_source().is_err());
    for (i, mutation) in [
        DROP_FAST.to_string(),
        format!("{DROP_USED} DROP TABLE read_edge_order"),
        format!(
            "{DROP_FAST} UPDATE generation_read_manifest SET manifest_json=replace(manifest_json,'\"version\":2','\"version\":1'); DROP TABLE read_file_order"
        ),
    ].into_iter().enumerate() {
        let mut current = fixture("current");
        current.generation_id = format!("current{i}");
        writer.commit(&current, &lock).unwrap();
        writer.conn.execute_batch(&mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        let candidates = session
            .query_scoped_candidate_inventory("current", &Default::default())
            .unwrap();
        assert_eq!(candidates.items.len(), 3);
        assert_eq!(
            candidates.source,
            if mutation == DROP_FAST {
                MetadataSource::Projection
            } else {
                MetadataSource::LegacySnapshot
            }
        );
        assert!(reader.projection_source.get().is_none());
        assert_eq!(
            session.query_file_inventory_snapshot_order().unwrap().len(),
            3
        );
        assert_eq!(session.query_parser_diagnostic_presence().unwrap().len(), 3);
        if candidates.source == MetadataSource::LegacySnapshot {
            writer
                .conn
                .execute_batch("UPDATE generations SET snapshot='invalid'")
                .unwrap();
            let corrupt = Store::open(d.path(), false).unwrap();
            assert_eq!(
                corrupt
                    .read_session()
                    .unwrap()
                    .query_scoped_candidate_inventory("current", &Default::default())
                    .unwrap_err()
                    .code,
                "CORRUPT_INDEX"
            );
        }
    }
    assert!(writer.query_projection.get().is_none());
    assert!(writer.read_session().is_err());
}

#[test]
fn fast_and_old_writer_candidates_match_unicode_oracle_global_fts500_and_snapshot_order() {
    for fast in [true, false] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        let mut s = fixture("unicode");
        let template = s.files[0].symbols[0].clone();
        s.files[0].symbols = (0..660)
            .rev()
            .map(|i| {
                let mut symbol = template.clone();
                symbol.symbol_id = format!("candidate{i:04}");
                symbol.name = match i {
                    0 => "İTEM".into(),
                    1 => "Éclair".into(),
                    2 => "ΟΣΟΣ".into(),
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
        if !fast {
            writer.conn.execute_batch(DROP_FAST).unwrap();
        }
        writer.conn.execute_batch("UPDATE generations SET snapshot='invalid'; UPDATE files SET facts_json='invalid'; UPDATE nodes SET data_json='invalid'").unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        for text in [
            "i\u{307}t",
            "écl",
            "οσος",
            "οσοσ",
            "módulo",
            "dír",
            "a",
            "%_",
            "literal",
            "globalneedle",
            "missing",
        ] {
            for filter in [
                QueryCandidateFilter::default(),
                QueryCandidateFilter {
                    language: Some("rust"),
                    kind: Some("Method"),
                    path_prefix: Some("DÍR/"),
                },
                QueryCandidateFilter {
                    path_prefix: Some("DÍR/0%_\0"),
                    ..Default::default()
                },
            ] {
                let actual = session
                    .query_scoped_candidate_inventory(text, &filter)
                    .unwrap();
                let fts = session.keyword(text, 500).unwrap();
                assert_eq!(actual.fts, fts);
                if text == "globalneedle" {
                    assert_eq!(fts.len(), 500);
                }
                let lower = text.to_lowercase();
                let expected: Vec<_> = s
                    .symbols()
                    .filter(|s| {
                        filter.language.is_none_or(|v| s.language == v)
                            && filter.kind.is_none_or(|v| s.kind == v)
                            && filter
                                .path_prefix
                                .is_none_or(|v| s.file_path.starts_with(v))
                            && (fts.iter().any(|(id, _)| *id == s.symbol_id)
                                || s.name == text
                                || s.qualified_name == text
                                || s.name.to_lowercase().starts_with(&lower)
                                || s.qualified_name.to_lowercase().contains(&lower)
                                || s.file_path.to_lowercase().contains(&lower))
                    })
                    .map(|s| SymbolInventoryEntry {
                        symbol_id: s.symbol_id.clone(),
                        name: s.name.clone(),
                        qualified_name: s.qualified_name.clone(),
                        file_path: s.file_path.clone(),
                        language: s.language.clone(),
                        kind: s.kind.clone(),
                        start_byte: s.span.start_byte,
                    })
                    .collect();
                assert_eq!(actual.items, expected, "{fast} {text:?} {filter:?}");
                assert_eq!(actual.source, MetadataSource::Projection);
            }
        }
        for text in ["a\0b%_", "\0.py"] {
            let original = session.keyword(text, 500).unwrap_err();
            let actual = session
                .query_scoped_candidate_inventory(text, &Default::default())
                .unwrap_err();
            assert_eq!(
                (actual.code, actual.message, actual.exit),
                (original.code, original.message, original.exit)
            );
        }
        assert!(reader.projection_source.get().is_none());
    }
}

#[test]
fn fast_projection_publication_failure_rolls_back_and_can_retry_with_pinned_readers() {
    for table in ["query_symbol_inventory", "generation_query_manifest"] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        let old = Store::open(d.path(), false).unwrap();
        let before = old
            .read_session()
            .unwrap()
            .query_scoped_candidate_inventory("old", &Default::default())
            .unwrap();
        writer.conn.execute_batch(&format!("CREATE TRIGGER reject_query BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'query projection failure'); END")).unwrap();
        assert!(writer.commit(&fixture("new"), &lock).is_err());
        assert_eq!(writer.load().unwrap().generation_id, "old");
        assert_eq!(writer.integrity().unwrap(), "ok");
        writer
            .conn
            .execute_batch("DROP TRIGGER reject_query")
            .unwrap();
        writer.commit(&fixture("new"), &lock).unwrap();
        assert_eq!(
            old.read_session()
                .unwrap()
                .query_scoped_candidate_inventory("old", &Default::default())
                .unwrap()
                .items,
            before.items
        );
        let fresh = Store::open(d.path(), false).unwrap();
        assert_eq!(
            fresh
                .read_session()
                .unwrap()
                .query_scoped_candidate_inventory("new", &Default::default())
                .unwrap()
                .items
                .len(),
            3
        );
        assert_eq!(fresh.integrity().unwrap(), "ok");
        assert_eq!(
            writer
                .conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let old_manifest: String = writer
            .conn
            .query_row(
                "SELECT manifest_json FROM generation_read_manifest",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&old_manifest).unwrap()["version"],
            2
        );
    }
}

#[test]
fn query_cache_never_crosses_same_generation_ids_roots_or_optional_old_writer_rows() {
    let first = crate::test_support::tempdir().unwrap();
    let second = crate::test_support::tempdir().unwrap();
    let mut a = Store::open(first.path(), true).unwrap();
    let mut b = Store::open(second.path(), true).unwrap();
    let a_lock = a.writer_lock().unwrap();
    let b_lock = b.writer_lock().unwrap();
    a.commit(&fixture("identical"), &a_lock).unwrap();
    b.commit(&fixture("identical"), &b_lock).unwrap();
    let old = Store::open(first.path(), false).unwrap();
    old.read_session().unwrap().query_scope_source().unwrap();
    b.conn
        .execute_batch("DELETE FROM query_symbol_inventory WHERE ordinal=1")
        .unwrap();
    let unrelated = Store::open(second.path(), false).unwrap();
    assert_eq!(
        unrelated
            .read_session()
            .unwrap()
            .query_scope_source()
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(unrelated.query_projection.get().is_none());
    // An old writer can retain the optional tables but publish no rows for its head.
    a.conn
        .execute_batch("DELETE FROM query_symbol_inventory; DELETE FROM generation_query_manifest")
        .unwrap();
    let old_writer = Store::open(first.path(), false).unwrap();
    assert_eq!(
        old_writer
            .read_session()
            .unwrap()
            .query_scoped_candidate_inventory("identical", &Default::default())
            .unwrap()
            .items
            .len(),
        3
    );
    assert_eq!(
        old_writer.query_projection.get(),
        Some(&(MetadataSource::Projection, false))
    );
    assert_eq!(
        old.query_projection.get(),
        Some(&(MetadataSource::Projection, true))
    );
    // A genuine old full validator needs neither optional table and still succeeds.
    assert_eq!(
        old_writer
            .read_session()
            .unwrap()
            .read_projection_source()
            .unwrap(),
        MetadataSource::Projection
    );
    assert_eq!(
        old.read_session()
            .unwrap()
            .query_scoped_candidate_inventory("identical", &Default::default())
            .unwrap()
            .items
            .len(),
        3
    );
    assert!(a.query_projection.get().is_none());
    a.conn.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(a.read_session().err().unwrap().code, "PINNED_READ_REQUIRED");
    a.conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn empty_query_projection_and_explicit_false_diagnostics_are_not_guessed() {
    for empty in [true, false] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        let mut snapshot = fixture("empty");
        snapshot.diagnostics.clear();
        for file in &mut snapshot.files {
            file.diagnostics.clear();
        }
        if empty {
            snapshot.files.clear();
            snapshot.edges.clear();
        }
        writer.commit(&snapshot, &lock).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        assert_eq!(
            session.query_scope_source().unwrap(),
            MetadataSource::Projection
        );
        assert_eq!(
            session
                .query_scoped_candidate_inventory("empty", &Default::default())
                .unwrap()
                .items
                .len(),
            snapshot.symbols().count()
        );
        assert_eq!(
            session.query_file_inventory_snapshot_order().unwrap().len(),
            snapshot.files.len()
        );
        let diagnostics = session.query_parser_diagnostic_presence().unwrap();
        assert_eq!(diagnostics.len(), snapshot.files.len());
        assert!(diagnostics.iter().all(|d| !d.has_parser_diagnostics));
        assert!(reader.projection_source.get().is_none());
    }
}

#[test]
fn query_file_utf8_scalar_types_and_duplicate_identity_evidence_refuse() {
    for mutation in [
        "UPDATE files SET path=CAST(x'ff' AS TEXT)",
        "UPDATE files SET language=CAST(x'ff' AS TEXT)",
        "UPDATE files SET content_hash=CAST(x'ff' AS TEXT)",
        "UPDATE files SET size_bytes=-1",
        "UPDATE files SET size_bytes=0.5",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_file_diagnostics SET has_diagnostics='bad'",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_file_diagnostics SET has_diagnostics=0.5",
        "PRAGMA foreign_keys=OFF; CREATE TABLE replacement AS SELECT * FROM read_file_diagnostics; DROP TABLE read_file_diagnostics; ALTER TABLE replacement RENAME TO read_file_diagnostics; UPDATE read_file_diagnostics SET id=(SELECT id FROM read_file_diagnostics LIMIT 1)",
        "PRAGMA foreign_keys=OFF; CREATE TABLE replacement AS SELECT * FROM query_symbol_inventory; DROP TABLE query_symbol_inventory; ALTER TABLE replacement RENAME TO query_symbol_inventory; UPDATE query_symbol_inventory SET id='symbol0'",
    ] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("file"), &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        assert_eq!(
            session
                .query_file_inventory_snapshot_order()
                .unwrap_err()
                .code,
            "CORRUPT_INDEX",
            "{mutation}"
        );
        assert!(reader.projection_source.get().is_none());
    }
}
