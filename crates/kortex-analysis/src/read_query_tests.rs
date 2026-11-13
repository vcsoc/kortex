use super::*;
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Config) {
    let temp = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "a.py",
            "def target():\n    return 1\ndef caller():\n    target()\nclass First:\n    def same(self):\n        pass\n",
        ),
        (
            "b.py",
            "class Second:\n    def same(self):\n        pass\ndef ÄLPHA():\n    pass\ndef İstanbul():\n    pass\n",
        ),
        (
            "doc.rs",
            "/// searchable documentation\npub fn incidental() {}\n",
        ),
    ] {
        std::fs::write(temp.path().join(path), source).unwrap();
    }
    let mut source = String::new();
    for index in 0..45 {
        source.push_str(&format!(
            "def item_{index:02}():\n    \"\"\"searchable documentation\"\"\"\n    pass\n"
        ));
    }
    std::fs::write(temp.path().join("Ärea.py"), source).unwrap();
    let config = Config::default();
    kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    (temp, config)
}
fn stable(mut response: Value) -> Value {
    response.as_object_mut().unwrap().remove("request_id");
    response.as_object_mut().unwrap().remove("timing_ms");
    response
}
fn compare(eager: &Service, lazy: &ReadService, args: &Value) -> Value {
    let expected = eager.execute("query", args);
    let actual = lazy.execute("query", args);
    match (expected, actual) {
        (Ok(expected), Ok(actual)) => {
            assert_eq!(stable(actual.clone()), stable(expected), "args={args}");
            jsonschema::validator_for(&response_schema("query"))
                .unwrap()
                .validate(&actual)
                .unwrap();
            actual
        }
        (Err(expected), Err(actual)) => {
            assert_eq!(
                (&actual.code, &actual.message, actual.exit),
                (&expected.code, &expected.message, expected.exit),
                "args={args}"
            );
            json!({"error":actual.code})
        }
        (expected, actual) => {
            panic!("eager/lazy mismatch: args={args}, eager={expected:?}, lazy={actual:?}")
        }
    }
}
fn pages(eager: &Service, lazy: &ReadService, mut args: Value) -> Vec<Value> {
    let mut all = vec![];
    for _ in 0..100 {
        let response = compare(eager, lazy, &args);
        all.extend(
            response["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        let cursor = &response["page"]["next_cursor"];
        if cursor.is_null() {
            return all;
        }
        args["cursor"] = cursor.clone();
    }
    panic!("pagination did not complete");
}

#[test]
fn actual_lazy_execute_matches_complete_eager_pages_filters_unicode_ambiguity_and_citations() {
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    for args in [
        json!({"text":"target","require_fresh":true}),
        json!({"text":"TARGET"}),
        json!({"text":"same","limit":1}),
        json!({"text":"First.same"}),
        json!({"text":"ärea","limit":7}),
        json!({"text":"älp"}),
        json!({"text":"i\u{307}"}),
        json!({"text":"searchable","limit":3}),
        json!({"text":"documentation","language":"rust"}),
        json!({"text":"item","kind":"function","path_prefix":"Ärea","language":"python","limit":4}),
        json!({"text":"item","path_prefix":"absent"}),
        json!({"text":"item","language":"go"}),
        json!({"text":"missing"}),
        json!({"text":"item","cursor":"100000"}),
    ] {
        pages(&eager, &lazy, args);
        assert!(
            !lazy.eager_materialized(),
            "query must not hide eager fallback"
        );
    }
    let response = compare(&eager, &lazy, &json!({"text":"target","limit":1}));
    let symbol = &response["data"]["items"][0]["symbol"];
    assert_eq!(symbol["file_path"], "a.py");
    assert_eq!(symbol["span"]["start_line"], 1);
    assert_eq!(symbol["span"]["start_byte"], 0);
    let original = eager
        .snapshot
        .symbol(symbol["symbol_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(symbol, &serde_json::to_value(original).unwrap());
    assert!(
        response["data"]["items"][0]["why_matched"]
            .as_array()
            .unwrap()
            .contains(&json!("exact_symbol"))
    );
}

#[test]
fn actual_lazy_execute_keeps_budgets_cursor_fingerprints_policy_and_control() {
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
    let unbounded = pages(&eager, &lazy, json!({"text":"item","limit":500}));
    let budgeted = pages(
        &eager,
        &lazy,
        json!({"text":"item","limit":20,"max_response_bytes":5000}),
    );
    assert_eq!(
        budgeted, unbounded,
        "budget repagination must not skip facts"
    );
    let first = compare(&eager, &lazy, &json!({"text":"item","limit":1}));
    let cursor = first["page"]["next_cursor"].clone();
    for args in [
        json!({"text":"item","limit":7,"cursor":cursor,"require_fresh":true}),
        json!({"text":"item","cursor":"1"}),
        json!({"text":"item","cursor":cursor,"kind":"function"}),
        json!({"text":"other","cursor":cursor}),
        json!({"text":"item","generation_id":"wrong"}),
        json!({"text":"item","cursor":"kx1.invalid.1"}),
        json!({"text":" "}),
        json!({"text":""}),
        json!({"text":"item","unknown":true}),
        json!({"text":"item","limit":0}),
        json!({"text":"item","max_response_bytes":1024}),
        json!({"text":"item","max_response_bytes":16777216}),
    ] {
        compare(&eager, &lazy, &args);
    }
    let parent = RequestControl::new(Duration::from_secs(20)).unwrap();
    parent.cancel();
    assert_eq!(
        lazy.execute_with_control("query", &json!({"text":"item"}), &parent)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let expired = RequestControl::from_deadline(Instant::now() - Duration::from_secs(1));
    assert_eq!(
        lazy.execute_with_control("query", &json!({"text":"item"}), &expired)
            .unwrap_err()
            .code,
        "TIMEOUT"
    );
    assert!(!lazy.eager_materialized());
    let mut gated = config;
    gated.search.embeddings = "local".into();
    // Config.validate requires complete local paths; mutate after opening as retained SDK tests do.
    let mut eager = eager;
    eager.config.search.embeddings = gated.search.embeddings.clone();
    let mut lazy = lazy;
    lazy.config.search.embeddings = gated.search.embeddings;
    assert_eq!(
        compare(&eager, &lazy, &json!({"text":"item"}))["error"],
        "LOCAL_MODEL_GATE"
    );
}

#[test]
fn parser_partial_and_stale_source_are_truthful_without_top_level_diagnostics() {
    let (temp, config) = fixture();
    std::fs::write(temp.path().join("bad.py"), "def broken(:\n    pass\n").unwrap();
    let report = kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    assert!(report.snapshot.diagnostics.is_empty());
    assert!(
        report
            .snapshot
            .files
            .iter()
            .any(|f| !f.diagnostics.is_empty())
    );
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    let response = compare(&eager, &lazy, &json!({"text":"target"}));
    assert_eq!(response["partial"], true);
    assert_eq!(response["index_status"], "fresh");
    std::fs::write(temp.path().join("a.py"), "def target():\n    return 2\n").unwrap();
    let response = compare(&eager, &lazy, &json!({"text":"target"}));
    assert_eq!(response["index_status"], "stale");
    assert_eq!(response["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(
        compare(
            &eager,
            &lazy,
            &json!({"text":"target","require_fresh":true})
        )["error"],
        "STALE_INDEX"
    );
    assert!(!lazy.eager_materialized());
}

#[test]
fn query_and_explicit_unsupported_fallback_remain_pinned_across_real_replacement() {
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
    let original_generation = eager.snapshot.generation_id.clone();
    std::fs::write(temp.path().join("new.py"), "def target():\n    return 99\n").unwrap();
    let replacement =
        kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    assert_ne!(replacement.snapshot.generation_id, original_generation);
    let response = compare(&eager, &lazy, &json!({"text":"target","limit":1}));
    assert_eq!(response["generation_id"], original_generation);
    assert_eq!(response["index_status"], "stale");
    assert!(!lazy.eager_materialized());
    let args = json!({"symbol":"a.py:target"});
    assert_eq!(
        stable(lazy.execute("context", &args).unwrap()),
        stable(eager.execute("context", &args).unwrap())
    );
    assert!(
        lazy.eager_materialized(),
        "context remains an explicit eager fallback, not claimed lazy"
    );
    compare(&eager, &lazy, &json!({"text":"new.py"}));
    let control = RequestControl::new(Duration::from_secs(10)).unwrap();
    assert_eq!(
        lazy.into_eager(&control).unwrap().snapshot.generation_id,
        original_generation
    );
}

#[test]
fn missing_legacy_query_projection_falls_back_on_same_store_and_never_guesses_diagnostics() {
    for legacy_metadata in [false, true] {
        let (temp, config) = fixture();
        std::fs::write(temp.path().join("bad.py"), "def broken(:\n    pass\n").unwrap();
        kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
        let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
        // A pre-projection generation has no new evidence, not orphan current rows.
        db.execute_batch(
            "BEGIN;
             DELETE FROM read_symbol_inventory;
             DELETE FROM read_edge_order;
             DELETE FROM read_file_diagnostics;
             DELETE FROM read_file_order;
             DELETE FROM generation_read_manifest;
             DELETE FROM query_symbol_inventory;
             DELETE FROM generation_query_manifest;
             COMMIT;",
        )
        .unwrap();
        if legacy_metadata {
            db.execute("DELETE FROM generation_metadata", []).unwrap();
        }
        drop(db);
        let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
        let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
        assert_eq!(
            lazy.metadata_source(),
            if legacy_metadata {
                kortex_storage::MetadataSource::LegacySnapshot
            } else {
                kortex_storage::MetadataSource::Projection
            }
        );
        std::fs::write(temp.path().join("new.py"), "def added():\n    pass\n").unwrap();
        kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
        pages(&eager, &lazy, json!({"text":"item","limit":7}));
        assert_eq!(
            compare(&eager, &lazy, &json!({"text":"target"}))["partial"],
            true
        );
        assert!(lazy.eager_materialized());
        assert_eq!(lazy.metadata().generation_id, eager.snapshot.generation_id);
    }
}

#[test]
fn orphan_current_projection_rows_are_not_downgraded_to_legacy_fallback() {
    let (temp, config) = fixture();
    let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
    db.execute("DELETE FROM generation_read_manifest", [])
        .unwrap();
    drop(db);
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    assert_eq!(
        lazy.execute("query", &json!({"text":"item"}))
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(!lazy.eager_materialized());
}

#[test]
fn actual_query_avoids_unselected_full_facts_and_refuses_corrupt_present_evidence() {
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
    db.execute(
        "UPDATE generations SET snapshot='invalid snapshot JSON'",
        [],
    )
    .unwrap();
    db.execute("UPDATE files SET facts_json='invalid parsed file JSON'", [])
        .unwrap();
    db.execute(
        "UPDATE nodes SET data_json='invalid unrelated symbol JSON' WHERE name='caller'",
        [],
    )
    .unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
    compare(
        &eager,
        &lazy,
        &json!({"text":"target","limit":1,"require_fresh":true}),
    );
    compare(&eager, &lazy, &json!({"text":"absent"}));
    assert!(
        !lazy.eager_materialized(),
        "actual operation, not just facade projection methods"
    );
    assert_eq!(
        lazy.execute("query", &json!({"text":"caller","limit":1}))
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(Service::open_at_root(temp.path(), config.clone()).is_err());
    db.execute(
        "DELETE FROM read_file_diagnostics WHERE id=(SELECT id FROM read_file_diagnostics LIMIT 1)",
        [],
    )
    .unwrap();
    let corrupt = ReadService::open_at_root(temp.path(), config).unwrap();
    assert_eq!(
        corrupt
            .execute("query", &json!({"text":"target"}))
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(
        !corrupt.eager_materialized(),
        "present corrupt evidence is not a legacy fallback"
    );
}

#[test]
#[ignore = "requires explicitly supplied freshly rebuilt KORTEX_TEST_CLI; execute separately"]
fn actual_cli_query_uses_lazy_dispatch_but_context_keeps_honest_eager_fallback() {
    let binary = std::env::var_os("KORTEX_TEST_CLI").expect("supply trusted rebuilt CLI path");
    let binary = PathBuf::from(binary).canonicalize().unwrap();
    eprintln!(
        "actual CLI BLAKE3 {} {}",
        hash(std::fs::read(&binary).unwrap()),
        binary.display()
    );
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config).unwrap();
    let expected = eager
        .execute(
            "query",
            &json!({"text":"target","limit":1,"require_fresh":true}),
        )
        .unwrap();
    let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
    db.execute(
        "UPDATE generations SET snapshot='invalid snapshot JSON'",
        [],
    )
    .unwrap();
    db.execute("UPDATE files SET facts_json='invalid parsed file JSON'", [])
        .unwrap();
    db.execute(
        "UPDATE nodes SET data_json='invalid unrelated symbol JSON' WHERE name='caller'",
        [],
    )
    .unwrap();
    assert!(db.execute("DELETE FROM read_edge_order", []).unwrap() > 0);
    drop(db);
    let output = std::process::Command::new(&binary)
        .arg("--repo")
        .arg(temp.path())
        .args([
            "--json",
            "--require-fresh",
            "query",
            "target",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stable(actual), stable(expected));
    let output = std::process::Command::new(&binary)
        .arg("--repo")
        .arg(temp.path())
        .args(["--json", "context", "a.py:target"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(6),
        "unsupported context must not fabricate an empty lazy result"
    );
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CORRUPT_INDEX");
}

#[test]
fn top_level_partial_classification_is_unchanged_for_clean_truncated_and_refused_messages() {
    let (temp, config) = fixture();
    let mut snapshot = Store::open(temp.path(), false).unwrap().load().unwrap();
    for (message, partial) in [
        ("ordinary informational diagnostic", false),
        ("inventory truncated by budget", true),
        ("unsafe input refused", true),
    ] {
        snapshot.diagnostics = vec![message.into()];
        snapshot.generation_id = hash(message);
        let mut writer = Store::open(temp.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&snapshot, &lock).unwrap();
        drop(lock);
        drop(writer);
        let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
        let lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
        assert_eq!(
            compare(&eager, &lazy, &json!({"text":"target"}))["partial"],
            partial
        );
        assert!(!lazy.eager_materialized());
    }
}

#[test]
fn exact_unicode_context_and_literal_wildcard_and_nul_errors_match_eager() {
    let (temp, config) = fixture();
    std::fs::write(
        temp.path().join("literal_%_.py"),
        "def ΟΣ():\n    pass\ndef Σ():\n    pass\n",
    )
    .unwrap();
    kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    for args in [
        json!({"text":"ος"}),
        json!({"text":"οσ"}),
        json!({"text":"σ"}),
        json!({"text":"%"}),
        json!({"text":"_"}),
        json!({"text":"ΟΣ","path_prefix":"literal_%_"}),
        json!({"text":"target","path_prefix":"\0"}),
        json!({"text":"target\0suffix"}),
        json!({"text":"target"}),
    ] {
        compare(&eager, &lazy, &args);
    }
    assert!(!lazy.eager_materialized());
}

#[test]
fn bounded_retry_page_cache_preserves_filters_offsets_limits_and_current_source_hashes() {
    let (temp, config) = fixture();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    for args in [
        json!({"text":"item","limit":20}),
        json!({"text":"item","limit":7}),
        json!({"text":"item","limit":1,"max_response_bytes":5000}),
        json!({"text":"item","limit":30}),
        json!({"text":"item","limit":3,"cursor":"1"}),
        json!({"text":"item","limit":2,"cursor":"1"}),
        json!({"text":"item","limit":7,"language":"rust"}),
        json!({"text":"item","limit":7,"kind":"method"}),
        json!({"text":"item","limit":7,"path_prefix":"b"}),
        json!({"text":"target","limit":20}),
        json!({"text":"target","limit":1}),
    ] {
        compare(&eager, &lazy, &args);
    }
    // Same-size edit after cache warmup: every retry still rereads/hashes current source.
    std::fs::write(temp.path().join("a.py"), "def target():\n    return 2\ndef caller():\n    target()\nclass First:\n    def same(self):\n        pass\n").unwrap();
    assert_eq!(
        compare(&eager, &lazy, &json!({"text":"target","limit":1}))["index_status"],
        "stale"
    );
    assert_eq!(
        compare(
            &eager,
            &lazy,
            &json!({"text":"target","limit":1,"require_fresh":true})
        )["error"],
        "STALE_INDEX"
    );
    assert!(!lazy.eager_materialized());
}

#[test]
fn immutable_cache_is_reader_owned_even_when_a_publication_reuses_generation_id() {
    let (temp, config) = fixture();
    let old_eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let old_lazy = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
    compare(&old_eager, &old_lazy, &json!({"text":"target","limit":20}));
    let original_generation = old_eager.snapshot.generation_id.clone();
    let mut replacement = old_eager.snapshot.clone();
    replacement.generation_id = hash("intermediate-before-generation-id-reuse");
    let file = replacement
        .files
        .iter_mut()
        .find(|file| file.path == "a.py")
        .unwrap();
    file.diagnostics.push("new per-file parser evidence".into());
    file.symbols
        .iter_mut()
        .find(|symbol| symbol.name == "target")
        .unwrap()
        .doc_comment = Some("new actual pinned publication fact".into());
    let mut writer = Store::open(temp.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&replacement, &lock).unwrap();
    // The intervening publication retired the original primary key. Reusing this
    // caller-supplied ID is legal, but cannot authorize a cross-reader cache hit.
    replacement.generation_id = original_generation;
    writer.commit(&replacement, &lock).unwrap();
    drop(lock);
    drop(writer);
    let new_eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let new_lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    assert_eq!(
        new_lazy.metadata().generation_id,
        old_lazy.metadata().generation_id
    );
    let old = compare(&old_eager, &old_lazy, &json!({"text":"target","limit":1}));
    let new = compare(&new_eager, &new_lazy, &json!({"text":"target","limit":1}));
    assert_eq!(old["partial"], false);
    assert_eq!(new["partial"], true);
    assert_ne!(old["data"]["items"], new["data"]["items"]);
    assert!(!new_lazy.eager_materialized());
    assert!(!old_lazy.eager_materialized());
}

#[test]
fn freshness_preserves_arbitrary_snapshot_file_order_instead_of_silently_sorting() {
    let (temp, config) = fixture();
    let mut snapshot = Store::open(temp.path(), false).unwrap().load().unwrap();
    snapshot.files.reverse();
    snapshot.edges.reverse();
    snapshot.generation_id = hash("reversed-real-fixture");
    let mut writer = Store::open(temp.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&snapshot, &lock).unwrap();
    drop(lock);
    drop(writer);
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    let response = compare(&eager, &lazy, &json!({"text":"target"}));
    assert_eq!(
        response["index_status"], "stale",
        "legacy eager compares file vector order exactly"
    );
    assert!(!lazy.eager_materialized());
}
