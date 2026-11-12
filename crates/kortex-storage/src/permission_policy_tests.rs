#![cfg(unix)]
use crate::{Store, registry::Registry};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn other_user_writable_index_artifacts_are_refused_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    drop(Store::open(&root, true).unwrap());
    for name in ["index.db", "writer.lock", "index-owner.json"] {
        let path = root.join(".kortex").join(name);
        let original = fs::read(&path).unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        for mode in [0o620, 0o602] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            for create in [false, true] {
                let error = Store::open(&root, create)
                    .err()
                    .expect("accepted artifact writable by others");
                assert_eq!(error.code, "PATH_REFUSED");
                assert_eq!(error.exit, 8);
                assert_eq!(fs::read(&path).unwrap(), original);
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    mode
                );
            }
        }
        fs::set_permissions(&path, permissions).unwrap();
    }
    assert!(Store::open(&root, false).is_ok());
}

#[test]
fn other_user_writable_registry_is_refused_without_mutation() {
    let directory = crate::test_support::tempdir().unwrap();
    let path = directory.path().join("registry.db");
    drop(Registry::at(&path).unwrap());
    let original = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    let error = Registry::at(&path)
        .err()
        .expect("accepted registry writable by others");
    assert_eq!(error.code, "PATH_REFUSED");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o666
    );
}
