use kortex_core::Snapshot;
use kortex_storage::registry::{Registry, RepositorySelection};
use rusqlite::Connection;
#[path = "../src/test_support.rs"]
mod test_support;

#[test]
fn legacy_registry_migrates_provenance_atomically_and_retains_compatibility() {
    let d = test_support::tempdir().unwrap();
    let path = d.path().join("registry.db");
    let root = d.path().join("effective");
    test_support::directory(&root).unwrap();
    test_support::file(&path, b"").unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE registry(id TEXT PRIMARY KEY,root TEXT NOT NULL,generation_id TEXT NOT NULL);").unwrap();
    conn.execute(
        "INSERT INTO registry VALUES('repo',?1,'legacy')",
        [root.to_str().unwrap()],
    )
    .unwrap();
    drop(conn);
    let registry = Registry::at(&path).unwrap();
    assert_eq!(registry.selection("repo").unwrap(), None);
    assert_eq!(registry.selection("missing").unwrap(), None);
    let mut snapshot = Snapshot {
        repo_id: "repo".into(),
        root: root.to_str().unwrap().into(),
        generation_id: "next".into(),
        ..Default::default()
    };
    let selection = RepositorySelection {
        invocation_root: d.path().join("invocation"),
        explicit_config: Some(d.path().join("elsewhere/selected.toml")),
        effective_root: root.clone(),
    };
    registry
        .register_with_selection(&snapshot, &selection)
        .unwrap();
    drop(registry);
    let registry = Registry::at(&path).unwrap();
    assert_eq!(registry.selection("repo").unwrap(), Some(selection.clone()));
    snapshot.generation_id = "refresh".into();
    registry.register(&snapshot).unwrap();
    assert_eq!(registry.selection("repo").unwrap(), Some(selection.clone()));
    assert_eq!(
        registry.list().unwrap(),
        vec![("repo".into(), snapshot.root.clone(), "refresh".into())]
    );

    // Real SQLite failure on provenance insertion must roll back the generation update too.
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_selection BEFORE INSERT ON repository_selection BEGIN SELECT RAISE(ABORT,'injected selection failure'); END;").unwrap();
    snapshot.generation_id = "must-rollback".into();
    assert!(
        registry
            .register_with_selection(&snapshot, &selection)
            .is_err()
    );
    assert_eq!(registry.list().unwrap()[0].2, "refresh");
    assert_eq!(registry.selection("repo").unwrap(), Some(selection.clone()));
    conn.execute_batch("DROP TRIGGER fail_selection").unwrap();

    let mut bad = selection.clone();
    bad.effective_root = d.path().join("wrong");
    assert_eq!(
        registry
            .register_with_selection(&snapshot, &bad)
            .unwrap_err()
            .code,
        "REPO_SELECTION"
    );
    bad = selection.clone();
    bad.explicit_config = Some("relative.toml".into());
    assert_eq!(
        registry
            .register_with_selection(&snapshot, &bad)
            .unwrap_err()
            .code,
        "REPO_SELECTION"
    );
    assert_eq!(registry.list().unwrap()[0].2, "refresh");
    snapshot.root = d.path().join("new-identity").to_str().unwrap().into();
    registry.register(&snapshot).unwrap();
    assert_eq!(registry.selection("repo").unwrap(), None);
    snapshot.root = root.to_str().unwrap().into();
    registry
        .register_with_selection(&snapshot, &selection)
        .unwrap();
    registry.remove("repo").unwrap();
    assert_eq!(registry.selection("repo").unwrap(), None);
    let remaining: i64 = conn
        .query_row("SELECT count(*) FROM repository_selection", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn concurrent_selection_registration_has_no_lost_or_torn_provenance() {
    let d = test_support::tempdir().unwrap();
    let db = d.path().join("registry.db");
    drop(Registry::at(&db).unwrap());
    let mut workers = vec![];
    for worker in 0..4 {
        let db = db.clone();
        let base = d.path().join(format!("invocation-{worker}"));
        workers.push(std::thread::spawn(move || {
            let registry = Registry::at(&db).unwrap();
            for n in 0..8 {
                let id = format!("{worker}-{n}");
                let root = base.join("effective");
                let snapshot = Snapshot {
                    repo_id: id.clone(),
                    root: root.to_str().unwrap().into(),
                    generation_id: "registered".into(),
                    ..Default::default()
                };
                let selection = RepositorySelection {
                    invocation_root: base.clone(),
                    explicit_config: Some(base.join("trusted.toml")),
                    effective_root: root,
                };
                registry
                    .register_with_selection(&snapshot, &selection)
                    .unwrap();
                assert_eq!(registry.selection(&id).unwrap(), Some(selection));
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let registry = Registry::at(&db).unwrap();
    let rows = registry.list().unwrap();
    assert_eq!(rows.len(), 32);
    for (id, root, generation) in rows {
        assert_eq!(generation, "registered");
        let selection = registry.selection(&id).unwrap().unwrap();
        assert_eq!(selection.effective_root.to_str().unwrap(), root);
        assert_eq!(
            selection.explicit_config,
            Some(selection.invocation_root.join("trusted.toml"))
        );
    }
}

#[cfg(windows)]
#[test]
fn extended_windows_path_provenance_matches_public_root_identity() {
    let d = test_support::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("registry.db")).unwrap();
    let selection = RepositorySelection {
        invocation_root: r"\\?\C:\invocation".into(),
        explicit_config: Some(r"\\?\C:\config\trusted.toml".into()),
        effective_root: r"\\?\C:\selected\root".into(),
    };
    let snapshot = Snapshot {
        repo_id: "windows-identity".into(),
        root: "C:/selected/root".into(),
        ..Default::default()
    };
    registry
        .register_with_selection(&snapshot, &selection)
        .unwrap();
    assert_eq!(
        registry.selection("windows-identity").unwrap(),
        Some(selection)
    );
}

#[test]
fn corrupt_provenance_is_not_silently_reinterpreted() {
    let d = test_support::tempdir().unwrap();
    let path = d.path().join("registry.db");
    let registry = Registry::at(&path).unwrap();
    let root = d.path().join("root");
    let snapshot = Snapshot {
        repo_id: "repo".into(),
        root: root.to_str().unwrap().into(),
        ..Default::default()
    };
    let selection = RepositorySelection {
        invocation_root: root.clone(),
        explicit_config: None,
        effective_root: root,
    };
    registry
        .register_with_selection(&snapshot, &selection)
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    for raw in [
        "{",
        r#"{"invocation_root":"relative","explicit_config":null,"effective_root":"/wrong"}"#,
        r#"{"invocation_root":"/root","explicit_config":null,"effective_root":"/root","secret":"not a supported field"}"#,
    ] {
        conn.execute("UPDATE repository_selection SET selection_json=?1", [raw])
            .unwrap();
        assert_eq!(
            registry.selection("repo").unwrap_err().code,
            "CORRUPT_REGISTRY"
        );
    }
}
