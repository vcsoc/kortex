use super::*;

#[test]
fn actual_query_scope_never_authorizes_full_edge_validation_on_the_same_store() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("scope.py"),
        "def target():\n    return 1\ndef caller():\n    return target()\n",
    )
    .unwrap();
    let config = Config::default();
    kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
    let removed = db.execute("DELETE FROM read_edge_order", []).unwrap();
    assert!(removed > 0, "exercise actual parser/resolver edge evidence");
    drop(db);
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    let args = json!({"text":"target","limit":1,"require_fresh":true});
    let mut expected = eager.execute("query", &args).unwrap();
    let mut actual = lazy.execute("query", &args).unwrap();
    for response in [&mut expected, &mut actual] {
        response.as_object_mut().unwrap().remove("request_id");
        response.as_object_mut().unwrap().remove("timing_ms");
    }
    assert_eq!(actual, expected);
    assert!(!lazy.eager_materialized());
    assert_eq!(lazy.freshness().unwrap(), "fresh");
    assert_eq!(
        lazy.with_store(|store| store.read_session()?.read_projection_source())
            .unwrap_err()
            .code,
        "CORRUPT_INDEX",
        "query approval cannot seed full approval on the SAME pinned Store"
    );
    let mut retry = lazy.execute("query", &args).unwrap();
    retry.as_object_mut().unwrap().remove("request_id");
    retry.as_object_mut().unwrap().remove("timing_ms");
    assert_eq!(retry, expected);
    assert!(!lazy.eager_materialized());
}

#[test]
fn retry_cache_has_a_byte_cap_even_when_repository_allows_larger_responses() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = String::new();
    for index in 0..100 {
        source.push_str(&format!(
            "/// {}\npub fn bounded_{index}() {{}}\n",
            "x".repeat(4000)
        ));
    }
    std::fs::write(temp.path().join("cache.rs"), source).unwrap();
    let mut config = Config::default();
    config.mcp.max_response_bytes = 1024 * 1024;
    kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    let eager = Service::open_at_root(temp.path(), config.clone()).unwrap();
    let lazy = ReadService::open_at_root(temp.path(), config).unwrap();
    let args = json!({"text":"bounded_","limit":100});
    let expected = eager.execute("query", &args).unwrap();
    let actual = lazy.execute("query", &args).unwrap();
    assert_eq!(expected["data"], actual["data"]);
    assert!(serde_json::to_vec(&actual["data"]["items"]).unwrap().len() > QUERY_PAGE_CACHE_BYTES);
    assert!(
        lazy.query_page.borrow().is_none(),
        "do not retain an oversized retry page"
    );
    let args = json!({"text":"bounded_","limit":1});
    let actual = lazy.execute("query", &args).unwrap();
    let cached = lazy.query_page.borrow();
    let cached = cached
        .as_ref()
        .expect("a small bounded retry page is retained");
    assert_eq!(
        serde_json::to_value(&cached.items).unwrap(),
        actual["data"]["items"]
    );
    assert!(serde_json::to_vec(&cached.items).unwrap().len() <= QUERY_PAGE_CACHE_BYTES);
    assert_eq!(cached.limit, 1);
    assert!(!lazy.eager_materialized());
}
