//! Actual fixed-source parser/global-resolver publication diagnostic, NEVER adoption/migration.
//! Run each backend in a separate /usr/bin/time -l process for comparable whole-process RSS.
use kortex_core::{ParsedFile, Snapshot, hash};
use kortex_storage::{
    Store, WriterLock,
    experimental::{Baseline, DifferentialStore},
};
use std::{collections::HashMap, io::Write, path::Path, time::Instant};

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn emit(phase: &str, start: Instant) {
    println!("phase={phase} ms={:.3}", ms(start));
    std::io::stdout().flush().unwrap();
}
enum Backend {
    Schema1(Store),
    Differential(DifferentialStore),
}
impl Backend {
    fn open(mode: &str, root: &Path, writable: bool) -> Self {
        if mode == "schema1" {
            Self::Schema1(Store::open(root, writable).unwrap())
        } else {
            Self::Differential(
                DifferentialStore::open_scratch(&root.join("experiment"), writable).unwrap(),
            )
        }
    }
    fn writer_lock(&self) -> WriterLock {
        match self {
            Self::Schema1(s) => s.writer_lock().unwrap(),
            Self::Differential(s) => s.writer_lock().unwrap(),
        }
    }
    fn publish(&mut self, s: &Snapshot, b: Option<&Baseline>, lock: &WriterLock) {
        let start = Instant::now();
        match self {
            Self::Schema1(store) => {
                let timings = store.commit_profiled(s, lock).unwrap();
                println!(
                    "generation={} total_publish_ms={:.3} schema1_phases={timings:?}",
                    s.generation_id,
                    ms(start)
                );
            }
            Self::Differential(store) => {
                let (changes, timings) = store.publish_profiled(s, b, lock).unwrap();
                println!(
                    "generation={} total_publish_ms={:.3} changes={changes:?} differential_phases={timings:?}",
                    s.generation_id,
                    ms(start)
                );
            }
        }
        std::io::stdout().flush().unwrap();
    }
    fn load(&self) -> Snapshot {
        match self {
            Self::Schema1(s) => s.read_session().unwrap().load().unwrap(),
            Self::Differential(s) => s.load().unwrap(),
        }
    }
    fn fts_digest(&self) -> String {
        match self {
            Self::Schema1(s) => s.read_session().unwrap().keyword_fingerprint().unwrap(),
            Self::Differential(s) => s.keyword_fingerprint().unwrap(),
        }
    }
    fn keyword(&self, text: &str) -> Vec<(String, f64)> {
        match self {
            Self::Schema1(s) => s.read_session().unwrap().keyword(text, 1000).unwrap(),
            Self::Differential(s) => s.keyword(text, 1000).unwrap(),
        }
    }
    fn cached(&self) -> (Vec<ParsedFile>, Option<Baseline>) {
        match self {
            Self::Schema1(_) => (self.load().files, None),
            Self::Differential(s) => {
                let (files, baseline) = s.baseline().unwrap().split_cached_files();
                (files, Some(baseline))
            }
        }
    }
}
fn snapshot(files: Vec<ParsedFile>, generation: &str) -> Snapshot {
    Snapshot {
        repo_id: "diagnostic-resolver".into(),
        root: "/storage-parsed-profile-v1".into(),
        generation_id: generation.into(),
        files,
        ..Default::default()
    }
}
fn resolve(s: &mut Snapshot, expected_calls: usize) {
    let start = Instant::now();
    s.edges = kortex_languages::resolve(s, None);
    assert_eq!(
        s.edges.iter().filter(|e| e.kind == "CALLS").count(),
        expected_calls
    );
    emit("complete_global_resolution", start);
}
fn digest(s: &Snapshot) -> String {
    hash(serde_json::to_vec(s).unwrap())
}
fn verify(reader: &Backend, expected: &Snapshot) -> String {
    let start = Instant::now();
    let actual = reader.load();
    // Compare complete serialized facts, preserving vector order and all numeric/diagnostic data.
    assert_eq!(digest(&actual), digest(expected));
    println!(
        "generation={} snapshot_blake3={} files_blake3={} edges_blake3={}",
        actual.generation_id,
        digest(&actual),
        hash(serde_json::to_vec(&actual.files).unwrap()),
        hash(serde_json::to_vec(&actual.edges).unwrap())
    );
    emit("load_complete_facts_and_hash_compare", start);
    let start = Instant::now();
    let fts = reader.fts_digest();
    println!(
        "generation={} complete_actual_fts_blake3={fts}",
        expected.generation_id
    );
    emit("complete_actual_fts_hash", start);
    fts
}
fn main() {
    kortex_repo::placement::require_derived_data_support().unwrap();
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("differential");
    assert!(["schema1", "differential"].contains(&mode));
    let files: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10000);
    let functions: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(10);
    assert!(files > 2 && functions > 0);
    let mut dataset = String::new();
    let mut sources: Vec<_> = (0..files)
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
        "diagnostic_only=true acceptance=false backend={mode} parser={} files={files} functions_per_file={functions} samples=1 dataset_blake3={}",
        kortex_core::PARSER_VERSION,
        hash(&dataset)
    );
    drop(dataset);
    let start = Instant::now();
    let parsed = sources
        .iter()
        .map(|(path, source)| {
            kortex_parse::parse("diagnostic-resolver", path, "python", source).unwrap()
        })
        .collect();
    emit("parse_all_source", start);
    let mut initial = snapshot(parsed, "diagnostic-initial");
    resolve(&mut initial, files * functions);
    let symbols: HashMap<_, _> = initial
        .symbols()
        .map(|s| (s.symbol_id.as_str(), s))
        .collect();
    for e in initial.edges.iter().filter(|e| e.kind == "CALLS") {
        assert_eq!(e.resolution, "exact");
        let source = symbols[e.source_id.as_str()];
        let target = symbols[e.target_id.as_deref().unwrap()];
        let i: usize = source
            .file_path
            .trim_start_matches('m')
            .trim_end_matches(".py")
            .parse()
            .unwrap();
        assert_eq!(target.file_path, format!("m{:05}.py", (i + 1) % files));
        assert_eq!(target.name, "f0");
    }
    drop(symbols);
    println!(
        "symbols={} edges={} source_edges_blake3={}",
        initial.symbols().count(),
        initial.edges.len(),
        hash(serde_json::to_vec(&initial.edges).unwrap())
    );
    let mut builder = tempfile::Builder::new();
    builder.prefix("kortex-storage-profile-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let root = builder.tempdir().unwrap();
    let mut writer = if mode == "schema1" {
        Backend::Schema1(Store::open(root.path(), true).unwrap())
    } else {
        Backend::Differential(
            DifferentialStore::create_scratch(&root.path().join("experiment")).unwrap(),
        )
    };
    let lock = writer.writer_lock();
    writer.publish(&initial, None, &lock);
    let pinned = Backend::open(mode, root.path(), false);
    let old_hash = digest(&initial);
    let old_fts = verify(&pinned, &initial);
    drop(initial);
    let start = Instant::now();
    let (cached, baseline) = pinned.cached();
    emit("actual_baseline_scan_and_owned_cache", start);
    let unchanged = snapshot(cached, "diagnostic-unchanged");
    // COMPLETE facts must be supplied even for unchanged publication.
    let mut unchanged = unchanged;
    resolve(&mut unchanged, files * functions);
    writer.publish(&unchanged, baseline.as_ref(), &lock);
    let current = Backend::open(mode, root.path(), false);
    assert_eq!(verify(&current, &unchanged), old_fts);
    drop(unchanged);
    drop(baseline);
    let start = Instant::now();
    let (mut cached, baseline) = current.cached();
    emit("replacement_actual_baseline_scan_and_owned_cache", start);
    let start = Instant::now();
    sources[1].1 = sources[1].1.replace("def f0():", "def renamedneedle():");
    cached[1] = kortex_parse::parse(
        "diagnostic-resolver",
        &sources[1].0,
        "python",
        &sources[1].1,
    )
    .unwrap();
    emit("one_file_actual_source_edit_parse", start);
    let mut updated = snapshot(cached, "diagnostic-updated");
    resolve(&mut updated, files * functions);
    // Module zero is unchanged but its cross-file calls MUST lose the removed target.
    let zero_ids: std::collections::HashSet<_> = updated.files[0]
        .symbols
        .iter()
        .map(|s| s.symbol_id.as_str())
        .collect();
    let zero_calls: Vec<_> = updated
        .edges
        .iter()
        .filter(|e| e.kind == "CALLS" && zero_ids.contains(e.source_id.as_str()))
        .collect();
    assert_eq!(zero_calls.len(), functions);
    assert!(
        zero_calls
            .iter()
            .all(|e| e.resolution != "exact" && e.target_id.is_none())
    );
    drop(zero_calls);
    drop(zero_ids);
    // Independent complete reparse/re-resolution, not a hash-only expected result.
    let start = Instant::now();
    let reparsed = sources
        .iter()
        .map(|(p, s)| kortex_parse::parse("diagnostic-resolver", p, "python", s).unwrap())
        .collect();
    let mut fresh = snapshot(reparsed, "diagnostic-updated");
    fresh.edges = kortex_languages::resolve(&fresh, None);
    assert_eq!(digest(&fresh), digest(&updated));
    emit("independent_fresh_reparse_global_equivalence", start);
    drop(fresh);
    writer.publish(&updated, baseline.as_ref(), &lock);
    let new_reader = Backend::open(mode, root.path(), false);
    let new_fts = verify(&new_reader, &updated);
    assert_ne!(old_fts, new_fts);
    assert_eq!(new_reader.keyword("renamedneedle").len(), 1);
    assert!(pinned.keyword("renamedneedle").is_empty());
    assert_eq!(digest(&pinned.load()), old_hash);
    assert_eq!(pinned.fts_digest(), old_fts);
    assert_eq!(current.fts_digest(), old_fts);
    if let Backend::Schema1(store) = &writer {
        assert_eq!(store.integrity().unwrap(), "ok");
    }
    println!(
        "complete_facts_fts_and_pinned_old_generation_verified=true acceptance=false no_percentile_claim=true excludes_discovery_watch_query_reference_distributions_and_migration=true"
    );
}
