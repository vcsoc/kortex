use super::*;
use kortex_core::{Edge, ParsedFile, Span};

pub(crate) fn fixture(generation: &str) -> Snapshot {
    let mut s = Snapshot {
        repo_id: "repo".into(),
        root: "/fixed-fixture".into(),
        generation_id: generation.into(),
        git_head: Some("head".into()),
        config_hash: "config".into(),
        grammar_hash: "grammar".into(),
        resolution_hash: "resolution".into(),
        diagnostics: vec!["nonempty generation diagnostic".into()],
        ..Default::default()
    };
    for i in (0..3).rev() {
        let id = format!("symbol{i}");
        s.files.push(ParsedFile {
            path: format!("{i}.py"), language: "python".into(), content_hash: "same".into(), size_bytes: 27,
            symbols: vec![Symbol {
                symbol_id: id.clone(), symbol_revision_id: "revision".into(), repo_id: "repo".into(),
                kind: "Function".into(), language: "python".into(), name: format!("{generation}%_{i}"),
                qualified_name: format!("module{i}.function"), file_path: format!("{i}.py"), span: Span::default(),
                signature: "def function():".into(), visibility: "public".into(),
                attributes: serde_json::json!({"source_snippet": "needle", "number": 1.25, "nested": [true, null]}),
                content_hash: "same".into(), doc_comment: Some("docs".into()), owner_id: None,
            }], references: vec![], imports: vec![], import_scopes: vec![], import_spans: vec![],
            diagnostics: vec![format!("file diagnostic {i}")],
        });
        s.edges.push(Edge {
            id: format!("edge{i}"),
            repo_id: "repo".into(),
            source_id: "symbol0".into(),
            target_id: if i == 2 { None } else { Some("symbol1".into()) },
            kind: "CALLS".into(),
            source_span: Span::default(),
            resolution: if i == 2 { "ambiguous" } else { "exact" }.into(),
            confidence: 0.75,
            resolver: "test".into(),
            evidence: format!("evidence {i}"),
            candidates: if i == 2 {
                vec!["symbol1".into(), "symbol2".into()]
            } else {
                vec![]
            },
        });
    }
    s
}

#[test]
fn pinned_projections_are_complete_bounded_literal_and_generation_coherent() {
    let d = tempfile::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let first = fixture("old");
    writer.commit(&first, &lock).unwrap();
    assert_eq!(
        writer.read_session().err().unwrap().code,
        "PINNED_READ_REQUIRED"
    );
    let reader = Store::open(d.path(), false).unwrap();
    let session = reader.read_session().unwrap();
    assert_eq!(session.metadata(), &SnapshotMetadata::from_snapshot(&first));
    assert_eq!(session.metadata_source(), MetadataSource::Projection);
    let inventory = session.file_inventory().unwrap();
    assert_eq!(
        inventory
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>(),
        ["0.py", "1.py", "2.py"]
    );
    assert_eq!(inventory[0].size_bytes, 27);
    assert_eq!(session.get_symbol("absent").unwrap(), None);
    assert_eq!(
        session.get_symbol("symbol0").unwrap().as_ref(),
        first.symbol("symbol0")
    );
    let filter = SymbolFilter {
        name_prefix: Some("old%_"),
        ..Default::default()
    };
    let page = session.symbol_candidates(&filter, None, 2).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_after_id.as_deref(), Some("symbol1"));
    let rest = session
        .symbol_candidates(&filter, page.next_after_id.as_deref(), 2)
        .unwrap();
    assert_eq!(rest.items[0].symbol_id, "symbol2");
    assert!(rest.next_after_id.is_none());
    for filter in [
        SymbolFilter {
            name: Some("OLD%_0"),
            ..Default::default()
        },
        SymbolFilter {
            name_prefix: Some("old%X"),
            ..Default::default()
        },
        SymbolFilter {
            file_path: Some("absent"),
            ..Default::default()
        },
    ] {
        assert!(
            session
                .symbol_candidates(&filter, None, 10)
                .unwrap()
                .items
                .is_empty()
        );
    }
    let exact = SymbolFilter {
        name: Some("old%_1"),
        qualified_name: Some("module1.function"),
        name_prefix: Some("old"),
        file_path: Some("1.py"),
        kind: Some("Function"),
    };
    assert_eq!(
        session.symbol_candidates(&exact, None, 1).unwrap().items[0].symbol_id,
        "symbol1"
    );
    let incoming = session.incoming_edges("symbol1", None, 1).unwrap();
    assert_eq!(incoming.items[0].id, "edge0");
    assert_eq!(incoming.next_after_id.as_deref(), Some("edge0"));
    assert_eq!(
        session
            .incoming_edges("symbol1", Some("edge0"), 1)
            .unwrap()
            .items[0]
            .id,
        "edge1"
    );
    assert!(
        session
            .incoming_edges("symbol2", None, 1000)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        session
            .outgoing_edges("symbol0", None, 1000)
            .unwrap()
            .items
            .len(),
        3
    );
    for bad in [0, 1001, usize::MAX] {
        assert!(
            session
                .symbol_candidates(&Default::default(), None, bad)
                .is_err()
        );
        assert!(session.outgoing_edges("symbol0", None, bad).is_err());
        assert!(session.incoming_edges("symbol1", None, bad).is_err());
        assert!(session.keyword("needle", bad).is_err());
    }
    writer.commit(&fixture("new"), &lock).unwrap();
    assert_eq!(session.metadata().generation_id, "old");
    assert_eq!(
        session.get_symbol("symbol0").unwrap().unwrap().name,
        "old%_0"
    );
    assert_eq!(session.keyword("old", 10).unwrap().len(), 3);
    assert!(session.keyword("new", 10).unwrap().is_empty());
    assert_eq!(
        serde_json::to_value(session.load().unwrap()).unwrap(),
        serde_json::to_value(first).unwrap()
    );
    let fresh = Store::open(d.path(), false).unwrap();
    let fresh = fresh.read_session().unwrap();
    assert_eq!(fresh.metadata().generation_id, "new");
    assert_eq!(fresh.keyword("new", 10).unwrap().len(), 3);
}

#[test]
fn optional_metadata_never_invents_diagnostics_or_hides_corruption() {
    let d = tempfile::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let first = fixture("old");
    writer.commit(&first, &lock).unwrap();
    writer
        .conn
        .execute_batch("DROP TABLE generation_metadata")
        .unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    let session = reader.read_session().unwrap();
    assert_eq!(session.metadata_source(), MetadataSource::LegacySnapshot);
    assert_eq!(session.metadata(), &SnapshotMetadata::from_snapshot(&first));
    writer.commit(&fixture("new"), &lock).unwrap();
    // Projection reads must not touch the large duplicate JSON blobs.
    writer
        .conn
        .execute_batch(
            "UPDATE generations SET snapshot='invalid'; UPDATE files SET facts_json='invalid'",
        )
        .unwrap();
    let projected = Store::open(d.path(), false).unwrap();
    let projected = projected.read_session().unwrap();
    assert_eq!(projected.metadata_source(), MetadataSource::Projection);
    assert_eq!(projected.file_inventory().unwrap().len(), 3);
    assert!(projected.get_symbol("symbol0").unwrap().is_some());
    assert_eq!(projected.load().unwrap_err().code, "CORRUPT_INDEX");
    writer
        .conn
        .execute_batch("UPDATE generation_metadata SET metadata_json='{}'")
        .unwrap();
    let corrupt = Store::open(d.path(), false).unwrap();
    assert_eq!(corrupt.read_session().err().unwrap().code, "CORRUPT_INDEX");
    let version: i64 = writer
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
    // An older pinned reader still has the old schema/metadata fallback and complete facts.
    assert_eq!(session.metadata_source(), MetadataSource::LegacySnapshot);
    assert_eq!(session.load().unwrap().generation_id, "old");
}

#[test]
fn lightweight_inventory_order_and_per_file_partial_survive_invalid_full_json() {
    let d = tempfile::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let mut first = fixture("old");
    first.diagnostics.clear();
    first.files[0].diagnostics.clear();
    for (i, f) in first.files.iter_mut().enumerate() {
        let s = &mut f.symbols[0];
        s.name = ["İTEM", "Éclair", "Σcope"][i].into();
        s.qualified_name = format!("MÓDULO.{}", s.name);
        s.span.start_byte = 19 + i;
    }
    writer.commit(&first, &lock).unwrap();
    writer.conn.execute_batch("UPDATE generations SET snapshot='invalid'; UPDATE files SET facts_json='invalid'; UPDATE nodes SET data_json='invalid'").unwrap();
    let pinned = Store::open(d.path(), false).unwrap();
    let session = pinned.read_session().unwrap();
    assert_eq!(
        session.read_projection_source().unwrap(),
        MetadataSource::Projection
    );
    assert!(session.metadata().diagnostics.is_empty());
    let inventory = session.symbol_inventory().unwrap();
    assert_eq!(
        session
            .file_inventory_snapshot_order()
            .unwrap()
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["2.py", "1.py", "0.py"]
    );
    assert_eq!(
        inventory
            .iter()
            .map(|s| s.symbol_id.as_str())
            .collect::<Vec<_>>(),
        ["symbol2", "symbol1", "symbol0"]
    );
    for (row, actual) in inventory.iter().zip(first.symbols()) {
        assert_eq!(
            (
                &row.name,
                &row.qualified_name,
                &row.file_path,
                &row.language,
                &row.kind,
                row.start_byte
            ),
            (
                &actual.name,
                &actual.qualified_name,
                &actual.file_path,
                &actual.language,
                &actual.kind,
                actual.span.start_byte
            )
        );
    }
    // Actual Rust Unicode matching BEFORE pagination: SQLite lower would miss these.
    for query in ["i\u{307}t", "écl", "σc", "módulo"] {
        let ids: Vec<_> = inventory
            .iter()
            .filter(|s| {
                s.name.to_lowercase().starts_with(query)
                    || s.qualified_name.to_lowercase().contains(query)
                    || s.file_path.to_lowercase().contains(query)
            })
            .map(|s| &s.symbol_id)
            .collect();
        let expected: Vec<_> = first
            .symbols()
            .filter(|s| {
                s.name.to_lowercase().starts_with(query)
                    || s.qualified_name.to_lowercase().contains(query)
                    || s.file_path.to_lowercase().contains(query)
            })
            .map(|s| &s.symbol_id)
            .collect();
        assert_eq!(ids, expected);
        assert!(!ids.is_empty());
    }
    let diagnostics = session.parser_diagnostic_presence().unwrap();
    assert_eq!(
        diagnostics
            .iter()
            .map(|f| (f.path.as_str(), f.has_parser_diagnostics))
            .collect::<Vec<_>>(),
        [("0.py", true), ("1.py", true), ("2.py", false)]
    );
    let page = session.outgoing_edges_ordered("symbol0", None, 1).unwrap();
    assert_eq!(
        (
            page.items[0].ordinal,
            page.items[0].edge.id.as_str(),
            page.next_after_ordinal
        ),
        (0, "edge2", Some(0))
    );
    let page = session
        .outgoing_edges_ordered("symbol0", page.next_after_ordinal, 2)
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|e| (e.ordinal, e.edge.id.as_str()))
            .collect::<Vec<_>>(),
        [(1, "edge1"), (2, "edge0")]
    );
    assert!(page.next_after_ordinal.is_none());
    let incoming = session.incoming_edges_ordered("symbol1", None, 1).unwrap();
    assert_eq!(
        (
            incoming.items[0].ordinal,
            incoming.items[0].edge.id.as_str(),
            incoming.next_after_ordinal
        ),
        (1, "edge1", Some(1))
    );
    assert!(
        session
            .incoming_edges_ordered("symbol2", None, 1)
            .unwrap()
            .items
            .is_empty()
    );
    for bad in [0, 1001, usize::MAX] {
        assert!(
            session
                .incoming_edges_ordered("symbol1", None, bad)
                .is_err()
        );
        assert!(
            session
                .outgoing_edges_ordered("symbol0", None, bad)
                .is_err()
        );
    }
    assert!(session.get_symbol("symbol0").is_err());
    assert!(session.load().is_err());
    writer.commit(&fixture("new"), &lock).unwrap();
    assert_eq!(session.symbol_inventory().unwrap(), inventory);
    assert_eq!(session.parser_diagnostic_presence().unwrap(), diagnostics);
    assert_eq!(
        session
            .outgoing_edges_ordered("symbol0", None, 3)
            .unwrap()
            .items[0]
            .edge,
        first.edges[0]
    );
    let fresh = Store::open(d.path(), false).unwrap();
    assert_eq!(
        fresh.read_session().unwrap().symbol_inventory().unwrap()[0].name,
        "new%_2"
    );
}

#[test]
fn optional_new_evidence_falls_back_truthfully_but_present_corruption_refuses() {
    for mutation in [
        "DELETE FROM read_symbol_inventory WHERE id='symbol1'",
        "DELETE FROM read_edge_order WHERE id='edge1'",
        "DELETE FROM read_file_diagnostics WHERE id=(SELECT id FROM files LIMIT 1)",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_file_diagnostics SET has_diagnostics=2",
        "UPDATE generation_read_manifest SET manifest_json='{}'",
        "DELETE FROM generation_read_manifest",
        "DROP TABLE generation_read_manifest",
        "DROP TABLE read_file_diagnostics",
        "DROP TABLE read_file_order",
        "DELETE FROM read_file_order WHERE ordinal=1",
        "UPDATE read_file_order SET ordinal=90 WHERE ordinal=1",
        "UPDATE read_edge_order SET ordinal=90 WHERE id='edge1'",
        "PRAGMA ignore_check_constraints=ON; UPDATE read_symbol_inventory SET start_byte=-1",
    ] {
        let d = tempfile::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        assert_eq!(
            session.symbol_inventory().unwrap_err().code,
            "CORRUPT_INDEX",
            "{mutation}"
        );
        assert_eq!(
            session.parser_diagnostic_presence().unwrap_err().code,
            "CORRUPT_INDEX"
        );
        assert_eq!(
            session
                .outgoing_edges_ordered("symbol0", None, 10)
                .unwrap_err()
                .code,
            "CORRUPT_INDEX"
        );
    }
    for mutation in [
        "DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_edge_order; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order",
        "DELETE FROM generation_read_manifest; DELETE FROM read_symbol_inventory; DELETE FROM read_edge_order; DELETE FROM read_file_diagnostics; DELETE FROM read_file_order",
    ] {
        let d = tempfile::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        let first = fixture("old");
        writer.commit(&first, &lock).unwrap();
        writer.conn.execute_batch(mutation).unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        assert_eq!(session.metadata_source(), MetadataSource::Projection);
        assert_eq!(
            session.read_projection_source().unwrap(),
            MetadataSource::LegacySnapshot
        );
        assert_eq!(session.symbol_inventory().unwrap().len(), 3);
        assert!(
            session
                .parser_diagnostic_presence()
                .unwrap()
                .iter()
                .all(|f| f.has_parser_diagnostics)
        );
        assert_eq!(
            session
                .outgoing_edges_ordered("symbol0", None, 3)
                .unwrap()
                .items
                .iter()
                .map(|e| &e.edge)
                .collect::<Vec<_>>(),
            first.edges.iter().collect::<Vec<_>>()
        );
        writer
            .conn
            .execute_batch("UPDATE generations SET snapshot='invalid'")
            .unwrap();
        let corrupt = Store::open(d.path(), false).unwrap();
        assert!(
            corrupt
                .read_session()
                .unwrap()
                .parser_diagnostic_presence()
                .is_err()
        );
        assert_eq!(session.load().unwrap().generation_id, "old");
        let version: i64 = writer
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }
}

#[test]
fn new_projection_failure_is_atomic_and_retry_preserves_pinned_evidence() {
    for table in [
        "generation_read_manifest",
        "read_symbol_inventory",
        "read_edge_order",
        "read_file_diagnostics",
        "read_file_order",
    ] {
        let d = tempfile::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        let pinned = Store::open(d.path(), false).unwrap();
        let old = pinned.read_session().unwrap();
        let inventory = old.symbol_inventory().unwrap();
        writer.conn.execute_batch(&format!("CREATE TRIGGER fail_read_projection BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'projection failure'); END;")).unwrap();
        assert!(writer.commit(&fixture("new"), &lock).is_err());
        assert_eq!(writer.load().unwrap().generation_id, "old");
        writer
            .conn
            .execute_batch("DROP TRIGGER fail_read_projection")
            .unwrap();
        writer.commit(&fixture("new"), &lock).unwrap();
        assert_eq!(old.symbol_inventory().unwrap(), inventory);
        let reader = Store::open(d.path(), false).unwrap();
        assert_eq!(
            reader.read_session().unwrap().symbol_inventory().unwrap()[0].name,
            "new%_2"
        );
        assert_eq!(writer.integrity().unwrap(), "ok");
    }
}

#[test]
fn earlier_projection_without_file_order_reports_legacy_before_full_materialization() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture("old"), &lock).unwrap();
    writer.conn.execute_batch("UPDATE generation_read_manifest SET manifest_json=replace(manifest_json,'\"version\":2','\"version\":1'); DROP TABLE read_file_order").unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    let old = reader.read_session().unwrap();
    assert_eq!(
        old.read_projection_source().unwrap(),
        MetadataSource::LegacySnapshot
    );
    assert_eq!(
        old.file_inventory_snapshot_order()
            .unwrap()
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["2.py", "1.py", "0.py"]
    );
    writer
        .conn
        .execute_batch("UPDATE generations SET snapshot='invalid'")
        .unwrap();
    let corrupt = Store::open(d.path(), false).unwrap();
    let session = corrupt.read_session().unwrap();
    // The caller sees missing order BEFORE deciding to fall back; no hidden load here.
    assert_eq!(
        session.read_projection_source().unwrap(),
        MetadataSource::LegacySnapshot
    );
    assert!(session.file_inventory_snapshot_order().is_err());
    writer.commit(&fixture("new"), &lock).unwrap();
    let fresh = Store::open(d.path(), false).unwrap();
    assert_eq!(
        fresh
            .read_session()
            .unwrap()
            .read_projection_source()
            .unwrap(),
        MetadataSource::Projection
    );
    assert_eq!(old.file_inventory_snapshot_order().unwrap().len(), 3);
}

#[test]
fn ordered_adjacency_crosses_501_context_boundary_without_tie_reordering() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let mut s = fixture("many");
    s.edges = (0..1201)
        .map(|i| {
            let mut edge = s.edges[i % 3].clone();
            edge.id = format!("edge-{:05}", 1200 - i);
            if i % 7 == 0 {
                edge.source_id = "symbol2".into()
            }
            edge
        })
        .collect();
    writer.commit(&s, &lock).unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    let session = reader.read_session().unwrap();
    for incoming in [false, true] {
        for limit in [1, 500, 501, 1000] {
            let id = if incoming { "symbol1" } else { "symbol0" };
            let expected: Vec<_> = s
                .edges
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    if incoming {
                        e.target_id.as_deref() == Some(id)
                    } else {
                        e.source_id == id
                    }
                })
                .collect();
            let mut after = None;
            let mut actual = Vec::new();
            loop {
                let page = if incoming {
                    session.incoming_edges_ordered(id, after, limit)
                } else {
                    session.outgoing_edges_ordered(id, after, limit)
                }
                .unwrap();
                assert!(page.items.len() <= limit);
                actual.extend(page.items.into_iter().map(|e| (e.ordinal, e.edge)));
                after = page.next_after_ordinal;
                if after.is_none() {
                    break;
                }
            }
            assert_eq!(actual.len(), expected.len());
            for ((ordinal, edge), (expected_ordinal, expected_edge)) in actual.iter().zip(expected)
            {
                assert_eq!(*ordinal, expected_ordinal);
                assert_eq!(
                    serde_json::to_vec(edge).unwrap(),
                    serde_json::to_vec(expected_edge).unwrap()
                );
            }
        }
    }
    let empty = Snapshot {
        repo_id: s.repo_id.clone(),
        root: s.root.clone(),
        generation_id: "empty".into(),
        ..Default::default()
    };
    writer.commit(&empty, &lock).unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    let empty = reader.read_session().unwrap();
    assert_eq!(
        empty.read_projection_source().unwrap(),
        MetadataSource::Projection
    );
    assert!(empty.symbol_inventory().unwrap().is_empty());
    assert!(empty.parser_diagnostic_presence().unwrap().is_empty());
    assert!(
        empty
            .outgoing_edges_ordered("symbol0", None, 500)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(session.symbol_inventory().unwrap().len(), 3);
}

#[test]
fn metadata_projection_failure_rolls_back_and_retry_succeeds() {
    let d = tempfile::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture("old"), &lock).unwrap();
    writer.conn.execute_batch("CREATE TRIGGER fail_metadata BEFORE INSERT ON generation_metadata BEGIN SELECT RAISE(ABORT,'injected metadata failure'); END;").unwrap();
    assert!(writer.commit(&fixture("new"), &lock).is_err());
    let reader = Store::open(d.path(), false).unwrap();
    assert_eq!(
        reader.read_session().unwrap().metadata().generation_id,
        "old"
    );
    writer
        .conn
        .execute_batch("DROP TRIGGER fail_metadata")
        .unwrap();
    writer.commit(&fixture("new"), &lock).unwrap();
    assert_eq!(writer.load().unwrap().generation_id, "new");
}
