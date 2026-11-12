use super::*;
#[cfg(unix)]
use kortex_core::hash;

#[cfg(unix)]
fn directory() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("cache");
    (temp, path)
}
#[cfg(unix)]
#[test]
fn failed_transaction_retains_previous_vectors_and_errors_are_not_cache_misses() {
    let (_temp, path) = directory();
    let model = hash("versioned-model-fixture");
    let content = hash("old");
    let vector = [1., 0., 0., 0.];
    let mut cache = SqliteVectorPersistence::open(&path, 1, 1024).unwrap();
    cache.put(&model, &content, &vector).unwrap();
    cache.conn.execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON vectors BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
    assert_eq!(
        cache.put(&model, &hash("new"), &vector).unwrap_err().code,
        "VECTOR_CACHE_STORAGE"
    );
    assert_eq!(cache.get(&model, &content, 4).unwrap().unwrap(), vector);
    assert_eq!(cache.usage().unwrap(), (1, 232));
    cache
        .conn
        .execute_batch("DROP TRIGGER reject_insert")
        .unwrap();
    cache.put(&model, &hash("new"), &vector).unwrap();
    assert!(cache.get(&model, &content, 4).unwrap().is_none());
    drop(cache);
    std::fs::write(path.join(DATABASE), b"retained corrupted cache bytes").unwrap();
    assert!(SqliteVectorPersistence::open(&path, 1, 1024).is_err());
    assert_eq!(
        std::fs::read(path.join(DATABASE)).unwrap(),
        b"retained corrupted cache bytes"
    );
}
#[cfg(unix)]
#[test]
fn malformed_keys_dimensions_norm_and_checksum_refuse_without_fabricating_hits() {
    let (_temp, path) = directory();
    let mut cache = SqliteVectorPersistence::open(&path, 10, 1024).unwrap();
    let model = hash("model");
    let content = hash("content");
    for vector in [
        vec![],
        vec![f32::NAN],
        vec![f32::INFINITY],
        vec![0.0],
        vec![2.0],
        vec![1.0; 4097],
    ] {
        assert!(cache.put(&model, &content, &vector).is_err());
    }
    assert!(
        cache
            .put("raw source text is not a key", &content, &[1.0])
            .is_err()
    );
    assert!(cache.put(&model, &"A".repeat(64), &[1.0]).is_err());
    assert_eq!(cache.usage().unwrap(), (0, 0));
    cache.put(&model, &content, &[1., 0., 0., 0.]).unwrap();
    assert_eq!(
        cache.get(&model, &content, 3).unwrap_err().code,
        "VECTOR_CACHE_CORRUPT"
    );
    for vector in [[f32::NAN, 0., 0., 0.], [0., 0., 0., 0.], [2., 0., 0., 0.]] {
        let bytes = vector
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        cache
            .conn
            .execute(
                "UPDATE vectors SET vector=?1,checksum=?2",
                params![&bytes, checksum(&model, &content, 4, &bytes)],
            )
            .unwrap();
        assert_eq!(
            cache.get(&model, &content, 4).unwrap_err().code,
            "VECTOR_CACHE_CORRUPT"
        );
    }
    cache
        .conn
        .execute("UPDATE vectors SET vector=zeroblob(20000),bytes=20216", [])
        .unwrap();
    assert_eq!(
        cache.get(&model, &content, 4).unwrap_err().code,
        "VECTOR_CACHE_CORRUPT"
    );
    drop(cache);
    assert!(SqliteVectorPersistence::open(&path, 10, 1024).is_err());
}
#[cfg(unix)]
#[test]
fn private_ancestors_artifacts_and_sidecars_are_checked_before_writes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (temp, path) = directory();
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        SqliteVectorPersistence::open(&path, 10, 1024)
            .err()
            .unwrap()
            .exit,
        8
    );
    assert!(!path.join(DATABASE).exists());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let outside = temp.path().join("outside");
    std::fs::write(&outside, b"untouched").unwrap();
    symlink(&outside, path.join(JOURNAL)).unwrap();
    assert!(SqliteVectorPersistence::open(&path, 10, 1024).is_err());
    assert!(!path.join(DATABASE).exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    std::fs::remove_file(path.join(JOURNAL)).unwrap();
    std::fs::hard_link(&outside, path.join(DATABASE)).unwrap();
    assert!(SqliteVectorPersistence::open(&path, 10, 1024).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    std::fs::remove_file(path.join(DATABASE)).unwrap();
    let cache = SqliteVectorPersistence::open(&path, 10, 1024).unwrap();
    assert_eq!(
        std::fs::metadata(path.join(DATABASE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.join(JOURNAL))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(cache);
    let writable = temp.path().canonicalize().unwrap().join("writable");
    std::fs::create_dir(&writable).unwrap();
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(SqliteVectorPersistence::open(&writable.join("not-created"), 10, 1024).is_err());
    assert!(!writable.join("not-created").exists());
}
#[cfg(target_os = "macos")]
#[test]
fn native_allow_and_inheritance_acls_refuse_cache_before_secret_writes() {
    use std::os::unix::fs::PermissionsExt;
    for inheritance in ["", ",file_inherit,directory_inherit,only_inherit"] {
        let (_temp, path) = directory();
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = std::process::Command::new("/bin/chmod")
            .args([
                "+a",
                &format!(
                    "everyone allow read,write,search,delete_child,writesecurity{inheritance}"
                ),
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let before = std::process::Command::new("/bin/ls")
            .arg("-lde")
            .arg(&path)
            .output()
            .unwrap();
        let error = SqliteVectorPersistence::open(&path.join("missing/cache"), 10, 1024)
            .err()
            .unwrap();
        assert_eq!(error.exit, 8);
        assert!(error.message.contains("ACL"), "{error}");
        assert!(!path.join("missing").exists());
        assert!(!path.join(DATABASE).exists());
        let after = std::process::Command::new("/bin/ls")
            .arg("-lde")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            before.stdout, after.stdout,
            "refusal must not strip native ACLs"
        );
    }
}
#[cfg(unix)]
#[test]
fn oversized_sparse_journal_refuses_before_sqlite_or_missing_database_creation() {
    use std::os::unix::fs::{FileExt, MetadataExt};
    for existing_database in [false, true] {
        let (_temp, path) = directory();
        if existing_database {
            drop(SqliteVectorPersistence::open(&path, 10, 1024).unwrap());
        }
        let anchor = DirectoryAnchor::acquire(&path, true).unwrap();
        let journal = if existing_database {
            anchor.open_existing(JOURNAL, true).unwrap()
        } else {
            anchor.create_new(JOURNAL).unwrap()
        };
        journal.set_len(MAX_JOURNAL_BYTES + 1).unwrap();
        // SQLite journal magic, plus distinct actual bytes at the sparse file's end.
        let prefix = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];
        let suffix = *b"retained-journal";
        journal.write_all_at(&prefix, 0).unwrap();
        let end = MAX_JOURNAL_BYTES + 1 - suffix.len() as u64;
        journal.write_all_at(&suffix, end).unwrap();
        journal.sync_all().unwrap();
        let identity = |metadata: std::fs::Metadata| {
            (
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
                metadata.len(),
                metadata.blocks(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        };
        let before = identity(journal.metadata().unwrap());
        let database_before = existing_database.then(|| {
            (
                identity(std::fs::metadata(path.join(DATABASE)).unwrap()),
                std::fs::read(path.join(DATABASE)).unwrap(),
            )
        });
        let error = SqliteVectorPersistence::open(&path, 10, 1024)
            .err()
            .unwrap();
        assert_eq!(error.code, "VECTOR_CACHE_CORRUPT");
        assert!(error.message.contains("journal exceeds 4 GiB"));
        assert_eq!(identity(journal.metadata().unwrap()), before);
        assert_eq!(
            identity(std::fs::metadata(path.join(JOURNAL)).unwrap()),
            before
        );
        let mut actual_prefix = [0; 8];
        let mut actual_suffix = [0; 16];
        journal.read_exact_at(&mut actual_prefix, 0).unwrap();
        journal.read_exact_at(&mut actual_suffix, end).unwrap();
        assert_eq!(actual_prefix, prefix);
        assert_eq!(actual_suffix, suffix);
        if let Some((metadata, bytes)) = database_before {
            assert_eq!(
                identity(std::fs::metadata(path.join(DATABASE)).unwrap()),
                metadata
            );
            assert_eq!(std::fs::read(path.join(DATABASE)).unwrap(), bytes);
        } else {
            assert!(
                !path.join(DATABASE).exists(),
                "oversized orphan journal must not create the main DB"
            );
        }
        assert!(!path.join("vectors-v1.db-wal").exists());
        assert!(!path.join("vectors-v1.db-shm").exists());
    }
}

#[cfg(unix)]
#[test]
fn journal_descriptor_cap_allows_sector_and_spill_overhead_beyond_main_db_cap() {
    let (_temp, path) = directory();
    let anchor = DirectoryAnchor::acquire(&path, true).unwrap();
    let journal = anchor.create_new(JOURNAL).unwrap();
    let maximum_frame_and_sector_overhead = 32768_u64 * (4096 + 8 + 65536) + 65536;
    assert!(MAX_JOURNAL_BYTES > maximum_frame_and_sector_overhead);
    journal.set_len(MAX_JOURNAL_BYTES).unwrap();
    validate_artifact_size(&journal, JOURNAL).unwrap();
    journal.set_len(MAX_JOURNAL_BYTES + 1).unwrap();
    assert_eq!(
        validate_artifact_size(&journal, JOURNAL).unwrap_err().code,
        "VECTOR_CACHE_CORRUPT"
    );
    assert!(!path.join(DATABASE).exists());
}

#[cfg(not(unix))]
#[test]
fn unsupported_privacy_refuses_before_directory_creation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("not-created");
    assert_eq!(
        SqliteVectorPersistence::open(&path, 1, 1024)
            .err()
            .unwrap()
            .code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert!(!path.exists());
}
