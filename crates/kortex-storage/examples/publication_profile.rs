//! Fixed-domain publication diagnostic. Generated facts are inert, not parsed source.
//! This is not query/indexer/platform acceptance. Run identical arguments before/after.
use kortex_core::{Edge, ParsedFile, Snapshot, Span, Symbol, hash, id};
use kortex_storage::Store;
use std::{io::Write, time::Instant};
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "10000".into())
        .parse()
        .unwrap();
    assert!(n > 1);
    let repo = "storage-fixed-dataset-v1";
    let mut snapshot = Snapshot {
        repo_id: repo.into(),
        root: "/storage-fixed-dataset-v1".into(),
        generation_id: "first".into(),
        ..Default::default()
    };
    for i in 0..n {
        let path = format!("m{i:05}.py");
        let mut file = ParsedFile {
            path: path.clone(),
            language: "python".into(),
            content_hash: hash(&path),
            size_bytes: 330,
            symbols: vec![],
            references: vec![],
            imports: vec![],
            import_scopes: vec![],
            import_spans: vec![],
            diagnostics: vec![],
        };
        for j in 0..11 {
            let name = if j == 0 {
                path.clone()
            } else {
                format!("f{}", j - 1)
            };
            let symbol_id = id(&[repo, &path, &j.to_string()]);
            file.symbols.push(Symbol {
                symbol_id: symbol_id.clone(),
                symbol_revision_id: id(&[&symbol_id, "revision"]),
                repo_id: repo.into(),
                kind: if j == 0 { "File" } else { "Function" }.into(),
                language: "python".into(),
                name: name.clone(),
                qualified_name: format!("m{i:05}.{name}"),
                file_path: path.clone(),
                span: Span {
                    start_line: j * 3 + 1,
                    end_line: j * 3 + 3,
                    ..Default::default()
                },
                signature: format!("def {name}():"),
                visibility: "public".into(),
                attributes: serde_json::json!({"source_snippet": "next_call()"}),
                content_hash: hash(&name),
                doc_comment: None,
                owner_id: None,
            });
        }
        for j in 0..32 {
            let source = id(&[repo, &path, &(j % 11).to_string()]);
            let next_path = format!("m{:05}.py", (i + 1) % n);
            let target = id(&[repo, &next_path, &(j % 11).to_string()]);
            snapshot.edges.push(Edge {
                id: id(&[repo, &path, &j.to_string(), "edge"]),
                repo_id: repo.into(),
                source_id: source,
                target_id: Some(target),
                kind: "CALLS".into(),
                source_span: Span::default(),
                resolution: "exact".into(),
                confidence: 1.0,
                resolver: "fixed-domain-v1".into(),
                evidence: "fixed cross-file edge".into(),
                candidates: vec![],
            });
        }
        snapshot.files.push(file);
    }
    println!(
        "diagnostic_only=true fixed_domain_v1=true files={n} symbols={} edges={} snapshot_blake3={}",
        snapshot.symbols().count(),
        snapshot.edges.len(),
        hash(serde_json::to_vec(&snapshot).unwrap())
    );
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    for generation in ["first", "replacement", "one-file-change"] {
        snapshot.generation_id = generation.into();
        if generation == "one-file-change" {
            snapshot.files[0].content_hash = hash("changed");
            snapshot.files[0].symbols[1].name = "changedneedle".into();
        }
        std::io::stdout().flush().unwrap();
        let start = Instant::now();
        let timings = store.commit_profiled(&snapshot, &lock).unwrap();
        println!(
            "generation={generation} total_ms={:.3} phases={timings:?}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(store.load().unwrap().edges, snapshot.edges);
        assert_eq!(store.load().unwrap().files, snapshot.files);
        if generation == "one-file-change" {
            assert_eq!(
                store
                    .keyword(generation, "changedneedle", 10)
                    .unwrap()
                    .len(),
                1
            );
        }
    }
    let start = Instant::now();
    assert_eq!(store.integrity().unwrap(), "ok");
    println!(
        "full_integrity_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    println!("acceptance=false excludes_parser_resolver_discovery_queries_watch_and_quiescence");
}
