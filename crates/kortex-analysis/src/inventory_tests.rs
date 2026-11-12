use super::*;
#[test]
fn actual_file_inventory_has_generation_pinned_filtered_pages() {
    let (_temp, mut service) = indexed(&[
        ("a.py", "def a():\n    pass\n"),
        ("src/b.rs", "fn b() {}\n"),
        ("src/c.rs", "fn c() {}\n"),
    ]);
    let first = service
        .execute(
            "files",
            &json!({"path_prefix":"src/","limit":1,"generation_id":service.snapshot.generation_id}),
        )
        .unwrap();
    assert_eq!(first["data"]["items"][0]["path"], "src/b.rs");
    assert_eq!(first["data"]["items"][0]["language"], "rust");
    assert_eq!(
        first["data"]["items"][0]["content_hash"],
        hash("fn b() {}\n")
    );
    assert_eq!(first["data"]["items"][0]["size_bytes"], 10);
    let args = json!({"path_prefix":"src/","limit":1,"generation_id":service.snapshot.generation_id,"cursor":first["page"]["next_cursor"]});
    let second = service.execute("files", &args).unwrap();
    assert_eq!(second["data"]["items"][0]["path"], "src/c.rs");
    assert!(second["page"]["next_cursor"].is_null());
    let mut other = args.clone();
    other["path_prefix"] = json!("a");
    assert_eq!(
        service.execute("files", &other).unwrap_err().code,
        "INVALID_CURSOR"
    );
    service.snapshot.generation_id = "changed".into();
    let mut unpinned = args;
    unpinned.as_object_mut().unwrap().remove("generation_id");
    assert_eq!(
        service.execute("files", &unpinned).unwrap_err().code,
        "INVALID_CURSOR"
    );
    jsonschema::validator_for(&response_schema("files"))
        .unwrap()
        .validate(&first)
        .unwrap();
}
#[test]
fn impact_includes_persisted_test_kinds_through_reverse_call_graph_without_claiming_framework_proof()
 {
    let (_temp, service) = indexed(&[
        (
            "app.py",
            "from auth import check\ndef entry():\n    check()\ndef test_entry():\n    entry()\n",
        ),
        ("auth.py", "def check():\n    pass\n"),
    ]);
    let persisted = service
        .snapshot
        .symbols()
        .find(|s| s.name == "test_entry")
        .unwrap();
    assert_eq!(persisted.kind, "test");
    let impact = service
        .execute("impact", &json!({"symbol":"check","depth":3}))
        .unwrap();
    assert_eq!(
        impact["data"]["likely_tests"].as_array().unwrap().len(),
        1,
        "{impact:#}"
    );
    let association = &impact["data"]["likely_test_associations"][0];
    assert_eq!(association["edge_path"].as_array().unwrap().len(), 2);
    assert_eq!(association["test_evidence"]["framework_verified"], false);
    assert_eq!(association["test_evidence"]["heuristic"], true);
    for edge_id in association["edge_path"].as_array().unwrap() {
        assert!(
            service
                .snapshot
                .edges
                .iter()
                .any(|e| e.id == edge_id.as_str().unwrap()
                    && e.kind == "CALLS"
                    && e.resolution == "exact")
        );
    }
}
#[test]
fn invocation_config_root_is_resolved_once_and_open_at_root_never_expands_git() {
    if std::env::var_os("KORTEX_ROOT_ONCE_CHILD").is_none() {
        let isolated = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "intelligence_tests::inventory::invocation_config_root_is_resolved_once_and_open_at_root_never_expands_git", "--nocapture"])
            .env("KORTEX_ROOT_ONCE_CHILD", "1")
            .env("KORTEX_REGISTRY", isolated.path().join("private-registry/registry.db"))
            .env("XDG_CONFIG_HOME", isolated.path().join("config"))
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    std::fs::write(
        temp.path().join("nested/code.py"),
        "def value():\n    pass\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join(".kortex.toml"),
        "[repository]\nroot='nested'\n",
    )
    .unwrap();
    let config = Config::load(temp.path(), None).unwrap();
    let root = config.resolved_root(temp.path()).unwrap();
    kortex_indexer::index_at_root(&root, &config, &Default::default()).unwrap();
    let service = Service::open(temp.path()).unwrap();
    assert_eq!(service.root, root);
    assert_eq!(service.snapshot.files.len(), 1);
    assert!(service.lookup("value").is_ok());
    let direct = Service::open_at_root(&root, config).unwrap();
    assert_eq!(direct.root, root);
    assert_eq!(direct.freshness().unwrap(), "fresh");
}

#[test]
fn shared_cache_child() {
    let Some(root) = std::env::var_os("KORTEX_ANALYSIS_CACHE_CHILD_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    std::fs::write(root.join("code.py"), "def cached():\n    pass\n").unwrap();
    std::fs::write(root.join(".kortex.toml"), "[index]\nstorage='user-cache'\n").unwrap();
    let config = Config::load(&root, None).unwrap();
    kortex_indexer::index_at_root(&root, &config, &Default::default()).unwrap();
    let service = Service::open(&root).unwrap();
    assert_eq!(service.config.index.storage, "user-cache");
    assert_eq!(service.freshness().unwrap(), "fresh");
    assert!(service.lookup("cached").is_ok());
    assert!(!root.join(".kortex").exists());
    assert!(
        service.store.directory.starts_with(
            PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap())
                .canonicalize()
                .unwrap()
        )
    );
}
#[test]
fn service_reads_the_same_configured_shared_cache_as_the_indexer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir(&root).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "intelligence_tests::inventory::shared_cache_child",
            "--nocapture",
        ])
        .env("KORTEX_ANALYSIS_CACHE_CHILD_ROOT", &root)
        .env(
            "KORTEX_REGISTRY",
            temp.path().join("private-registry/registry.db"),
        )
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn configured_local_search_is_refused_not_silently_replaced_with_keywords() {
    let (_temp, mut service) = indexed(&[("a.py", "def value():\n    pass\n")]);
    service.config.search.embeddings = "local".into();
    let error = service
        .execute("query", &json!({"text":"value"}))
        .unwrap_err();
    assert_eq!(error.code, "LOCAL_MODEL_GATE");
    assert_eq!(error.exit, 7);
}
