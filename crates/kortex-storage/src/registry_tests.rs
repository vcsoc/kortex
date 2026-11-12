use super::registry::Registry;
use super::*;

fn entry(id: &str, root: &Path, generation: &str) -> Snapshot {
    Snapshot {
        repo_id: id.into(),
        root: root.display().to_string(),
        generation_id: generation.into(),
        ..Snapshot::default()
    }
}

#[test]
fn registry_upsert_restart_and_remove_preserve_source_and_index() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("nested/registry.db");
    let repo = d.path().join("repository");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("source.py"), "valuable source\n").unwrap();
    let mut store = Store::open(&repo, true).unwrap();
    let lock = store.writer_lock().unwrap();
    let id = "id'); DROP TABLE registry; --";
    let first = entry(id, &repo, "one");
    store.commit(&first, &lock).unwrap();
    drop(lock);
    let registry = Registry::at(&db).unwrap();
    assert!(registry.list().unwrap().is_empty());
    registry.register(&first).unwrap();
    let next = entry(id, &repo, "two");
    registry.register(&next).unwrap();
    drop(registry);
    let registry = Registry::at(&db).unwrap();
    assert_eq!(
        registry.list().unwrap(),
        vec![(id.into(), next.root.clone(), "two".into())]
    );
    registry.remove(id).unwrap();
    assert!(registry.list().unwrap().is_empty());
    let error = registry.remove(id).unwrap_err();
    assert_eq!((error.code, error.exit), ("REPO_NOT_FOUND", 3));
    assert_eq!(
        fs::read_to_string(repo.join("source.py")).unwrap(),
        "valuable source\n"
    );
    assert_eq!(
        store.load().unwrap().generation_id,
        "one",
        "unregistering must never delete the index"
    );
    assert_eq!(store.integrity().unwrap(), "ok");
}

#[test]
fn independent_registry_connections_do_not_lose_entries() {
    let d = crate::test_support::tempdir().unwrap();
    let db = d.path().join("registry.db");
    let registry = Registry::at(&db).unwrap();
    let mut workers = vec![];
    for worker in 0..4 {
        let db = db.clone();
        workers.push(std::thread::spawn(move || {
            let registry = Registry::at(&db).unwrap();
            for n in 0..8 {
                let id = format!("{worker}-{n}");
                registry
                    .register(&entry(&id, Path::new(&format!("/repo/{id}")), "one"))
                    .unwrap();
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let rows = registry.list().unwrap();
    assert_eq!(rows.len(), 32);
    assert!(
        rows.windows(2)
            .all(|w| (&w[0].1, &w[0].0) < (&w[1].1, &w[1].0))
    );
    for (id, root, generation) in rows {
        assert_eq!(root, format!("/repo/{id}"));
        assert_eq!(generation, "one");
    }
}

#[test]
fn registry_path_child() {
    let Some(expected) = std::env::var_os("KORTEX_TEST_EXPECT_REGISTRY") else {
        return;
    };
    if expected == "error" {
        let error = Registry::path().unwrap_err();
        assert_eq!((error.code, error.exit), ("CONFIG_HOME", 2));
        assert!(Registry::open().is_err());
    } else {
        assert_eq!(Registry::path().unwrap(), PathBuf::from(expected));
        let registry = Registry::open().unwrap();
        assert!(registry.list().unwrap().is_empty());
    }
}

#[test]
fn registry_environment_precedence_is_isolated_from_other_tests() {
    let d = tempfile::tempdir().unwrap();
    for case in ["custom", "xdg", "local", "home", "missing"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "registry_tests::registry_path_child",
            "--nocapture",
        ]);
        for key in ["KORTEX_REGISTRY", "XDG_DATA_HOME", "LOCALAPPDATA", "HOME"] {
            child.env_remove(key);
        }
        let home = d.path().join(format!("{case}-home"));
        let local = d.path().join(format!("{case}-local"));
        let xdg = d.path().join(format!("{case}-xdg"));
        if case != "missing" {
            child.env("HOME", &home);
        }
        if matches!(case, "custom" | "xdg" | "local") {
            child.env("LOCALAPPDATA", &local);
        }
        if matches!(case, "custom" | "xdg") {
            child.env("XDG_DATA_HOME", &xdg);
        }
        let expected = match case {
            "custom" => {
                let custom = d.path().join("custom/registry.db");
                child.env("KORTEX_REGISTRY", &custom);
                custom
            }
            "xdg" => xdg.join("kortex/registry.db"),
            "local" => local.join("kortex/registry.db"),
            "home" => home.join(".local/share/kortex/registry.db"),
            "missing" => PathBuf::from("error"),
            _ => unreachable!(),
        };
        let output = child
            .env("KORTEX_TEST_EXPECT_REGISTRY", expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn damaged_registry_is_reported_without_repair_or_data_loss() {
    let d = crate::test_support::tempdir().unwrap();
    let path = d.path().join("registry.db");
    let bytes = b"corrupt registry backup must remain";
    crate::test_support::file(&path, bytes).unwrap();
    let error = Registry::at(&path).err().expect("corruption accepted");
    assert_eq!((error.code, error.exit), ("STORAGE_ERROR", 6));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let blocked = d.path().join("file");
    fs::write(&blocked, "not a directory").unwrap();
    assert!(Registry::at(&blocked.join("registry.db")).is_err());
    assert_eq!(fs::read_to_string(&blocked).unwrap(), "not a directory");
}
