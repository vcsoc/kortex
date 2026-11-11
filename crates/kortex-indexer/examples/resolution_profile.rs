//! Small, reproducible diagnostic, NOT reference-scale acceptance or target enforcement.
//! Runs only Kortex; generated Python is inert fixture data. No uploads or dependencies.
use kortex_config::Config;
use kortex_core::{Snapshot, hash};
use kortex_indexer::{Options, index_at_root};
use std::{hint::black_box, time::Instant};
fn millis(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn summary(label: &str, values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
    let p95 = (values.len() as f64 * 0.95).ceil() as usize - 1;
    println!(
        "{label}: n={} min_ms={:.3} median_ms={:.3} p95_ms={:.3}",
        values.len(),
        values[0],
        values[values.len() / 2],
        values[p95]
    );
}
fn edge_hash(s: &Snapshot) -> String {
    hash(serde_json::to_vec(&s.edges).unwrap())
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let files: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(1000);
    let functions: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10);
    let samples: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(7);
    assert!(files > 1 && functions > 0 && samples > 0);
    let d = tempfile::tempdir().unwrap();
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
        std::fs::write(d.path().join(path), source).unwrap();
    }
    let config = Config::default();
    println!(
        "diagnostic_only=true parser={} files={files} functions_per_file={functions} dataset_sha256={} workers=default max_workers=8",
        kortex_core::PARSER_VERSION,
        hash(&dataset)
    );
    let start = Instant::now();
    let first = index_at_root(d.path(), &config, &Options::default()).unwrap();
    println!(
        "full_index_ms={:.3} symbols={} edges={} edge_sha256={}",
        millis(start),
        first.snapshot.symbols().count(),
        first.snapshot.edges.len(),
        edge_hash(&first.snapshot)
    );
    let mut resolve_times = vec![];
    for _ in 0..samples {
        let start = Instant::now();
        let edges = kortex_languages::resolve(black_box(&first.snapshot), None);
        resolve_times.push(millis(start));
        assert_eq!(edges, first.snapshot.edges);
        black_box(edges);
    }
    summary("resolve_immutable_facts", &mut resolve_times);
    let mut update_times = vec![];
    let edit = d.path().join("m00000.py");
    let original = std::fs::read_to_string(&edit).unwrap();
    for i in 0..samples {
        std::fs::write(&edit, format!("{original}# diagnostic update {i}\n")).unwrap();
        let start = Instant::now();
        let update = index_at_root(d.path(), &config, &Options::default()).unwrap();
        update_times.push(millis(start));
        assert_eq!((update.parsed, update.reused), (1, files - 1));
        assert_eq!(update.snapshot.edges, first.snapshot.edges);
    }
    summary("single_comment_index_publish", &mut update_times);
    println!(
        "note=warm_local_diagnostic_includes_global_resolution_and_sqlite_publication_not_watch_latency_not_reference_acceptance"
    );
}
