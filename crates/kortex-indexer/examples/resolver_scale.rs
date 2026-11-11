//! Resolver phase isolation at reference file/symbol cardinality; NOT end-to-end acceptance.
//! Generated Python is inert data. Optional `publish` measures ONE real SQLite commit.
use kortex_core::{Snapshot, hash};
use rayon::prelude::*;
use std::{collections::HashMap, hint::black_box, io::Write, time::Instant};
fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let files: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(10000);
    let functions: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10);
    let samples: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(7);
    assert!(files > 1 && functions > 0 && samples > 0);
    let mut dataset = String::new();
    let sources: Vec<_> = (0..files)
        .map(|i| {
            let path = format!("m{i:05}.py");
            let mut source = format!("from m{:05} import f0 as next_call\n", (i + 1) % files);
            for j in 0..functions {
                source.push_str(&format!("def f{j}():\n    next_call()\n"));
            }
            dataset.push_str(&path);
            dataset.push('\0');
            dataset.push_str(&source);
            (path, source)
        })
        .collect();
    println!(
        "diagnostic_only=true phase_isolation=true parser={} files={files} functions_per_file={functions} samples={samples} dataset_sha256={}",
        kortex_core::PARSER_VERSION,
        hash(dataset)
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let start = Instant::now();
    let parsed = pool.install(|| {
        sources
            .par_iter()
            .map(|(path, source)| {
                kortex_parse::parse("diagnostic-resolver", path, "python", source).unwrap()
            })
            .collect()
    });
    let mut snapshot = Snapshot {
        repo_id: "diagnostic-resolver".into(),
        generation_id: "diagnostic-initial".into(),
        files: parsed,
        ..Default::default()
    };
    println!(
        "parse_only_ms={:.3} symbols={}",
        elapsed(start),
        snapshot.symbols().count()
    );
    let symbols: HashMap<_, _> = snapshot
        .symbols()
        .map(|s| (s.symbol_id.as_str(), s))
        .collect();
    let mut digest = None;
    let mut timings = vec![];
    for i in 0..samples {
        let start = Instant::now();
        let edges = kortex_languages::resolve(black_box(&snapshot), None);
        let ms = elapsed(start);
        timings.push(ms);
        let calls: Vec<_> = edges.iter().filter(|e| e.kind == "CALLS").collect();
        assert_eq!(calls.len(), files * functions);
        for e in calls {
            assert_eq!(e.resolution, "exact");
            let source = symbols[&e.source_id.as_str()];
            let target = symbols[&e.target_id.as_deref().unwrap()];
            assert_eq!(target.name, "f0");
            let i: usize = source
                .file_path
                .trim_start_matches('m')
                .trim_end_matches(".py")
                .parse()
                .unwrap();
            assert_eq!(target.file_path, format!("m{:05}.py", (i + 1) % files));
        }
        let current = hash(serde_json::to_vec(&edges).unwrap());
        if let Some(expected) = &digest {
            assert_eq!(expected, &current);
        } else {
            digest = Some(current);
        }
        println!("resolve_sample={i} ms={ms:.3} edges={}", edges.len());
        black_box(edges);
    }
    drop(symbols);
    timings.sort_by(f64::total_cmp);
    println!(
        "resolve_only_median_ms={:.3} p95_ms={:.3} edge_sha256={}",
        timings[samples / 2],
        timings[(samples as f64 * 0.95).ceil() as usize - 1],
        digest.unwrap()
    );
    let start = Instant::now();
    let edges = kortex_languages::resolve_with_context(&snapshot, &Default::default());
    println!("resolve_default_context_ms={:.3}", elapsed(start));
    snapshot.edges = edges;
    std::io::stdout().flush().unwrap();
    if args.get(4).is_some_and(|arg| arg == "publish") {
        let d = tempfile::tempdir().unwrap();
        snapshot.root = kortex_core::public_path(d.path());
        let mut store = kortex_storage::Store::open(d.path(), true).unwrap();
        let lock = store.writer_lock().unwrap();
        println!(
            "sqlite_commit_start=true files={files} symbols={} edges={}",
            snapshot.symbols().count(),
            snapshot.edges.len()
        );
        std::io::stdout().flush().unwrap();
        let start = Instant::now();
        store.commit(&snapshot, &lock).unwrap();
        println!("sqlite_single_commit_ms={:.3}", elapsed(start));
        let start = Instant::now();
        assert_eq!(store.load().unwrap().edges, snapshot.edges);
        println!("sqlite_load_validate_ms={:.3}", elapsed(start));
        if args.get(5).is_some_and(|arg| arg == "replace") {
            snapshot.generation_id = "diagnostic-replacement".into();
            let start = Instant::now();
            store.commit(&snapshot, &lock).unwrap();
            println!("sqlite_replacement_commit_ms={:.3}", elapsed(start));
            assert_eq!(store.load().unwrap().edges, snapshot.edges);
        }
    }
    println!(
        "acceptance=false note=phase_diagnostic_excludes_discovery_watch_query_and_reference_sample_counts"
    );
}
