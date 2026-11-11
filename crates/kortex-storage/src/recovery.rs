use super::*;
use std::{
    process::Command,
    time::{Duration, Instant},
};
#[cfg(unix)]
#[test]
fn symlinked_database_sidecars_are_refused() {
    for name in ["index.db", "index.db-wal", "index.db-shm", "writer.lock"] {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::create_dir(d.path().join(".kortex")).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join(".kortex").join(name)).unwrap();
        match Store::open(d.path(), true) {
            Ok(_) => panic!("symlink accepted: {name}"),
            Err(e) => assert_eq!(e.exit, 8),
        }
    }
}
#[test]
fn crash_child() {
    let Some(root) = std::env::var_os("KORTEX_CRASH_TEST_ROOT") else {
        return;
    };
    let store = Store::open(Path::new(&root), true).unwrap();
    let _lock = store.writer_lock().unwrap();
    store.conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO generations(id,repo_id,status,config_hash,grammar_hash,started_at,snapshot) VALUES('interrupted','repo','building','','',datetime('now'),'{}');").unwrap();
    std::fs::write(Path::new(&root).join("transaction-ready"), "ready").unwrap();
    std::thread::sleep(Duration::from_secs(60));
    panic!("parent did not kill crash worker");
}
#[test]
fn killed_writer_preserves_last_generation_and_releases_lock() {
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store
        .commit(
            &Snapshot {
                repo_id: "repo".into(),
                root: d.path().display().to_string(),
                generation_id: "good".into(),
                ..Snapshot::default()
            },
            &lock,
        )
        .unwrap();
    drop(lock);
    drop(store);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "recovery::crash_child", "--nocapture"])
        .env("KORTEX_CRASH_TEST_ROOT", d.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !d.path().join("transaction-ready").exists() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("worker did not enter transaction");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let store = Store::open(d.path(), true).unwrap();
    assert_eq!(store.load().unwrap().generation_id, "good");
    assert_eq!(store.integrity().unwrap(), "ok");
    assert!(store.writer_lock().is_ok());
    let count: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM generations WHERE id='interrupted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
