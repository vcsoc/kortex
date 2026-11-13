//! Actual inert source only; the Go fixture is parsed, never compiled or executed.
use kortex_config::Config;
use kortex_indexer::{Options, index_at_root};
use kortex_storage::Store;
use std::fs;

#[test]
fn default_output_refusal_preserves_committed_index_then_retry_reuses_facts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = root.join("main.go");
    let seed = "package app\nfunc retained_entry() {}\n";
    fs::write(&path, seed).unwrap();
    let config = Config::default();
    let first = index_at_root(root, &config, &Options::default()).unwrap();
    let db = root.join(".kortex/index.db");
    let original = fs::read(&db).unwrap();
    let names = (0..9)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let ty = format!("struct{{ Value string `{}` }}", "\\".repeat(1920 * 1024));
    let source =
        format!("package app\ntype Service struct{{}}\nfunc (s *Service) Run({names} {ty}) {{}}\n");
    assert!(source.len() < config.repository.max_file_size_bytes as usize);
    fs::write(&path, source).unwrap();
    let error = match index_at_root(root, &config, &Options::default()) {
        Err(error) => error,
        Ok(_) => panic!("oversized emitted facts must not publish"),
    };
    assert_eq!(error.code, "RESOURCE_LIMIT");
    assert_eq!(error.exit, 9);
    assert!(error.message.contains("JSON bytes"));
    assert_eq!(fs::read(&db).unwrap(), original);
    let retained = Store::open(root, false).unwrap().load().unwrap();
    assert_eq!(retained.generation_id, first.snapshot.generation_id);
    assert_eq!(retained.files, first.snapshot.files);
    assert_eq!(retained.edges, first.snapshot.edges);
    fs::write(path, seed).unwrap();
    let retry = index_at_root(root, &config, &Options::default()).unwrap();
    assert_eq!(retry.reused, 1);
    assert_eq!(retry.snapshot.files, first.snapshot.files);
    assert_eq!(retry.snapshot.edges, first.snapshot.edges);
}
