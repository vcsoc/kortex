//! Tiny actual-parser/SQLite-plan correctness smoke, not a latency benchmark.
use kortex_core::Snapshot;
use kortex_storage::Store;
use rusqlite::{Connection, OpenFlags};

fn main() {
    let root = tempfile::tempdir().unwrap();
    let mut snapshot = Snapshot {
        repo_id: "query-scope-smoke".into(),
        root: "/fixed-query-scope-smoke".into(),
        generation_id: "smoke".into(),
        ..Default::default()
    };
    for i in 0..3 {
        let source = format!(
            "from m{} import target\ndef target():\n    target()\n",
            (i + 1) % 3
        );
        snapshot.files.push(
            kortex_parse::parse(&snapshot.repo_id, &format!("m{i}.py"), "python", &source).unwrap(),
        );
    }
    snapshot.edges = kortex_languages::resolve(&snapshot, None);
    let mut writer = Store::open(root.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&snapshot, &lock).unwrap();
    let reader = Store::open(root.path(), false).unwrap();
    let session = reader.read_session().unwrap();
    let fts = session.keyword("target", 500).unwrap();
    let candidates = session
        .query_scoped_candidate_inventory("target", &Default::default())
        .unwrap();
    assert_eq!(fts, candidates.fts);
    for entry in &candidates.items {
        assert_eq!(
            Some(&session.query_symbol(entry).unwrap()),
            snapshot.symbol(&entry.symbol_id)
        );
    }
    assert_eq!(
        session.query_file_inventory_snapshot_order().unwrap().len(),
        3
    );
    assert_eq!(session.query_parser_diagnostic_presence().unwrap().len(), 3);
    assert_eq!(
        serde_json::to_vec(&session.load().unwrap()).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
    assert_eq!(reader.integrity().unwrap(), "ok");
    let conn = Connection::open_with_flags(
        writer.directory.join("index.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for sql in [
        "SELECT id,name,qualified_name,file_path,language,kind,start_byte FROM query_symbol_inventory WHERE generation_id=?1 ORDER BY ordinal",
        "SELECT EXISTS(SELECT 1 FROM query_symbol_inventory p LEFT JOIN nodes f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 AND (typeof(p.id)<>'text' OR f.id IS NULL))",
    ] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan: Vec<String> = stmt
            .query_map(["smoke"], |r| r.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        println!("{}", serde_json::json!({"sql": sql, "plan": plan}));
    }
    println!(
        "{}",
        serde_json::json!({"small_smoke_only": true, "files": snapshot.files.len(), "symbols": snapshot.symbols().count(), "edges": snapshot.edges.len(), "candidate_rows": candidates.items.len(), "complete_snapshot_and_selected_facts": true, "performance_claim": false})
    );
}
