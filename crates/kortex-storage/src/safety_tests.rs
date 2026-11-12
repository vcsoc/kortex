use super::*;
use kortex_core::{Edge, ParsedFile, Span};

fn snapshot(generation: &str, name: &str) -> Snapshot {
    Snapshot {
        repo_id: "repo".into(),
        root: "/sample".into(),
        generation_id: generation.into(),
        files: vec![ParsedFile {
            path: "source.py".into(),
            language: "python".into(),
            content_hash: "hash".into(),
            size_bytes: 20,
            symbols: vec![Symbol {
                symbol_id: "symbol".into(),
                symbol_revision_id: generation.into(),
                repo_id: "repo".into(),
                kind: "function".into(),
                language: "python".into(),
                name: name.into(),
                qualified_name: format!("source.{name}"),
                file_path: "source.py".into(),
                span: Span {
                    start_line: 1,
                    end_line: 2,
                    ..Span::default()
                },
                signature: format!("def {name}():"),
                visibility: "public".into(),
                attributes: serde_json::json!({"source_snippet": "return 42"}),
                content_hash: "hash".into(),
                doc_comment: Some("indexed documentation".into()),
                owner_id: None,
            }],
            references: vec![],
            imports: vec![],
            import_scopes: vec![],
            import_spans: vec![],
            diagnostics: vec![],
        }],
        edges: vec![Edge {
            id: "edge".into(),
            repo_id: "repo".into(),
            source_id: "symbol".into(),
            target_id: Some("symbol".into()),
            kind: "CALLS".into(),
            source_span: Span::default(),
            resolution: "exact".into(),
            confidence: 1.0,
            resolver: "fixture".into(),
            evidence: "recursive call".into(),
            candidates: vec![],
        }],
        ..Snapshot::default()
    }
}

#[test]
fn integrity_detects_logical_fts_graph_and_foreign_key_corruption() {
    for corruption in [
        "DELETE FROM symbol_fts",
        "UPDATE symbol_fts SET symbol_id='orphan'",
        "DELETE FROM edges",
        "PRAGMA foreign_keys=OFF; UPDATE edges SET source_id='missing'",
        "UPDATE generations SET snapshot='{'",
    ] {
        let d = tempfile::tempdir().unwrap();
        let mut store = Store::open(d.path(), true).unwrap();
        let lock = store.writer_lock().unwrap();
        store.commit(&snapshot("good", "oldneedle"), &lock).unwrap();
        assert_eq!(store.integrity().unwrap(), "ok");
        store.conn.execute_batch(corruption).unwrap();
        assert_eq!(
            store.integrity().unwrap_err().code,
            "CORRUPT_INDEX",
            "{corruption}"
        );
    }
}
fn assert_good(store: &Store) {
    let loaded = store.load().unwrap();
    assert_eq!(loaded.generation_id, "good");
    assert_eq!(store.symbol_rows().unwrap(), loaded.files[0].symbols);
    assert_eq!(
        store.keyword("good", "oldneedle", 10).unwrap()[0].0,
        "symbol"
    );
    assert!(store.keyword("good", "newneedle", 10).unwrap().is_empty());
    for table in [
        "generations",
        "files",
        "nodes",
        "edges",
        "symbol_fts",
        "index_state",
        "repositories",
    ] {
        let count: i64 = store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "{table} retained partial state");
    }
    assert_eq!(store.integrity().unwrap(), "ok");
}

#[test]
fn failures_during_files_nodes_edges_and_publication_keep_last_good_generation() {
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&snapshot("good", "oldneedle"), &lock).unwrap();
    for phase in ["files", "nodes", "foreign_key", "edge", "confidence"] {
        let mut bad = snapshot("bad", "newneedle");
        match phase {
            "files" => bad.files.push(bad.files[0].clone()),
            "nodes" => {
                let duplicate = bad.files[0].symbols[0].clone();
                bad.files[0].symbols.push(duplicate);
            }
            "foreign_key" => bad.files[0].symbols[0].repo_id = "missing".into(),
            "edge" => bad.edges[0].target_id = Some("missing".into()),
            "confidence" => bad.edges[0].confidence = 2.0,
            _ => unreachable!(),
        }
        let error = store.commit(&bad, &lock).unwrap_err();
        assert_eq!(error.code, "STORAGE_ERROR", "{phase}");
        assert_good(&store);
    }
    // Fail after publication and FTS deletion, just before old-generation GC.
    store.conn.execute_batch("CREATE TRIGGER fail_gc BEFORE DELETE ON generations BEGIN SELECT RAISE(ABORT,'injected GC failure'); END;").unwrap();
    assert_eq!(
        store
            .commit(&snapshot("bad", "newneedle"), &lock)
            .unwrap_err()
            .code,
        "STORAGE_ERROR"
    );
    assert_good(&store);
    store.conn.execute_batch("DROP TRIGGER fail_gc").unwrap();
    store.commit(&snapshot("next", "newneedle"), &lock).unwrap();
    assert_eq!(store.load().unwrap().generation_id, "next");
    assert!(store.keyword("next", "oldneedle", 10).unwrap().is_empty());
    assert_eq!(
        store.keyword("next", "newneedle", 10).unwrap()[0].0,
        "symbol"
    );
    assert_eq!(store.integrity().unwrap(), "ok");
}

#[test]
fn pinned_reader_keeps_matching_graph_and_fts_through_generation_gc() {
    let d = tempfile::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer
        .commit(&snapshot("good", "oldneedle"), &lock)
        .unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    assert_good(&reader); // This read establishes the WAL snapshot.
    writer
        .commit(&snapshot("next", "newneedle"), &lock)
        .unwrap();
    assert_good(&reader);
    assert!(reader.keyword("next", "newneedle", 10).unwrap().is_empty());
    let fresh = Store::open(d.path(), false).unwrap();
    assert_eq!(fresh.load().unwrap().generation_id, "next");
    assert_eq!(fresh.symbol_rows().unwrap()[0].name, "newneedle");
    assert!(fresh.keyword("good", "oldneedle", 10).unwrap().is_empty());
    assert!(fresh.keyword("next", " \t ", 10).unwrap().is_empty());
    assert!(fresh.keyword("next", "newneedle", 0).unwrap().is_empty());
    assert!(
        fresh
            .keyword("next", "\" OR * NOT (", 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fresh.keyword("next", "newneedle", 1).unwrap().len(), 1);
}

#[test]
fn wrong_repository_lock_and_read_only_commit_do_not_change_index() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut store = Store::open(a.path(), true).unwrap();
    let other = Store::open(b.path(), true).unwrap();
    let other_lock = other.writer_lock().unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&snapshot("good", "oldneedle"), &lock).unwrap();
    assert_eq!(
        store
            .commit(&snapshot("bad", "newneedle"), &other_lock)
            .unwrap_err()
            .code,
        "WRITER_LOCK_MISMATCH"
    );
    assert_good(&store);
    assert_eq!(other.load().unwrap_err().code, "INDEX_NOT_FOUND");
    let mut reader = Store::open(a.path(), false).unwrap();
    assert_eq!(
        reader
            .commit(&snapshot("bad", "newneedle"), &lock)
            .unwrap_err()
            .code,
        "STORAGE_ERROR"
    );
    assert_good(&reader);
    drop(lock);
    assert!(store.writer_lock().is_ok());
}

#[test]
fn future_schema_is_refused_without_rewriting_database() {
    let d = tempfile::tempdir().unwrap();
    let directory = d.path().join(".kortex");
    crate::test_support::directory(&directory).unwrap();
    let db = directory.join("index.db");
    crate::test_support::file(&db, b"").unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("PRAGMA user_version=99; CREATE TABLE future(payload TEXT); INSERT INTO future VALUES('retain this');").unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    for create in [false, true, true] {
        let error = Store::open(d.path(), create)
            .err()
            .expect("future schema accepted");
        assert_eq!(error.code, "INDEX_VERSION");
        assert_eq!(
            fs::read(&db).unwrap(),
            before,
            "refusal rewrote future schema"
        );
    }
}

#[test]
fn missing_schema_and_corrupt_snapshot_have_distinct_diagnostics() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        Store::open(d.path(), false).err().unwrap().code,
        "INDEX_NOT_FOUND"
    );
    assert!(!d.path().join(".kortex").exists());
    crate::test_support::directory(&d.path().join(".kortex")).unwrap();
    let db = d.path().join(".kortex/index.db");
    crate::test_support::file(&db, b"").unwrap();
    drop(Connection::open(&db).unwrap());
    assert_eq!(
        Store::open(d.path(), false).err().unwrap().code,
        "INDEX_VERSION"
    );
    let mut store = Store::open(d.path(), true).unwrap();
    assert_eq!(store.load().unwrap_err().code, "INDEX_NOT_FOUND");
    let lock = store.writer_lock().unwrap();
    store.commit(&snapshot("good", "oldneedle"), &lock).unwrap();
    store
        .conn
        .execute("UPDATE generations SET snapshot='{'", [])
        .unwrap();
    assert_eq!(store.load().unwrap_err().code, "CORRUPT_INDEX");
    assert_eq!(store.symbol_rows().unwrap_err().code, "CORRUPT_INDEX");
    let raw: String = store
        .conn
        .query_row("SELECT snapshot FROM generations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        raw, "{",
        "reading corruption must not attempt destructive repair"
    );
}

#[test]
fn corrupt_database_and_existing_backup_are_never_replaced() {
    let d = tempfile::tempdir().unwrap();
    let directory = d.path().join(".kortex");
    crate::test_support::directory(&directory).unwrap();
    let db = directory.join("index.db");
    let backup = directory.join("index.db.bak");
    crate::test_support::file(&db, b"corrupt original evidence").unwrap();
    crate::test_support::file(&backup, b"previous preserved backup").unwrap();
    #[cfg(unix)]
    let owners = {
        use std::os::unix::fs::MetadataExt;
        (
            fs::metadata(&db).unwrap().uid(),
            fs::metadata(&backup).unwrap().uid(),
        )
    };
    for create in [false, true] {
        assert_eq!(
            Store::open(d.path(), create).err().unwrap().code,
            "STORAGE_ERROR"
        );
        assert_eq!(
            fs::read_to_string(&db).unwrap(),
            "corrupt original evidence"
        );
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "previous preserved backup"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                (
                    fs::metadata(&db).unwrap().uid(),
                    fs::metadata(&backup).unwrap().uid()
                ),
                owners
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn writer_lock_rechecks_symlink_after_store_open() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), true).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "not an owned lock").unwrap();
    let lock = d.path().join(".kortex/writer.lock");
    fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(outside.path(), &lock).unwrap();
    assert_eq!(store.writer_lock().err().unwrap().code, "PATH_REFUSED");
    assert_eq!(
        fs::read_to_string(outside.path()).unwrap(),
        "not an owned lock"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_index_directory_refuses_both_readers_and_writers() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("keep");
    fs::write(&sentinel, "owned elsewhere").unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join(".kortex")).unwrap();
    for create in [false, true] {
        assert_eq!(
            Store::open(d.path(), create).err().unwrap().code,
            "PATH_REFUSED"
        );
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "owned elsewhere");
        assert!(!outside.path().join("index.db").exists());
    }
}
