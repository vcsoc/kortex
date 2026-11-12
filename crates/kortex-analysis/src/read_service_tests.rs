use super::*;
use kortex_storage::{MetadataSource, SymbolFilter};
use std::time::Duration;

#[test]
fn real_projection_reader_does_not_decode_snapshot_or_unselected_symbol_facts() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("a.py"),
        "def target():\n    pass\ndef caller():\n    target()\n",
    )
    .unwrap();
    let config = Config::default();
    let report = kortex_indexer::index_at_root(temp.path(), &config, &Default::default()).unwrap();
    let symbol = report
        .snapshot
        .symbols()
        .find(|s| s.name == "target")
        .unwrap();
    let target = symbol.symbol_id.clone();
    let db = rusqlite::Connection::open(temp.path().join(".kortex/index.db")).unwrap();
    db.execute(
        "UPDATE generations SET snapshot='invalid snapshot JSON'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET data_json='invalid unrelated fact JSON' WHERE name='caller'",
        [],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        Service::open_at_root(temp.path(), config.clone())
            .err()
            .unwrap()
            .code,
        "CORRUPT_INDEX"
    );
    let read = ReadService::open_at_root(temp.path(), config).unwrap();
    assert_eq!(read.metadata_source(), MetadataSource::Projection);
    assert_eq!(read.metadata().generation_id, report.snapshot.generation_id);
    let control = RequestControl::new(Duration::from_secs(10)).unwrap();
    assert_eq!(read.file_inventory(&control).unwrap().len(), 1);
    assert_eq!(
        serde_json::to_value(read.get_symbol(&target, &control).unwrap().unwrap()).unwrap(),
        serde_json::to_value(symbol).unwrap()
    );
    let page = read
        .symbol_candidates(
            &SymbolFilter {
                name: Some("target"),
                ..Default::default()
            },
            None,
            1,
            &control,
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.next_after_id.is_none());
    assert!(
        read.incoming_edges(&target, None, 100, &control)
            .unwrap()
            .items
            .iter()
            .any(|e| e.kind == "CALLS")
    );
    assert!(!read.keyword("target", 10, &control).unwrap().is_empty());
    assert!(read.outgoing_edges(&target, None, 0, &control).is_err());
    control.cancel();
    assert_eq!(
        read.get_symbol(&target, &control).unwrap_err().code,
        "CANCELLED"
    );
    let active = RequestControl::new(Duration::from_secs(10)).unwrap();
    assert_eq!(
        read.into_eager(&active).err().unwrap().code,
        "CORRUPT_INDEX"
    );
}

#[test]
fn lazy_rows_and_explicit_eager_fallback_keep_the_original_pinned_generation() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("a.py");
    std::fs::write(&file, "def target():\n    return 1\n").unwrap();
    let config = Config::default();
    let before = kortex_indexer::index_at_root(temp.path(), &config, &Default::default())
        .unwrap()
        .snapshot;
    let target = before
        .symbols()
        .find(|s| s.name == "target")
        .unwrap()
        .clone();
    let read = ReadService::open_at_root(temp.path(), config.clone()).unwrap();
    std::fs::write(
        &file,
        "def target():\n    return 2\ndef added():\n    target()\n",
    )
    .unwrap();
    let after = kortex_indexer::index_at_root(temp.path(), &config, &Default::default())
        .unwrap()
        .snapshot;
    assert_ne!(before.generation_id, after.generation_id);
    let control = RequestControl::new(Duration::from_secs(10)).unwrap();
    assert_eq!(read.metadata().generation_id, before.generation_id);
    assert_eq!(
        serde_json::to_value(
            read.get_symbol(&target.symbol_id, &control)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(target).unwrap()
    );
    assert!(read.keyword("added", 10, &control).unwrap().is_empty());
    let current = ReadService::open_at_root(temp.path(), config).unwrap();
    assert_eq!(current.metadata().generation_id, after.generation_id);
    assert!(!current.keyword("added", 10, &control).unwrap().is_empty());
    let eager = read.into_eager(&control).unwrap();
    assert_eq!(
        serde_json::to_value(eager.snapshot).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}
