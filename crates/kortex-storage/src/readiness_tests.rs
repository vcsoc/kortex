use super::*;
use std::process::Command;
fn sample(id: &str) -> Snapshot {
    Snapshot {
        repo_id: "repository".into(),
        root: "/sample".into(),
        generation_id: id.into(),
        ..Snapshot::default()
    }
}
#[test]
fn legacy_migration_backs_up_consistent_last_good_and_records_versions() {
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&sample("good"), &lock).unwrap();
    store
        .conn
        .execute_batch(
            "PRAGMA user_version=0; DELETE FROM schema_migrations; DROP TABLE storage_versions;",
        )
        .unwrap();
    drop(lock);
    // Keep the old connection alive so committed pages remain in WAL during backup.
    let legacy = store;
    assert!(
        fs::metadata(legacy.directory.join("index.db-wal"))
            .unwrap()
            .len()
            > 32
    );
    let main_header = fs::read(legacy.directory.join("index.db")).unwrap();
    assert_eq!(
        u32::from_be_bytes(main_header[60..64].try_into().unwrap()),
        1
    );
    let store = Store::open(d.path(), true).unwrap();
    assert_eq!(store.load().unwrap().generation_id, "good");
    let version: String = store
        .conn
        .query_row(
            "SELECT value FROM storage_versions WHERE key='schema'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, "1");
    let backups: Vec<_> = fs::read_dir(&store.directory)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("migration-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let original = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        original
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        original
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        original
            .query_row(
                "SELECT snapshot FROM generations WHERE id='good'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap()
            .contains("good")
    );
    drop(legacy);
    drop(store);
    assert_eq!(
        Store::open(d.path(), false)
            .unwrap()
            .load()
            .unwrap()
            .generation_id,
        "good"
    );
}
#[test]
fn migration_failure_rolls_back_and_unrecognized_schema_is_not_adopted() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), true).unwrap();
    store.conn.execute_batch("PRAGMA user_version=0; DELETE FROM schema_migrations; CREATE TRIGGER deny_migration BEFORE INSERT ON schema_migrations BEGIN SELECT RAISE(ABORT,'migration denied'); END;").unwrap();
    drop(store);
    assert!(Store::open(d.path(), true).is_err());
    let c = Connection::open(d.path().join(".kortex/index.db")).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(c);
    let other = tempfile::tempdir().unwrap();
    fs::create_dir(other.path().join(".kortex")).unwrap();
    let c = Connection::open(other.path().join(".kortex/index.db")).unwrap();
    c.execute_batch("CREATE TABLE unrelated(secret TEXT); INSERT INTO unrelated VALUES('keep');")
        .unwrap();
    drop(c);
    assert_eq!(
        Store::open(other.path(), true).err().unwrap().code,
        "INDEX_VERSION"
    );
}
#[test]
fn actual_sqlite_full_rolls_back_publication_and_restart() {
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&sample("good"), &lock).unwrap();
    let pages: i64 = store
        .conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    store
        .conn
        .pragma_update(None, "max_page_count", pages + 1)
        .unwrap();
    let mut too_large = sample("full");
    too_large.root = "x".repeat(4 * 1024 * 1024);
    let error = store.commit(&too_large, &lock).unwrap_err();
    assert_eq!(error.code, "STORAGE_ERROR");
    assert!(error.message.to_lowercase().contains("full"), "{error}");
    assert_eq!(store.load().unwrap().generation_id, "good");
    assert_eq!(store.integrity().unwrap(), "ok");
    drop(lock);
    drop(store);
    assert_eq!(
        Store::open(d.path(), false)
            .unwrap()
            .load()
            .unwrap()
            .generation_id,
        "good"
    );
}
#[test]
fn repair_preserves_original_sidecars_ownership_and_never_source_or_config() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join(".kortex");
    fs::create_dir(&dir).unwrap();
    fs::write(d.path().join("source.rs"), "fn source(){}").unwrap();
    fs::write(d.path().join(".kortex.toml"), "schema_version=1").unwrap();
    fs::write(dir.join("config.toml"), "retain=true").unwrap();
    fs::write(dir.join("index.db.bak"), "previous backup").unwrap();
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        fs::write(dir.join(name), format!("original evidence {name}")).unwrap();
    }
    #[cfg(unix)]
    let before: Vec<_> = {
        use std::os::unix::fs::MetadataExt;
        ["index.db", "index.db-wal", "index.db-shm"]
            .map(|name| {
                let m = fs::metadata(dir.join(name)).unwrap();
                (name, m.ino(), m.uid(), m.mode())
            })
            .into_iter()
            .collect()
    };
    let report = Store::repair(d.path(), "repository").unwrap();
    assert_eq!(report.preserved_files.len(), 3);
    for name in &report.preserved_files {
        assert_eq!(
            fs::read_to_string(report.backup_directory.join(name)).unwrap(),
            format!("original evidence {name}")
        );
    }
    #[cfg(unix)]
    for (name, inode, owner, mode) in before {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(report.backup_directory.join(name)).unwrap();
        assert_eq!((m.ino(), m.uid(), m.mode()), (inode, owner, mode));
    }
    for (path, expected) in [
        (d.path().join("source.rs"), "fn source(){}"),
        (d.path().join(".kortex.toml"), "schema_version=1"),
        (dir.join("config.toml"), "retain=true"),
        (dir.join("index.db.bak"), "previous backup"),
    ] {
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
    let mut rebuilt = Store::open(d.path(), true).unwrap();
    let lock = rebuilt.writer_lock().unwrap();
    rebuilt.commit(&sample("rebuilt"), &lock).unwrap();
    assert_eq!(
        Store::repair(d.path(), "repository").unwrap_err().code,
        "WRITER_BUSY"
    );
    drop(lock);
    drop(rebuilt);
    assert_eq!(
        Store::open(d.path(), false)
            .unwrap()
            .load()
            .unwrap()
            .generation_id,
        "rebuilt"
    );
}
#[cfg(unix)]
#[test]
fn actual_directory_permission_failures_and_hardlinks_are_safe() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join(".kortex");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(Store::open(d.path(), true).is_err());
        assert!(!dir.join("index.db").exists());
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = d.path().join("source.rs");
    fs::write(&outside, "source").unwrap();
    fs::hard_link(&outside, dir.join("index.db")).unwrap();
    assert_eq!(
        Store::open(d.path(), true).err().unwrap().code,
        "PATH_REFUSED"
    );
    assert_eq!(
        Store::repair(d.path(), "repository").unwrap_err().code,
        "PATH_REFUSED"
    );
    assert_eq!(fs::read_to_string(outside).unwrap(), "source");
}
#[test]
fn user_cache_child() {
    let Some(root) = std::env::var_os("KORTEX_TEST_CACHE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let a = root.join("a");
    let b = root.join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut store = Store::open_with_storage(&a, "user-cache", true).unwrap();
    assert!(!a.join(".kortex").exists());
    assert_ne!(
        store.directory,
        Store::directory_for(&b, "user-cache").unwrap()
    );
    assert_eq!(
        store.directory,
        Store::directory_for(&a.join("."), "user-cache").unwrap()
    );
    let lock = store.writer_lock().unwrap();
    assert_eq!(
        Store::open_with_storage(&a, "user-cache", true)
            .err()
            .unwrap()
            .code,
        "WRITER_BUSY"
    );
    let other = Store::open_with_storage(&b, "user-cache", true).unwrap();
    let _other_lock = other.writer_lock().unwrap();
    store.commit(&sample("cache-good"), &lock).unwrap();
    drop(lock);
    drop(store);
    assert_eq!(
        Store::open_with_storage(&a, "user-cache", false)
            .unwrap()
            .load()
            .unwrap()
            .generation_id,
        "cache-good"
    );
}
#[test]
fn user_cache_stable_identity_locking_restart_in_isolated_environment() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "readiness_tests::user_cache_child",
            "--nocapture",
        ])
        .env("KORTEX_TEST_CACHE_ROOT", root.join("repos"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
