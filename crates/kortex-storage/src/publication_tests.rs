//! Real publication rollback/concurrent-reader regressions, including child process death.
use super::*;
use kortex_core::{Edge, ParsedFile, Span};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn checkpoint(phase: &str) {
    if std::env::var("KORTEX_PUBLICATION_TEST_PHASE").as_deref() == Ok(phase) {
        let root = std::env::var_os("KORTEX_PUBLICATION_TEST_ROOT").unwrap();
        fs::write(Path::new(&root).join("publication-ready"), phase).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        panic!("parent did not kill publication child");
    }
}
fn fixture(generation: &str, name: &str) -> Snapshot {
    let mut snapshot = Snapshot {
        repo_id: "repo".into(),
        root: "/fixture".into(),
        generation_id: generation.into(),
        ..Default::default()
    };
    for i in 0..2 {
        snapshot.files.push(ParsedFile {
            path: format!("{i}.py"),
            language: "python".into(),
            content_hash: "deliberately-unchanged".into(),
            size_bytes: 50,
            symbols: vec![Symbol {
                symbol_id: format!("symbol{i}"),
                symbol_revision_id: format!("revision{i}"),
                repo_id: "repo".into(),
                kind: "Function".into(),
                language: "python".into(),
                name: name.into(),
                qualified_name: format!("{i}.{name}"),
                file_path: format!("{i}.py"),
                span: Span::default(),
                signature: format!("def {name}():"),
                visibility: "public".into(),
                attributes: serde_json::json!({"source_snippet": "body"}),
                content_hash: "same-hash".into(),
                doc_comment: None,
                owner_id: None,
            }],
            references: vec![],
            imports: vec![],
            import_scopes: vec![],
            import_spans: vec![],
            diagnostics: vec![],
        });
        snapshot.edges.push(Edge {
            id: format!("edge{i}"),
            repo_id: "repo".into(),
            source_id: format!("symbol{i}"),
            target_id: Some(format!("symbol{}", 1 - i)),
            kind: "CALLS".into(),
            source_span: Span::default(),
            resolution: "exact".into(),
            confidence: 1.0,
            resolver: "fixture".into(),
            evidence: format!("evidence-{name}"),
            candidates: vec![],
        });
    }
    snapshot
}
fn assert_snapshot(store: &Store, expected: &Snapshot) {
    let loaded = store.load().unwrap();
    assert_eq!(loaded.generation_id, expected.generation_id);
    assert_eq!(loaded.files, expected.files);
    assert_eq!(loaded.edges, expected.edges);
    assert_eq!(
        store
            .keyword(
                &expected.generation_id,
                &expected.files[0].symbols[0].name,
                10
            )
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.integrity().unwrap(), "ok");
    let foreign_keys: i64 = store
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    for (name, table, expected) in [
        (
            "idx_nodes_lookup",
            "nodes",
            vec!["generation_id", "kind", "name"],
        ),
        (
            "idx_nodes_file",
            "nodes",
            vec!["generation_id", "file_path"],
        ),
        (
            "idx_nodes_qualified",
            "nodes",
            vec!["generation_id", "qualified_name"],
        ),
        (
            "idx_edges_source",
            "edges",
            vec!["generation_id", "source_id", "kind"],
        ),
        (
            "idx_edges_target",
            "edges",
            vec!["generation_id", "target_id", "kind"],
        ),
    ] {
        let mut stmt = store
            .conn
            .prepare(&format!("PRAGMA index_info({name})"))
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |r| r.get(2))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(columns, expected, "lookup index not restored: {name}");
        store
            .conn
            .prepare(&format!("SELECT 1 FROM {table} INDEXED BY {name} LIMIT 1"))
            .unwrap();
    }
}

#[test]
fn each_child_cleanup_failure_rolls_back_all_graph_fts_and_snapshot_rows() {
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    let old = fixture("old", "oldneedle");
    store.commit(&old, &lock).unwrap();
    for table in ["edges", "nodes", "files", "generations"] {
        store.conn.execute_batch(&format!("CREATE TRIGGER fail_retirement BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT,'injected retirement failure'); END;")).unwrap();
        assert!(store.commit(&fixture("new", "newneedle"), &lock).is_err());
        assert_snapshot(&store, &old);
        store
            .conn
            .execute_batch("DROP TRIGGER fail_retirement")
            .unwrap();
    }
    // Same content/revision hashes cannot suppress a changed fact or globally resolved edge.
    let mut next = fixture("new", "newneedle");
    next.edges[0].target_id = None;
    next.edges[0].resolution = "unresolved".into();
    let timings = store.commit_profiled(&next, &lock).unwrap();
    assert_snapshot(&store, &next);
    assert!(timings.durable_commit > Duration::ZERO);
    assert!(store.keyword("new", "oldneedle", 10).unwrap().is_empty());
    // Duplicate IDs must still fail, even when every field is otherwise unchanged.
    let mut duplicate = next.clone();
    duplicate.generation_id = "duplicate".into();
    duplicate.edges.push(duplicate.edges[0].clone());
    assert!(store.commit(&duplicate, &lock).is_err());
    assert_snapshot(&store, &next);
}

#[test]
fn fts_identity_verification_rejects_null_and_duplicate_identity_with_equal_counts() {
    for value in [None, Some("symbol0")] {
        let d = tempfile::tempdir().unwrap();
        let mut store = Store::open(d.path(), true).unwrap();
        let lock = store.writer_lock().unwrap();
        store.commit(&fixture("good", "needle"), &lock).unwrap();
        store
            .conn
            .execute(
                "UPDATE symbol_fts SET symbol_id=?1 WHERE symbol_id='symbol1'",
                [value],
            )
            .unwrap();
        assert_eq!(store.integrity().unwrap_err().code, "CORRUPT_INDEX");
    }
}

#[test]
fn publication_crash_child() {
    let Some(root) = std::env::var_os("KORTEX_PUBLICATION_TEST_ROOT") else {
        return;
    };
    let mut store = Store::open(Path::new(&root), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&fixture("new", "newneedle"), &lock).unwrap();
    panic!("publication did not enter requested checkpoint");
}

#[test]
fn real_commit_death_before_and_after_pointer_switch_keeps_readers_and_restart_atomic() {
    for phase in ["retired", "before_indexes", "published"] {
        let d = tempfile::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        let old = fixture("old", "oldneedle");
        writer.commit(&old, &lock).unwrap();
        drop(lock);
        drop(writer);
        let pinned = Store::open(d.path(), false).unwrap();
        assert_snapshot(&pinned, &old);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "publication_tests::publication_crash_child",
                "--nocapture",
            ])
            .env("KORTEX_PUBLICATION_TEST_ROOT", d.path())
            .env("KORTEX_PUBLICATION_TEST_PHASE", phase)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !d.path().join("publication-ready").exists() {
            if Instant::now() > deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("publication child did not reach {phase}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Both a pre-existing reader and a newly started reader see exactly the old
        // graph and FTS even after private child cleanup/pointer switch has occurred.
        assert_snapshot(&pinned, &old);
        let concurrent = Store::open(d.path(), false).unwrap();
        assert_snapshot(&concurrent, &old);
        assert_eq!(concurrent.writer_lock().err().unwrap().code, "WRITER_BUSY");
        child.kill().unwrap();
        child.wait().unwrap();
        let mut restarted = Store::open(d.path(), true).unwrap();
        assert_snapshot(&restarted, &old);
        let lock = restarted.writer_lock().unwrap();
        let next = fixture("new", "newneedle");
        restarted.commit(&next, &lock).unwrap();
        assert_snapshot(&restarted, &next);
        assert_snapshot(&pinned, &old);
        assert_snapshot(&concurrent, &old);
        assert_snapshot(&Store::open(d.path(), false).unwrap(), &next);
    }
}
