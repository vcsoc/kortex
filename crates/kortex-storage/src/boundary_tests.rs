use super::*;
#[test]
fn repair_refuses_future_version_in_main_header_and_uncheckpointed_wal() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), true).unwrap();
    drop(store);
    let dir = d.path().join(".kortex");
    let c = Connection::open(dir.join("index.db")).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=99;",
    )
    .unwrap();
    let original: Vec<_> = ["index.db", "index.db-wal", "index.db-shm"]
        .map(|name| (name, fs::read(dir.join(name)).unwrap()))
        .into_iter()
        .collect();
    assert_eq!(
        Store::repair(d.path(), "repository").unwrap_err().code,
        "INDEX_VERSION"
    );
    for (name, bytes) in original {
        assert_eq!(
            fs::read(dir.join(name)).unwrap(),
            bytes,
            "repair mutated {name}"
        );
    }
    drop(c);
    let bytes = fs::read(dir.join("index.db")).unwrap();
    assert_eq!(
        Store::repair(d.path(), "repository").unwrap_err().code,
        "INDEX_VERSION"
    );
    assert_eq!(fs::read(dir.join("index.db")).unwrap(), bytes);
}
#[cfg(unix)]
#[test]
fn planted_journal_registry_and_cache_ancestor_symlinks_never_touch_targets() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("source.rs");
    fs::write(&source, "never truncate").unwrap();
    fs::create_dir(root.join(".kortex")).unwrap();
    symlink(&source, root.join(".kortex/index.db-journal")).unwrap();
    assert_eq!(Store::open(&root, true).err().unwrap().code, "PATH_REFUSED");
    let registry = root.join("registry.db");
    symlink(&source, &registry).unwrap();
    assert_eq!(
        crate::registry::Registry::at(&registry).err().unwrap().code,
        "PATH_REFUSED"
    );
    let second = root.join("second.db");
    symlink(&source, root.join("second.db-journal")).unwrap();
    assert_eq!(
        crate::registry::Registry::at(&second).err().unwrap().code,
        "PATH_REFUSED"
    );
    assert!(!second.exists());
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    let cache = root.join("cache");
    fs::create_dir(&cache).unwrap();
    symlink(&outside, cache.join("kortex")).unwrap();
    assert_eq!(
        location::check_directory(&cache.join("kortex/indexes/hash"), true)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );
    assert!(!outside.join("indexes").exists());
    assert_eq!(fs::read_to_string(source).unwrap(), "never truncate");
}
#[test]
fn moved_index_identity_requires_explicit_preservation_before_rebuild() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    drop(Store::open(&a, true).unwrap());
    let marker = fs::read(a.join(".kortex/index-owner.json")).unwrap();
    fs::rename(a.join(".kortex"), b.join(".kortex")).unwrap();
    for create in [false, true] {
        assert_eq!(
            Store::open(&b, create).err().unwrap().code,
            "INDEX_IDENTITY"
        );
    }
    let report = Store::repair(&b, "repository").unwrap();
    assert_eq!(
        fs::read(report.backup_directory.join("index-owner.json")).unwrap(),
        marker
    );
    drop(Store::open(&b, true).unwrap());
    assert_ne!(
        fs::read(b.join(".kortex/index-owner.json")).unwrap(),
        marker
    );
}
#[cfg(unix)]
#[test]
fn cache_parent_permissions_refuse_cross_user_rename_boundaries() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let directory = d
        .path()
        .canonicalize()
        .unwrap()
        .join("cache/kortex/indexes/hash");
    location::check_index_directory(&directory, "user-cache", true).unwrap();
    let parent = directory.parent().unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o777)).unwrap();
    for create in [false, true] {
        assert_eq!(
            location::check_index_directory(&directory, "user-cache", create)
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }
    // Refusal does not silently chmod/adopt an unsafe existing directory.
    assert_eq!(
        fs::metadata(parent).unwrap().permissions().mode() & 0o777,
        0o777
    );
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    location::check_index_directory(&directory, "user-cache", false).unwrap();
}
#[cfg(unix)]
#[test]
fn permissions_are_private_without_changing_existing_ownership() {
    use std::os::unix::fs::MetadataExt;
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), true).unwrap();
    assert_eq!(
        fs::metadata(&store.directory).unwrap().mode() & 0o777,
        0o700
    );
    for name in ["index.db", "writer.lock"] {
        assert_eq!(
            fs::metadata(store.directory.join(name)).unwrap().mode() & 0o777,
            0o600
        );
    }
    let path = d.path().join("private/registry.db");
    let _registry = crate::registry::Registry::at(&path).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
}
