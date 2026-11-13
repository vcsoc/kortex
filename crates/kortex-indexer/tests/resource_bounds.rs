//! Small inert real-source fixtures only: parsed/indexed, never compiled or executed.
use kortex_config::Config;
use kortex_core::Snapshot;
use kortex_indexer::{Options, index_at_root};
use kortex_languages::{ResolutionContext, ResolutionLimits, resolve_bounded_with_context};
use kortex_storage::{Store, registry::Registry};
use std::{fs, path::Path};

fn go_source(definitions: usize, callers: usize) -> String {
    let mut source = "package fixture\n".to_owned();
    for _ in 0..definitions {
        source.push_str("func duplicate() {}\n");
    }
    for i in 0..callers {
        source.push_str(&format!("func caller{i}() {{ duplicate() }}\n"));
    }
    source
}
fn limits() -> ResolutionLimits {
    let c = Config::default().index.resources;
    ResolutionLimits {
        max_edges: c.max_edges,
        max_candidate_memberships: c.max_candidate_memberships,
        max_edge_bytes: c.max_edge_bytes,
        max_imports: c.max_imports,
        max_fact_bytes: c.max_parsed_bytes,
    }
}
fn snapshot(source: &str) -> Snapshot {
    Snapshot {
        repo_id: "resource-source".into(),
        files: vec![kortex_parse::parse("resource-source", "main.go", "go", source).unwrap()],
        ..Default::default()
    }
}
fn error(result: kortex_core::Result<kortex_indexer::Report>) -> kortex_core::Error {
    match result {
        Err(e) => e,
        Ok(_) => panic!("resource-limited attempt unexpectedly succeeded"),
    }
}

#[test]
fn real_candidate_membership_exact_boundary_plus_one_and_oracle_equivalence() {
    let s = snapshot(&go_source(3, 3));
    assert_eq!(s.files[0].references.len(), 3);
    let oracle = kortex_languages::resolve_with_context(&s, &ResolutionContext::default());
    let calls: Vec<_> = oracle.iter().filter(|e| e.kind == "CALLS").collect();
    assert_eq!(calls.len(), 3);
    assert!(
        calls.iter().all(|e| e.target_id.is_none()
            && e.resolution == "ambiguous"
            && e.candidates.len() == 3)
    );
    let memberships = calls.iter().map(|e| e.candidates.len()).sum::<usize>();
    assert_eq!(memberships, 9);
    let mut l = limits();
    l.max_candidate_memberships = memberships;
    assert_eq!(
        resolve_bounded_with_context(&s, &Default::default(), &l).unwrap(),
        oracle
    );
    l.max_candidate_memberships -= 1;
    let e = resolve_bounded_with_context(&s, &Default::default(), &l).unwrap_err();
    assert_eq!(e.code, "RESOURCE_LIMIT");
    assert!(e.message.contains("attempted 9"));
    // The same call shape at a second actual source size, not fabricated candidate arrays.
    let larger = snapshot(&go_source(4, 4));
    l.max_candidate_memberships = 16;
    let edges = resolve_bounded_with_context(&larger, &Default::default(), &l).unwrap();
    assert_eq!(edges.iter().map(|e| e.candidates.len()).sum::<usize>(), 16);
    l.max_candidate_memberships = 15;
    assert_eq!(
        resolve_bounded_with_context(&larger, &Default::default(), &l)
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}

#[test]
fn real_edge_count_and_exact_serialized_byte_boundaries() {
    let s = snapshot(&go_source(3, 3));
    let oracle = kortex_languages::resolve(&s, None);
    let bytes = oracle
        .iter()
        .map(|e| serde_json::to_vec(e).unwrap().len())
        .sum::<usize>();
    let mut l = limits();
    l.max_edges = oracle.len();
    l.max_edge_bytes = bytes;
    assert_eq!(
        resolve_bounded_with_context(&s, &Default::default(), &l).unwrap(),
        oracle
    );
    l.max_edge_bytes = bytes - 1;
    assert!(
        resolve_bounded_with_context(&s, &Default::default(), &l)
            .unwrap_err()
            .message
            .contains("edge JSON bytes")
    );
    l.max_edge_bytes = bytes;
    l.max_edges -= 1;
    assert!(
        resolve_bounded_with_context(&s, &Default::default(), &l)
            .unwrap_err()
            .message
            .contains("edges")
    );
}

#[test]
fn worker_aggregate_boundaries_include_cached_facts_and_retry() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..8 {
        fs::write(d.path().join(format!("part{i}.go")), format!("package fixture\nimport \"unknown/a\"\nimport \"unknown/b\"\nfunc leaf{i}(){{}}\nfunc entry{i}(){{leaf{i}(); leaf{i}()}}\n")).unwrap();
    }
    let initial = index_at_root(d.path(), &Config::default(), &Options::default()).unwrap();
    let symbols = initial.snapshot.files.iter().map(|f| f.symbols.len()).sum();
    let references = initial
        .snapshot
        .files
        .iter()
        .map(|f| f.references.len())
        .sum();
    let imports = initial.snapshot.files.iter().map(|f| f.imports.len()).sum();
    let bytes = initial
        .snapshot
        .files
        .iter()
        .map(|f| serde_json::to_vec(f).unwrap().len())
        .sum();
    let mut c = Config::default();
    c.index.resources.max_symbols = symbols;
    c.index.resources.max_references = references;
    c.index.resources.max_imports = imports;
    c.index.resources.max_parsed_bytes = bytes;
    assert_eq!(c.fingerprint().unwrap(), initial.snapshot.config_hash);
    for workers in [1, 4] {
        c.index.workers = workers;
        // workers historically participate in the semantic fingerprint; establish this cache first.
        let full = index_at_root(
            d.path(),
            &c,
            &Options {
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
        let reused = index_at_root(d.path(), &c, &Options::default()).unwrap();
        assert_eq!(reused.reused, 8);
        assert_eq!(full.snapshot.files, initial.snapshot.files);
        assert_eq!(full.snapshot.edges, initial.snapshot.edges);
        for dimension in ["symbols", "references", "imports", "bytes"] {
            let mut lower = c.clone();
            match dimension {
                "symbols" => lower.index.resources.max_symbols -= 1,
                "references" => lower.index.resources.max_references -= 1,
                "imports" => lower.index.resources.max_imports -= 1,
                _ => lower.index.resources.max_parsed_bytes -= 1,
            }
            // Both cached and freshly parsed worker paths must refuse the same aggregate.
            for full in [false, true] {
                let e = error(index_at_root(
                    d.path(),
                    &lower,
                    &Options {
                        full,
                        ..Default::default()
                    },
                ));
                assert_eq!(
                    e.code, "RESOURCE_LIMIT",
                    "{workers}/{dimension}/{full}: {e}"
                );
                assert_eq!(
                    Store::open(d.path(), false)
                        .unwrap()
                        .load()
                        .unwrap()
                        .generation_id,
                    reused.snapshot.generation_id
                );
            }
        }
        let retry = index_at_root(d.path(), &c, &Options::default()).unwrap();
        assert_eq!(retry.reused, 8);
        assert_eq!(retry.snapshot.edges, initial.snapshot.edges);
    }
}

#[test]
fn malformed_and_nested_source_refuse_before_extraction_but_control_succeeds() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("main.rs");
    let mut c = Config::default();
    c.index.resources.max_file_ast_depth = 12;
    c.index.resources.max_file_ast_nodes = 80;
    for source in [
        format!("fn nested(){{{}0{}}}", "(".repeat(40), ")".repeat(40)),
        format!(
            "fn broken(){{{}0{}}}\nfn invalid( {{",
            "(".repeat(40),
            ")".repeat(40)
        ),
    ] {
        fs::write(&path, source).unwrap();
        assert_eq!(
            error(index_at_root(
                d.path(),
                &c,
                &Options {
                    dry_run: true,
                    ..Default::default()
                }
            ))
            .code,
            "RESOURCE_LIMIT"
        );
        assert!(!d.path().join(".kortex").exists());
    }
    // A shallow malformed ERROR node is permitted with truthful syntax diagnostics.
    fs::write(&path, "fn broken() { (((").unwrap();
    let malformed = index_at_root(
        d.path(),
        &c,
        &Options {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        malformed.snapshot.files[0]
            .diagnostics
            .iter()
            .any(|d| d.contains("syntax errors"))
    );
    fs::write(&path, "fn leaf(){} fn control(){leaf();}").unwrap();
    let ok = index_at_root(d.path(), &c, &Options::default()).unwrap();
    assert!(
        ok.snapshot
            .edges
            .iter()
            .any(|e| e.kind == "CALLS" && e.resolution == "exact")
    );
}

#[test]
fn production_worker_admits_near_depth_cap_on_declared_stack() {
    let d = tempfile::tempdir().unwrap();
    let source = format!(
        "fn leaf(){{}} fn control(){{{}leaf(){}}}",
        "(".repeat(180),
        ")".repeat(180)
    );
    fs::write(d.path().join("main.rs"), source).unwrap();
    let result = index_at_root(d.path(), &Config::default(), &Options::default()).unwrap();
    assert!(
        result
            .snapshot
            .edges
            .iter()
            .any(|e| e.kind == "CALLS" && e.resolution == "exact")
    );
}

#[test]
fn alias_import_expansion_is_fail_closed_and_matches_unbounded_oracle() {
    let s = Snapshot {
        repo_id: "alias-budget".into(),
        files: [
            ("main.ts", "import { leaf } from '@lib'; leaf();"),
            ("a.ts", "export function leaf(){}"),
            ("b.ts", "export function leaf(){}"),
        ]
        .into_iter()
        .map(|(p, source)| kortex_parse::parse("alias-budget", p, "typescript", source).unwrap())
        .collect(),
        ..Default::default()
    };
    let context = ResolutionContext::default()
        .with_tsconfig(r#"{"compilerOptions":{"paths":{"@lib":["a","b"]}}}"#)
        .unwrap();
    let oracle = kortex_languages::resolve_with_context(&s, &context);
    let mut l = limits();
    l.max_imports = 2;
    assert_eq!(
        resolve_bounded_with_context(&s, &context, &l).unwrap(),
        oracle
    );
    assert!(
        oracle
            .iter()
            .any(|e| e.kind == "CALLS" && e.resolution == "ambiguous" && e.candidates.len() == 2)
    );
    l.max_imports = 1;
    assert!(
        resolve_bounded_with_context(&s, &context, &l)
            .unwrap_err()
            .message
            .contains("transformed imports")
    );
    l.max_imports = 2;
    l.max_fact_bytes = 1;
    assert!(
        resolve_bounded_with_context(&s, &context, &l)
            .unwrap_err()
            .message
            .contains("alias workspace")
    );
}

#[test]
fn reference_fixture_policy_model_uses_real_parsed_two_file_shape() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..2 {
        let mut source = format!("from m{:05} import f0 as next_call\n", (i + 1) % 2);
        for j in 0..10 {
            source.push_str(&format!("def f{j}():\n    next_call()\n"));
        }
        fs::write(d.path().join(format!("m{i:05}.py")), source).unwrap();
    }
    let report = index_at_root(
        d.path(),
        &Config::default(),
        &Options {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    let symbols = report
        .snapshot
        .files
        .iter()
        .map(|f| f.symbols.len())
        .sum::<usize>();
    let references = report
        .snapshot
        .files
        .iter()
        .map(|f| f.references.len())
        .sum::<usize>();
    let imports = report
        .snapshot
        .files
        .iter()
        .map(|f| f.imports.len())
        .sum::<usize>();
    let parsed_bytes = report
        .snapshot
        .files
        .iter()
        .map(|f| serde_json::to_vec(f).unwrap().len())
        .sum::<usize>();
    let edge_bytes = report
        .snapshot
        .edges
        .iter()
        .map(|e| serde_json::to_vec(e).unwrap().len())
        .sum::<usize>();
    assert_eq!(
        (symbols, references, imports, report.snapshot.edges.len()),
        (22, 20, 2, 64)
    );
    let caps = Config::default().index.resources;
    // All paths/module numbers in the fixed 10k fixture have the same five-digit width;
    // IDs/hashes are fixed-width. This is an algebraic policy model, NOT a scale run.
    for (measured, cap) in [
        (symbols, caps.max_symbols),
        (references, caps.max_references),
        (imports, caps.max_imports),
        (parsed_bytes, caps.max_parsed_bytes),
        (report.snapshot.edges.len(), caps.max_edges),
        (references, caps.max_candidate_memberships),
        (edge_bytes, caps.max_edge_bytes),
    ] {
        assert!(measured * 5_000 <= cap);
    }
    println!(
        "{}",
        serde_json::json!({"scope":"actual two-file parse/index; algebraic 10k-file policy model, not measured scale or RSS/performance acceptance", "measured_files":2, "measured_functions":20, "measured_symbols":symbols, "measured_references":references, "measured_imports":imports, "measured_parsed_json_bytes":parsed_bytes, "measured_edges":report.snapshot.edges.len(), "measured_edge_json_bytes":edge_bytes, "model_repeat_factor":5000})
    );
}

#[test]
fn resource_publication_child() {
    let Some(path) = std::env::var_os("KORTEX_RESOURCE_TEST_ROOT") else {
        return;
    };
    let root = Path::new(&path);
    let source = root.join("main.go");
    fs::write(&source, go_source(3, 3)).unwrap();
    let mut c = Config {
        origin: Some(kortex_config::SelectionOrigin {
            invocation_root: root.canonicalize().unwrap(),
            explicit_config: None,
        }),
        ..Default::default()
    };
    c.index.resources.max_candidate_memberships = 9;
    let first = index_at_root(root, &c, &Options::default()).unwrap();
    let generation = first.snapshot.generation_id.clone();
    let committed = serde_json::to_vec(&first.snapshot).unwrap();
    let registry = Registry::open().unwrap();
    let entries = registry.list().unwrap();
    let selection = registry.selection(&first.snapshot.repo_id).unwrap();
    drop(registry);
    let registry_path = Registry::path().unwrap();
    let registry_bytes = fs::read(&registry_path).unwrap();
    let db_path = root.join(".kortex/index.db");
    let db_bytes = fs::read(&db_path).unwrap();
    let pinned = Store::open(root, false).unwrap();
    let prior_fts = pinned.keyword(&generation, "duplicate", 100).unwrap();
    assert!(!prior_fts.is_empty());
    for dimension in [
        "memberships",
        "symbols",
        "references",
        "bytes",
        "ast",
        "changed-source",
    ] {
        let mut lower = c.clone();
        match dimension {
            "symbols" => {
                lower.index.resources.max_symbols = first.snapshot.files[0].symbols.len() - 1
            }
            "references" => {
                lower.index.resources.max_references = first.snapshot.files[0].references.len() - 1
            }
            "bytes" => {
                lower.index.resources.max_parsed_bytes =
                    serde_json::to_vec(&first.snapshot.files[0]).unwrap().len() - 1
            }
            "ast" => lower.index.resources.max_file_ast_nodes = 1,
            "changed-source" => fs::write(&source, go_source(3, 4)).unwrap(),
            _ => lower.index.resources.max_candidate_memberships = 8,
        }
        let failure = error(index_at_root(
            root,
            &lower,
            &Options {
                full: dimension == "ast",
                ..Default::default()
            },
        ));
        assert_eq!((failure.code, failure.exit), ("RESOURCE_LIMIT", 9));
        assert_eq!(fs::read(&db_path).unwrap(), db_bytes);
        assert_eq!(fs::read(&registry_path).unwrap(), registry_bytes);
        assert_eq!(
            pinned.keyword(&generation, "duplicate", 100).unwrap(),
            prior_fts
        );
        let restart = Store::open(root, false).unwrap();
        let loaded = restart.load().unwrap();
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), committed);
        assert_eq!(
            restart.keyword(&generation, "duplicate", 100).unwrap(),
            prior_fts
        );
        let registry = Registry::open().unwrap();
        assert_eq!(registry.list().unwrap(), entries);
        assert_eq!(
            registry.selection(&first.snapshot.repo_id).unwrap(),
            selection
        );
    }
    fs::write(&source, go_source(2, 2)).unwrap();
    let retry = index_at_root(root, &c, &Options::default()).unwrap();
    assert_ne!(retry.snapshot.generation_id, generation);
    assert_eq!(pinned.load().unwrap().generation_id, generation);
    assert_eq!(
        Store::open(root, false)
            .unwrap()
            .load()
            .unwrap()
            .generation_id,
        retry.snapshot.generation_id
    );
    assert_eq!(
        Registry::open().unwrap().list().unwrap()[0].2,
        retry.snapshot.generation_id
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [&db_path, &registry_path] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        for directory in [
            root.join(".kortex"),
            registry_path.parent().unwrap().to_owned(),
        ] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}

#[test]
fn real_prior_index_registry_fts_bytes_restart_and_private_retry() {
    let d = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resource_publication_child", "--nocapture"])
        .env("KORTEX_RESOURCE_TEST_ROOT", d.path())
        .env(
            "KORTEX_REGISTRY",
            d.path().join("private-registry/registry.db"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
