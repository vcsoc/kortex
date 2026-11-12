//! Real-source phase diagnostic, not reference/quiescent performance acceptance.
//! Inert generated Python is never executed. Every shared operation hashes sources.
use kortex_analysis::{ReadService, Service};
use kortex_config::Config;
use kortex_core::{RequestControl, hash};
use kortex_storage::Store;
use serde_json::{Value, json};
use std::{
    process::Command,
    time::{Duration, Instant},
};
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn stable(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("request_id");
    value.as_object_mut().unwrap().remove("timing_ms");
    value
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let files: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(10000);
    let functions: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10);
    let samples: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(10);
    assert!(
        (2..=10000).contains(&files)
            && (1..=10).contains(&functions)
            && (2..=1000).contains(&samples)
    );
    let cli = std::env::var_os("KORTEX_TEST_CLI").map(std::path::PathBuf::from);
    let cli_blake3 = cli.as_ref().map(|path| hash(std::fs::read(path).unwrap()));
    let executable_blake3 = hash(std::fs::read(std::env::current_exe().unwrap()).unwrap());
    let temp = tempfile::tempdir().unwrap();
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
        std::fs::write(temp.path().join(path), source).unwrap();
    }
    let config = Config::default();
    let begin = Instant::now();
    let report = kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    println!(
        "{}",
        json!({"diagnostic_only":true,"phase":"index","ms":ms(begin),"files":files,"functions":files*functions,"symbols":report.snapshot.symbols().count(),"edges":report.snapshot.edges.len(),"dataset_blake3":hash(dataset),"edges_blake3":hash(serde_json::to_vec(&report.snapshot.edges).unwrap()),"parser":kortex_core::PARSER_VERSION,"samples":samples,"cli":cli,"cli_blake3":cli_blake3,"profile_executable_blake3":executable_blake3})
    );
    drop(report);
    let queries = ["f0", "F0", "m00000.py", "next_call", "missing_term"];
    for sample in 0..samples {
        let text = queries[sample % queries.len()];
        let request = json!({"text":text,"limit":20,"require_fresh":true});
        let control = RequestControl::new(Duration::from_secs(60)).unwrap();
        let begin = Instant::now();
        let store = Store::open(temp.path(), false).unwrap();
        let store_open_ms = ms(begin);
        let begin = Instant::now();
        let session = store.read_session().unwrap();
        let metadata_ms = ms(begin);
        let begin = Instant::now();
        assert_eq!(
            session.read_projection_source().unwrap(),
            kortex_storage::MetadataSource::Projection
        );
        let projection_validation_ms = ms(begin);
        let begin = Instant::now();
        let inventory = session.file_inventory_snapshot_order().unwrap();
        let file_inventory_ms = ms(begin);
        drop(inventory);
        let begin = Instant::now();
        let inventory = session.symbol_inventory().unwrap();
        let symbol_inventory_ms = ms(begin);
        drop(inventory);
        let begin = Instant::now();
        let fts = session.keyword(text, 500).unwrap();
        let fts500_ms = ms(begin);
        drop(fts);
        let begin = Instant::now();
        let lazy_hits = kortex_search::projected::query_projected_with_control(
            &session,
            text,
            20,
            0,
            &Default::default(),
            &control,
        )
        .unwrap();
        let projected_retrieval_ms = ms(begin);
        let begin = Instant::now();
        let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
        let eager_open_ms = ms(begin);
        let begin = Instant::now();
        let eager_hits = kortex_search::query(&eager.store, &eager.snapshot, text, 20, 0).unwrap();
        let eager_retrieval_ms = ms(begin);
        assert_eq!(
            serde_json::to_value(lazy_hits).unwrap(),
            serde_json::to_value(eager_hits).unwrap()
        );
        let begin = Instant::now();
        let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
        let lazy_open_ms = ms(begin);
        let begin = Instant::now();
        assert_eq!(lazy.freshness().unwrap(), "fresh");
        let freshness_ms = ms(begin);
        let begin = Instant::now();
        let lazy_response = lazy.execute("query", &request).unwrap();
        let lazy_execute_with_freshness_ms = ms(begin);
        assert!(!lazy.eager_materialized());
        let begin = Instant::now();
        let eager_response = eager.execute("query", &request).unwrap();
        let eager_execute_with_freshness_ms = ms(begin);
        assert_eq!(stable(lazy_response.clone()), stable(eager_response));
        let cli_process_ms = cli.as_ref().map(|binary| {
            let begin = Instant::now();
            #[cfg(target_os = "macos")]
            let mut command = {
                let mut command = Command::new("/usr/bin/time");
                command.arg("-l").arg(binary);
                command
            };
            #[cfg(not(target_os = "macos"))]
            let mut command = Command::new(binary);
            let output = command
                .arg("--repo")
                .arg(temp.path())
                .args(["--json", "--require-fresh", "query", text, "--limit", "20"])
                .output()
                .unwrap();
            let elapsed = ms(begin);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(stable(value), stable(lazy_response));
            // Darwin time -l reports bytes; other platforms leave this unclaimed.
            let cli_max_rss_bytes = String::from_utf8_lossy(&output.stderr)
                .lines()
                .find(|line| line.contains("maximum resident set size"))
                .and_then(|line| line.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok());
            json!({"wall_ms":elapsed,"max_rss_bytes":cli_max_rss_bytes})
        });
        println!(
            "{}",
            json!({"diagnostic_only":true,"sample":sample,"text":text,"store_open_ms":store_open_ms,"metadata_ms":metadata_ms,"projection_validation_ms":projection_validation_ms,"file_inventory_ms":file_inventory_ms,"symbol_inventory_ms":symbol_inventory_ms,"fts500_ms":fts500_ms,"projected_retrieval_without_freshness_ms":projected_retrieval_ms,"eager_open_ms":eager_open_ms,"eager_retrieval_without_freshness_ms":eager_retrieval_ms,"lazy_open_ms":lazy_open_ms,"freshness_ms":freshness_ms,"lazy_execute_with_freshness_ms":lazy_execute_with_freshness_ms,"eager_execute_with_freshness_ms":eager_execute_with_freshness_ms,"cli_process":cli_process_ms,"query_eager_materialized":lazy.eager_materialized()})
        );
    }
    println!(
        "note=full_shared_query_and_optional_actual_cli_process_diagnostic_not_quiescent_not_reference_acceptance_no_freshness_bypass_context_impact_still_eager"
    );
}
