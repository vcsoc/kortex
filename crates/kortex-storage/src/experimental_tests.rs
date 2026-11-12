use super::*;
use crate::read_tests::fixture;
use std::{
    fs,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn private_parent() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    d
}
fn reader(path: &Path) -> DifferentialStore {
    DifferentialStore::open_scratch(path, false).unwrap()
}
fn assert_exact(store: &DifferentialStore, expected: &Snapshot) {
    assert_eq!(
        serde_json::to_vec(&store.load().unwrap()).unwrap(),
        serde_json::to_vec(expected).unwrap()
    );
    let physical: String = store
        .conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(physical, "ok");
    assert!(
        !store
            .conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let foreign_keys: i64 = store
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
}
fn mutation() -> Snapshot {
    let mut s = fixture("new");
    // Changes global semantic facts despite deliberately unchanged hashes.
    s.edges[0].resolution = "unresolved".into();
    s.edges[0].candidates.clear();
    s.edges[0].evidence = "new evidence independent of content/revision hashes".into();
    s.edges[1].target_id = Some("symbol2".into());
    s.files[0]
        .diagnostics
        .push("another exact diagnostic".into());
    s
}
#[test]
fn profiled_new_node_fts_absence_is_based_on_actual_rows_including_orphans() {
    let d = crate::test_support::tempdir().unwrap();
    let path = d.path().join("scratch");
    let mut writer = DifferentialStore::create_scratch(&path).unwrap();
    let lock = writer.writer_lock().unwrap();
    let old = fixture("old");
    writer.publish(&old, None, &lock).unwrap();
    let pinned = reader(&path);
    let old_fts = pinned.keyword_fingerprint().unwrap();
    let mut next = old.clone();
    next.generation_id = "new-node".into();
    let mut node = next.files[0].symbols[0].clone();
    node.symbol_id = "extra".into();
    node.name = "newneedle".into();
    next.files[0].symbols.push(node);
    // A new node ID does NOT prove its FTS identity is absent. The complete actual
    // FTS corpus, sealed into this baseline, includes this intentionally orphan row.
    writer.conn.execute("INSERT INTO symbol_fts(symbol_id,name,qualified_name,file_path,text) VALUES('extra','orphanneedle','orphan','orphan.py','orphan text')",[]).unwrap();
    let current = reader(&path);
    let baseline = current.baseline().unwrap();
    assert_ne!(current.keyword_fingerprint().unwrap(), old_fts);
    let (changes, timings) = writer
        .publish_profiled(&next, Some(&baseline), &lock)
        .unwrap();
    assert_eq!(
        (
            changes.nodes_written,
            changes.fts_written,
            changes.fts_deleted
        ),
        (1, 1, 1)
    );
    assert!(timings.actual_fingerprints > Duration::ZERO);
    assert_exact(&reader(&path), &next);
    let fresh = reader(&path);
    assert!(fresh.keyword("orphanneedle", 10).unwrap().is_empty());
    assert_eq!(fresh.keyword("newneedle", 10).unwrap().len(), 1);
    assert_eq!(pinned.keyword_fingerprint().unwrap(), old_fts);
    // Compare ALL actual FTS values with schema1, not merely selected query hits.
    let v1 = d.path().join("v1");
    crate::test_support::directory(&v1).unwrap();
    let mut store = crate::Store::open(&v1, true).unwrap();
    let v1lock = store.writer_lock().unwrap();
    store.commit(&next, &v1lock).unwrap();
    let v1reader = crate::Store::open(&v1, false).unwrap();
    assert_eq!(
        fresh.keyword_fingerprint().unwrap(),
        v1reader
            .read_session()
            .unwrap()
            .keyword_fingerprint()
            .unwrap()
    );
    writer
        .conn
        .execute(
            "UPDATE symbol_fts SET text='unqueried text corruption' WHERE symbol_id='extra'",
            [],
        )
        .unwrap();
    assert_ne!(
        reader(&path).keyword_fingerprint().unwrap(),
        fresh.keyword_fingerprint().unwrap()
    );
    writer
        .conn
        .execute(
            "INSERT INTO symbol_fts SELECT * FROM symbol_fts WHERE symbol_id='extra'",
            [],
        )
        .unwrap();
    assert_eq!(
        reader(&path).keyword_fingerprint().unwrap_err().code,
        "CORRUPT_INDEX"
    );
}

#[test]
fn isolated_store_preserves_all_facts_order_and_only_mutates_differences() {
    let d = private_parent();
    let path = d.path().join("scratch");
    let mut writer = DifferentialStore::create_scratch(&path).unwrap();
    assert!(DifferentialStore::create_scratch(&path).is_err());
    assert!(crate::Store::open(&path, false).is_err());
    let lock = writer.writer_lock().unwrap();
    let old = fixture("old");
    let first = writer.publish(&old, None, &lock).unwrap();
    assert_eq!(
        (
            first.files_written,
            first.nodes_written,
            first.edges_written
        ),
        (3, 3, 3)
    );
    let pinned = reader(&path);
    assert_exact(&pinned, &old);
    let (mut cached, baseline) = pinned.baseline().unwrap().split_cached_files();
    assert_eq!(cached, old.files);
    cached[0].symbols[0].name =
        "caller can mutate owned cache without forging comparison evidence".into();
    let mut unchanged = old.clone();
    unchanged.generation_id = "same-facts".into();
    assert_eq!(
        writer.publish(&unchanged, Some(&baseline), &lock).unwrap(),
        DifferentialChanges::default()
    );
    let current = reader(&path);
    assert_exact(&current, &unchanged);
    assert_eq!(
        writer
            .publish(&mutation(), Some(&baseline), &lock)
            .unwrap_err()
            .code,
        "BASELINE_REFUSED"
    );
    let baseline = current.baseline().unwrap();
    let mut reordered = unchanged.clone();
    reordered.generation_id = "reordered".into();
    reordered.files.reverse();
    reordered.edges.rotate_left(1);
    assert_eq!(
        writer.publish(&reordered, Some(&baseline), &lock).unwrap(),
        DifferentialChanges::default()
    );
    let current = reader(&path);
    assert_exact(&current, &reordered);
    let baseline = current.baseline().unwrap();
    let next = mutation();
    let changes = writer.publish(&next, Some(&baseline), &lock).unwrap();
    assert_eq!(
        (
            changes.files_written,
            changes.nodes_written,
            changes.edges_written
        ),
        (1, 3, 2)
    );
    assert_eq!(
        (
            changes.files_deleted,
            changes.nodes_deleted,
            changes.edges_deleted
        ),
        (0, 0, 0)
    );
    assert_eq!((changes.fts_written, changes.fts_deleted), (3, 3));
    let fresh = reader(&path);
    assert_exact(&fresh, &next);
    assert_eq!(fresh.keyword("new", 10).unwrap().len(), 3);
    assert!(fresh.keyword("old", 10).unwrap().is_empty());
    assert_exact(&pinned, &old);
    assert_eq!(pinned.keyword("old", 10).unwrap().len(), 3);
    assert!(pinned.keyword("new", 10).unwrap().is_empty());
    let synchronous: i64 = writer
        .conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(synchronous, 2);
    // No primary row or index generation retag/rebuild: only headers/layout have generations.
    for table in ["files", "nodes", "edges"] {
        let mut stmt = writer
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(1))
            .unwrap()
            .map(|v| v.unwrap())
            .collect();
        assert!(!names.iter().any(|n| n.contains("generation")));
    }
}
#[test]
fn duplicate_dangling_foreign_baselines_and_transaction_failures_are_atomic() {
    let d = private_parent();
    let path = d.path().join("scratch");
    let mut writer = DifferentialStore::create_scratch(&path).unwrap();
    let lock = writer.writer_lock().unwrap();
    let old = fixture("old");
    writer.publish(&old, None, &lock).unwrap();
    let pinned = reader(&path);
    let baseline = pinned.baseline().unwrap();
    let other_path = d.path().join("other");
    let mut other = DifferentialStore::create_scratch(&other_path).unwrap();
    let other_lock = other.writer_lock().unwrap();
    other.publish(&old, None, &other_lock).unwrap();
    let other_reader = reader(&other_path);
    let foreign = other_reader.baseline().unwrap();
    assert_eq!(
        writer
            .publish(&mutation(), Some(&foreign), &lock)
            .unwrap_err()
            .code,
        "BASELINE_REFUSED"
    );
    assert_eq!(
        writer.publish(&mutation(), None, &lock).unwrap_err().code,
        "BASELINE_REFUSED"
    );
    for case in 0..8 {
        let mut invalid = mutation();
        match case {
            0 => invalid.files.push(invalid.files[0].clone()),
            1 => {
                let duplicate = invalid.files[0].symbols[0].clone();
                invalid.files[0].symbols.push(duplicate);
            }
            2 => invalid.edges.push(invalid.edges[0].clone()),
            3 => invalid.edges[0].source_id = "missing".into(),
            4 => invalid.edges[0].target_id = Some("missing".into()),
            5 => invalid.edges[0].candidates.push("missing".into()),
            6 => invalid.files[0].symbols[0].owner_id = Some("missing".into()),
            _ => invalid.edges[0].confidence = f64::NAN,
        }
        assert!(writer.publish(&invalid, Some(&baseline), &lock).is_err());
        assert_exact(&reader(&path), &old);
    }
    for table in ["files", "nodes", "edges", "state"] {
        writer.conn.execute_batch(&format!("CREATE TRIGGER fail_mutation BEFORE UPDATE ON {table} BEGIN SELECT RAISE(ABORT,'injected mutation failure'); END")).unwrap();
        assert!(writer.publish(&mutation(), Some(&baseline), &lock).is_err());
        assert_exact(&reader(&path), &old);
        writer
            .conn
            .execute_batch("DROP TRIGGER fail_mutation")
            .unwrap();
    }
    let mut deleted = old.clone();
    deleted.generation_id = "deleted".into();
    deleted.edges.clear();
    deleted.files.retain(|f| f.path == "0.py");
    let changes = writer.publish(&deleted, Some(&baseline), &lock).unwrap();
    assert_eq!(
        (
            changes.files_deleted,
            changes.nodes_deleted,
            changes.edges_deleted
        ),
        (2, 2, 3)
    );
    assert_exact(&reader(&path), &deleted);
    assert_exact(&pinned, &old);
}
#[test]
fn baseline_rechecks_actual_rows_instead_of_trusting_stored_generation_label() {
    for sql in [
        "UPDATE edges SET data_json=replace(data_json,'evidence 0','tampered evidence') WHERE id='edge0'",
        "UPDATE edges SET target_id='symbol2' WHERE id='edge0'",
        "UPDATE nodes SET file_id=(SELECT file_id FROM nodes WHERE id='symbol1') WHERE id='symbol0'",
        "UPDATE symbol_fts SET text='tampered FTS' WHERE symbol_id='symbol0'",
        "UPDATE generations SET header_json=replace(header_json,'nonempty generation diagnostic','tampered diagnostic')",
    ] {
        let d = private_parent();
        let path = d.path().join("scratch");
        let mut writer = DifferentialStore::create_scratch(&path).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.publish(&fixture("old"), None, &lock).unwrap();
        let pinned = reader(&path);
        let baseline = pinned.baseline().unwrap();
        writer.conn.execute_batch(sql).unwrap();
        assert_eq!(
            writer
                .publish(&mutation(), Some(&baseline), &lock)
                .unwrap_err()
                .code,
            "BASELINE_REFUSED",
            "{sql}"
        );
        assert_exact(&pinned, &fixture("old"));
    }
}

#[test]
fn actual_parser_global_resolution_owned_cache_matches_complete_fresh_rebuild() {
    let d = private_parent();
    let path = d.path().join("scratch");
    let mut writer = DifferentialStore::create_scratch(&path).unwrap();
    let lock = writer.writer_lock().unwrap();
    let mut sources = [
        (
            "caller.py",
            "python",
            "from callee import target\n\ndef call():\n    target()\n",
        ),
        ("callee.py", "python", "def target():\n    pass\n"),
        (
            "types.rs",
            "rust",
            "struct Shape { pub width: usize }\nimpl Shape { fn width(&self)->usize { self.width } }\n",
        ),
        (
            "entry.ts",
            "typescript",
            "export function launch(): void { missing(); }\n",
        ),
    ];
    fn parsed(path: &str, language: &str, source: &str) -> ParsedFile {
        let mut file = kortex_parse::parse("source-repo", path, language, source).unwrap();
        file.symbols.reverse();
        file.diagnostics.push("exact stored file diagnostic".into());
        if let Some(s) = file.symbols.first_mut() {
            s.attributes["storage_equivalence"] = serde_json::json!({
                "fraction": 1.125, "negative_zero": -0.0, "maximum_unsigned": u64::MAX,
                "minimum_signed": i64::MIN, "nested": [true, null, {"items": ["a", "b"]}]
            });
        }
        file
    }
    fn snapshot(generation: &str, files: Vec<ParsedFile>) -> Snapshot {
        let mut s = Snapshot {
            repo_id: "source-repo".into(),
            root: "/fixed-actual-source".into(),
            generation_id: generation.into(),
            files,
            diagnostics: vec!["exact top-level diagnostic".into()],
            ..Default::default()
        };
        s.edges = kortex_languages::resolve_with_context(&s, &Default::default());
        s.edges.reverse();
        s
    }
    let old = snapshot(
        "old",
        sources
            .iter()
            .rev()
            .map(|(p, l, s)| parsed(p, l, s))
            .collect(),
    );
    assert!(
        old.edges
            .iter()
            .any(|e| e.kind == "CALLS" && e.resolution == "exact")
    );
    writer.publish(&old, None, &lock).unwrap();
    let legacy_root = d.path().join("legacy");
    fs::create_dir(&legacy_root).unwrap();
    let mut legacy = crate::Store::open(&legacy_root, true).unwrap();
    let legacy_lock = legacy.writer_lock().unwrap();
    legacy.commit(&old, &legacy_lock).unwrap();
    assert_eq!(
        serde_json::to_vec(&legacy.load().unwrap()).unwrap(),
        serde_json::to_vec(&old).unwrap()
    );
    let pinned = reader(&path);
    assert_exact(&pinned, &old);
    let (mut files, baseline) = pinned.baseline().unwrap().split_cached_files();
    sources[1].2 = "def renamed():\n    pass\n";
    let slot = files.iter_mut().find(|f| f.path == "callee.py").unwrap();
    *slot = parsed(sources[1].0, sources[1].1, sources[1].2);
    let next = snapshot("new", files);
    let full = snapshot(
        "new",
        sources
            .iter()
            .rev()
            .map(|(p, l, s)| parsed(p, l, s))
            .collect(),
    );
    assert_eq!(
        serde_json::to_value(&next).unwrap(),
        serde_json::to_value(&full).unwrap()
    );
    assert_ne!(old.edges, next.edges);
    assert_eq!(
        old.files
            .iter()
            .find(|f| f.path == "caller.py")
            .unwrap()
            .content_hash,
        next.files
            .iter()
            .find(|f| f.path == "caller.py")
            .unwrap()
            .content_hash
    );
    let changes = writer.publish(&next, Some(&baseline), &lock).unwrap();
    assert_eq!(changes.files_written, 1);
    assert!(changes.edges_written > 0 || changes.edges_deleted > 0);
    assert_exact(&reader(&path), &full);
    legacy.commit(&full, &legacy_lock).unwrap();
    assert_eq!(
        serde_json::to_vec(&legacy.load().unwrap()).unwrap(),
        serde_json::to_vec(&reader(&path).load().unwrap()).unwrap()
    );
    assert_exact(&pinned, &old);
}

#[test]
fn arbitrary_within_file_symbol_order_is_layout_not_fact_republication() {
    let d = private_parent();
    let path = d.path().join("scratch");
    let mut writer = DifferentialStore::create_scratch(&path).unwrap();
    let lock = writer.writer_lock().unwrap();
    let file = kortex_parse::parse(
        "repo",
        "order.py",
        "python",
        "def first():\n    pass\ndef second():\n    pass\n",
    )
    .unwrap();
    let old = Snapshot {
        repo_id: "repo".into(),
        root: "/fixed".into(),
        generation_id: "old".into(),
        files: vec![file],
        ..Default::default()
    };
    writer.publish(&old, None, &lock).unwrap();
    let pinned = reader(&path);
    let baseline = pinned.baseline().unwrap();
    let mut next = old.clone();
    next.generation_id = "reordered".into();
    next.files[0].symbols.reverse();
    assert_ne!(old.files[0].symbols, next.files[0].symbols);
    assert_eq!(
        writer.publish(&next, Some(&baseline), &lock).unwrap(),
        DifferentialChanges::default()
    );
    assert_exact(&reader(&path), &next);
    assert_exact(&pinned, &old);
}

fn crash_mutation() -> Snapshot {
    let mut s = mutation();
    let mut added = s.files.remove(0);
    added.path = "3.py".into();
    added.symbols[0].symbol_id = "symbol3".into();
    added.symbols[0].file_path = "3.py".into();
    added.symbols[0].name = "new%_3".into();
    s.files.push(added);
    s.edges.retain(|e| e.id != "edge2");
    s.edges
        .iter_mut()
        .find(|e| e.id == "edge1")
        .unwrap()
        .target_id = Some("symbol3".into());
    let mut added = s.edges[0].clone();
    added.id = "edge3".into();
    added.source_id = "symbol3".into();
    added.target_id = Some("symbol0".into());
    s.edges.push(added);
    s
}

pub(super) fn checkpoint(phase: &str) {
    if std::env::var("KORTEX_DIFFERENTIAL_TEST_PHASE").as_deref() == Ok(phase) {
        let root = std::env::var_os("KORTEX_DIFFERENTIAL_TEST_ROOT").unwrap();
        fs::write(Path::new(&root).join("ready"), phase).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        panic!("parent did not kill differential child");
    }
}
#[test]
fn crash_child() {
    let Some(root) = std::env::var_os("KORTEX_DIFFERENTIAL_TEST_ROOT") else {
        return;
    };
    let path = Path::new(&root).join("scratch");
    let mut store = DifferentialStore::open_scratch(&path, true).unwrap();
    let lock = store.writer_lock().unwrap();
    let pinned = reader(&path);
    let baseline = pinned.baseline().unwrap();
    store
        .publish(&crash_mutation(), Some(&baseline), &lock)
        .unwrap();
    panic!("did not reach requested checkpoint");
}
#[test]
fn killed_real_differential_publication_keeps_pinned_facts_fts_and_retry() {
    for phase in ["retired", "files", "nodes", "edges", "published"] {
        let d = private_parent();
        let path = d.path().join("scratch");
        let mut writer = DifferentialStore::create_scratch(&path).unwrap();
        let lock = writer.writer_lock().unwrap();
        let old = fixture("old");
        writer.publish(&old, None, &lock).unwrap();
        drop(lock);
        drop(writer);
        let pinned = reader(&path);
        assert_exact(&pinned, &old);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "experimental::tests::crash_child", "--nocapture"])
            .env("KORTEX_DIFFERENTIAL_TEST_ROOT", d.path())
            .env("KORTEX_DIFFERENTIAL_TEST_PHASE", phase)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !d.path().join("ready").exists() {
            if Instant::now() > deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not reach {phase}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let concurrent = reader(&path);
        assert_exact(&concurrent, &old);
        assert_exact(&pinned, &old);
        assert_eq!(concurrent.writer_lock().err().unwrap().code, "WRITER_BUSY");
        assert_eq!(pinned.keyword("old", 10).unwrap().len(), 3);
        assert!(concurrent.keyword("new", 10).unwrap().is_empty());
        child.kill().unwrap();
        child.wait().unwrap();
        let mut writer = DifferentialStore::open_scratch(&path, true).unwrap();
        let lock = writer.writer_lock().unwrap();
        let baseline = concurrent.baseline().unwrap();
        writer
            .publish(&crash_mutation(), Some(&baseline), &lock)
            .unwrap();
        assert_exact(&reader(&path), &crash_mutation());
        assert_exact(&pinned, &old);
        assert_exact(&concurrent, &old);
        assert_eq!(reader(&path).keyword("new", 10).unwrap().len(), 3);
        assert!(pinned.keyword("new", 10).unwrap().is_empty());
    }
}
