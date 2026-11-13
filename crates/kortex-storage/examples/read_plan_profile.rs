//! Real-source storage read/SQL-plan diagnostic. Never reference or percentile acceptance.
//! Compares an isolated optional covering-index trial; no existing index is mutated.
use kortex_core::{Snapshot, hash};
use kortex_storage::{QueryCandidateFilter, Store};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{path::Path, time::Instant};

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn plans(db: &Path) {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    for sql in [
        "SELECT count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),count(DISTINCT ordinal) FROM read_edge_order WHERE generation_id=?1",
        "SELECT coalesce(sum(n),0),count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),coalesce(max(invalid),0) FROM (SELECT ordinal,count(*) n,typeof(ordinal)<>'integer' invalid FROM read_edge_order WHERE generation_id=?1 GROUP BY ordinal)",
        "SELECT EXISTS(SELECT 1 FROM read_edge_order p LEFT JOIN edges e ON e.id=p.id AND e.generation_id=p.generation_id WHERE p.generation_id=?1 AND e.id IS NULL)",
        "SELECT n.id,n.name,n.qualified_name,n.file_path,p.language,n.kind,p.start_byte FROM read_symbol_inventory p JOIN nodes n ON n.id=p.id AND n.generation_id=p.generation_id WHERE p.generation_id=?1 ORDER BY p.ordinal",
    ] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let details: Vec<String> = stmt
            .query_map(["read-profile"], |r| r.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        println!("{}", json!({"sql":sql,"plan":details}));
    }
}
fn measure(root: &Path, snapshot: &Snapshot, label: &str, samples: usize) {
    for sample in 0..samples {
        let text = ["f0", "F0", "m00000.py", "next_call", "missing_term"][sample % 5];
        let start = Instant::now();
        let reader = Store::open(root, false).unwrap();
        let open_ms = ms(start);
        let session = reader.read_session().unwrap();
        let start = Instant::now();
        session.read_projection_source().unwrap();
        let validation_ms = ms(start);
        let start = Instant::now();
        let inventory = session.symbol_inventory().unwrap();
        let inventory_ms = ms(start);
        assert_eq!(inventory.len(), snapshot.symbols().count());
        drop(inventory);
        let start = Instant::now();
        let actual = session
            .query_candidate_inventory(text, &QueryCandidateFilter::default())
            .unwrap();
        let candidate_ms = ms(start);
        let lower = text.to_lowercase();
        let expected: Vec<_> = snapshot
            .symbols()
            .filter(|s| {
                s.name == text
                    || s.qualified_name == text
                    || s.name.to_lowercase().starts_with(&lower)
                    || s.qualified_name.to_lowercase().contains(&lower)
                    || s.file_path.to_lowercase().contains(&lower)
                    || actual.fts.iter().any(|(id, _)| id == &s.symbol_id)
            })
            .collect();
        assert_eq!(actual.items.len(), expected.len());
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
            let selected = session.get_symbol(&row.symbol_id).unwrap().unwrap();
            assert_eq!(&selected, expected);
        }
        let start = Instant::now();
        // This is a NEW session on the same immutable Store, as ReadService uses.
        reader
            .read_session()
            .unwrap()
            .read_projection_source()
            .unwrap();
        let same_store_validation_ms = ms(start);
        println!(
            "{}",
            json!({"diagnostic_only":true,"label":label,"sample":sample,"text":text,"open_ms":open_ms,"validation_ms":validation_ms,"inventory_ms":inventory_ms,"candidate_ms":candidate_ms,"candidate_rows":actual.items.len(),"same_store_validation_ms":same_store_validation_ms})
        );
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let files: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(10000);
    let functions: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10);
    let samples: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(10);
    assert!((3..=10000).contains(&files) && (1..=10).contains(&functions) && samples >= 10);
    let root = tempfile::tempdir().unwrap();
    let mut snapshot = Snapshot {
        repo_id: "diagnostic-resolver".into(),
        root: "/storage-read-profile-v1".into(),
        generation_id: "read-profile".into(),
        ..Default::default()
    };
    let mut dataset = String::new();
    for i in 0..files {
        let path = format!("m{i:05}.py");
        let mut source = format!("from m{:05} import f0 as next_call\n", (i + 1) % files);
        for j in 0..functions {
            source.push_str(&format!("def f{j}():\n    next_call()\n"));
        }
        dataset.push_str(&path);
        dataset.push('\0');
        dataset.push_str(&source);
        snapshot
            .files
            .push(kortex_parse::parse(&snapshot.repo_id, &path, "python", &source).unwrap());
    }
    snapshot.edges = kortex_languages::resolve(&snapshot, None);
    assert_eq!(
        snapshot
            .edges
            .iter()
            .filter(|e| e.kind == "CALLS" && e.resolution == "exact")
            .count(),
        files * functions
    );
    println!(
        "{}",
        json!({"diagnostic_only":true,"files":files,"functions":files*functions,"symbols":snapshot.symbols().count(),"edges":snapshot.edges.len(),"dataset_blake3":hash(dataset),"edges_blake3":hash(serde_json::to_vec(&snapshot.edges).unwrap()),"executable_blake3":hash(std::fs::read(std::env::current_exe().unwrap()).unwrap()),"snapshot_blake3":hash(serde_json::to_vec(&snapshot).unwrap()),"samples":samples})
    );
    let mut writer = Store::open(root.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&snapshot, &lock).unwrap();
    let db = writer.directory.join("index.db");
    let before = Store::open(root.path(), false).unwrap();
    let before_session = before.read_session().unwrap();
    let fts_hash = before_session.keyword_fingerprint().unwrap();
    assert_eq!(
        serde_json::to_vec(&before_session.load().unwrap()).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
    plans(&db);
    measure(root.path(), &snapshot, "before_covering_trial", samples);
    // Only this new private diagnostic database is changed, under its real writer lock.
    let connection = Connection::open(&db).unwrap();
    let start = Instant::now();
    connection.execute_batch("CREATE INDEX read_nodes_covering_trial ON nodes(generation_id,id,name,qualified_name,file_path,kind)").unwrap();
    println!("{}", json!({"covering_index_build_ms":ms(start)}));
    plans(&db);
    measure(root.path(), &snapshot, "after_covering_trial", samples);
    let after = Store::open(root.path(), false).unwrap();
    let after_session = after.read_session().unwrap();
    assert_eq!(fts_hash, after_session.keyword_fingerprint().unwrap());
    assert_eq!(fts_hash, before_session.keyword_fingerprint().unwrap());
    assert_eq!(
        serde_json::to_vec(&after_session.load().unwrap()).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&before_session.load().unwrap()).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
    assert_eq!(writer.integrity().unwrap(), "ok");
    println!(
        "{}",
        json!({"all_facts_and_fts_blake3":fts_hash,"complete_equivalence":true,"note":"nonquiescent diagnostic, not a target/p95/RSS improvement claim"})
    );
}
