//! Production entrypoint refusal tests, independent of legacy fixtures' process umask.
#[cfg(unix)]
mod unix {
    use crate::{Store, registry::Registry};
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn every_writable_ancestor_is_checked_before_store_registry_or_lock_side_effects() {
        for mode in [0o770, 0o772, 0o775, 0o777, 0o707, 0o722] {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().canonicalize().unwrap();
            let ancestor = root.join("ancestor");
            let leaf = ancestor.join("leaf");
            fs::create_dir_all(&leaf).unwrap();
            fs::set_permissions(&leaf, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&ancestor, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(Store::open(&leaf, true).err().unwrap().code, "PATH_REFUSED");
            assert!(!leaf.join(".kortex").exists());
            assert_eq!(
                Registry::at(&leaf.join("registry.db")).err().unwrap().code,
                "PATH_REFUSED"
            );
            assert!(!leaf.join("registry.db").exists());
            assert!(crate::acquire_writer(&leaf).is_err());
            assert!(!leaf.join("writer.lock").exists());
            assert_eq!(
                fs::metadata(&ancestor).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
    }
    #[test]
    fn already_open_writers_recheck_placement_without_chmod_or_fact_changes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(&root, true).unwrap();
        let lock = store.writer_lock().unwrap();
        store
            .commit(&crate::read_tests::fixture("old"), &lock)
            .unwrap();
        let registry = Registry::at(&root.join("registry.db")).unwrap();
        registry
            .register(&crate::read_tests::fixture("old"))
            .unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            store
                .commit(&crate::read_tests::fixture("new"), &lock)
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
        assert_eq!(
            registry
                .register(&crate::read_tests::fixture("new"))
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
        assert_eq!(registry.remove("repo").unwrap_err().code, "PATH_REFUSED");
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o777
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(store.load().unwrap().generation_id, "old");
        assert_eq!(registry.list().unwrap()[0].2, "old");
        store
            .commit(&crate::read_tests::fixture("new"), &lock)
            .unwrap();
    }
    #[test]
    fn readable_existing_artifacts_are_refused_not_adopted() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        drop(Store::open(&root, true).unwrap());
        let path = root.join(".kortex/index.db");
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        for create in [false, true] {
            assert_eq!(
                Store::open(&root, create).err().unwrap().code,
                "PATH_REFUSED"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    #[cfg(target_os = "macos")]
    fn add_acl(path: &std::path::Path, text: &str) {
        let result = std::process::Command::new("/bin/chmod")
            .args(["+a", text])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_allow_acl_is_refused_before_store_registry_repair_and_existing_file_use() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let blocked = root.join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
        add_acl(
            &blocked,
            "everyone allow read,write,search,delete_child,writesecurity,file_inherit,directory_inherit,only_inherit",
        );
        let error = Store::open(&blocked, true).err().unwrap();
        assert!(error.message.contains("ACL"), "{error}");
        assert!(!blocked.join(".kortex").exists());
        let error = Registry::at(&blocked.join("registry.db")).err().unwrap();
        assert!(error.message.contains("ACL"), "{error}");
        assert!(!blocked.join("registry.db").exists());
        let allowed = root.join("allowed");
        fs::create_dir(&allowed).unwrap();
        drop(Store::open(&allowed, true).unwrap());
        let db = allowed.join(".kortex/index.db");
        let bytes = fs::read(&db).unwrap();
        add_acl(&db, "everyone allow read,write,writesecurity");
        assert_eq!(
            fs::metadata(&db).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for create in [false, true] {
            let error = Store::open(&allowed, create).err().unwrap();
            assert!(error.message.contains("ACL"), "{error}");
        }
        let error = Store::repair(&allowed, "repository").unwrap_err();
        assert!(error.message.contains("ACL"), "{error}");
        assert_eq!(fs::read(&db).unwrap(), bytes);
        assert!(!fs::read_dir(allowed.join(".kortex")).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("repair-")
        }));
    }
}
#[cfg(windows)]
#[test]
fn unsupported_privacy_refuses_all_entrypoints_without_artifacts() {
    use crate::{Store, experimental::DifferentialStore, registry::Registry};
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    for create in [false, true] {
        assert_eq!(
            Store::open(root, create).err().unwrap().code,
            "PRIVATE_STORAGE_UNSUPPORTED"
        );
    }
    assert_eq!(
        Store::repair(root, "repository").unwrap_err().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert_eq!(
        Registry::at(&root.join("registry.db")).err().unwrap().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert_eq!(
        crate::acquire_writer(root).err().unwrap().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert_eq!(
        DifferentialStore::create_scratch(&root.join("scratch"))
            .err()
            .unwrap()
            .code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert!(std::fs::read_dir(root).unwrap().next().is_none());
}
