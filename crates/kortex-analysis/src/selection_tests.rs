use super::*;
use kortex_storage::registry::RepositorySelection;
use std::fs;

#[test]
fn registered_selection_child() {
    let Some(base) = std::env::var_os("KORTEX_SELECTION_TEST_ROOT") else {
        return;
    };
    let base = PathBuf::from(base).canonicalize().unwrap();
    let invocation = base.join("project");
    let effective = invocation.join("src");
    fs::create_dir_all(&effective).unwrap();
    fs::write(effective.join("app.py"), "def selected():\n    pass\n").unwrap();
    let mode = std::env::var("KORTEX_SELECTION_TEST_MODE").unwrap();
    let explicit_file = base.join("explicit.toml");
    let text = format!(
        "[repository]\nroot={:?}\n[index]\nstorage={:?}\n",
        effective.to_str().unwrap(),
        if mode == "cache" {
            "user-cache"
        } else {
            "repository"
        }
    );
    let explicit = if mode == "explicit" {
        fs::write(&explicit_file, &text).unwrap();
        Some(explicit_file.as_path())
    } else if mode == "user" {
        let user = kortex_config::user_config_path().unwrap();
        fs::create_dir_all(user.parent().unwrap()).unwrap();
        fs::write(user, &text).unwrap();
        None
    } else {
        fs::write(
            invocation.join(".kortex.toml"),
            if mode == "cache" {
                "[repository]\nroot='src'\n[index]\nstorage='user-cache'\n"
            } else {
                "[repository]\nroot='src'\n"
            },
        )
        .unwrap();
        None
    };
    let config = Config::load(&invocation, explicit).unwrap();
    let root = config.resolved_root(&invocation).unwrap();
    let dry = kortex_indexer::index_at_root(
        &root,
        &config,
        &kortex_indexer::Options {
            dry_run: true,
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!dry.committed);
    assert!(
        !Registry::path().unwrap().exists(),
        "dry runs may not create registry provenance"
    );
    let report = kortex_indexer::index_at_root(&root, &config, &Default::default()).unwrap();
    let repo_id = &report.snapshot.repo_id;
    // The indexer itself, not a transport test shim, has persisted the selection.
    let registry = Registry::open().unwrap();
    let selection = registry.selection(repo_id).unwrap().unwrap();
    assert_eq!(selection.invocation_root, invocation);
    assert_eq!(selection.explicit_config.as_deref(), explicit);
    assert_eq!(selection.effective_root, root);
    drop(registry);
    for _ in 0..2 {
        let service = Service::open_selected(Some(repo_id), None).unwrap();
        assert_eq!(service.root, root);
        assert_eq!(service.freshness().unwrap(), "fresh");
        assert!(service.lookup("selected").is_ok());
        service
            .execute("status", &json!({"require_fresh":true}))
            .unwrap();
    }
    assert_eq!(
        Service::open_with_config(&invocation, explicit)
            .unwrap()
            .freshness()
            .unwrap(),
        "fresh"
    );
    let restart = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "selection_tests::registered_reopen_child",
            "--nocapture",
        ])
        .env("KORTEX_SELECTION_REOPEN_ID", repo_id)
        .env("KORTEX_SELECTION_REOPEN_ROOT", &root)
        .output()
        .unwrap();
    assert!(
        restart.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    let page = list_repos_page(&json!({})).unwrap();
    assert_eq!(page["data"]["items"][0]["index_status"], "fresh");
    if mode == "cache" {
        assert!(!effective.join(".kortex").exists());
    }

    let outside = base.join("other");
    fs::create_dir(&outside).unwrap();
    // Auth checks must precede any index open at an unauthorized location.
    assert_eq!(
        Service::open_registered(repo_id, &outside, None)
            .err()
            .unwrap()
            .code,
        "REPO_FORBIDDEN"
    );
    let changed = base.join("changed.toml");
    fs::write(
        &changed,
        format!("[repository]\nroot={:?}\n", outside.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(
        Service::open_registered(repo_id, &root, Some(&changed))
            .err()
            .unwrap()
            .code,
        "REPO_FORBIDDEN"
    );
    let registry = Registry::open().unwrap();
    registry
        .register_with_selection(
            &report.snapshot,
            &RepositorySelection {
                invocation_root: outside.clone(),
                explicit_config: Some(changed),
                effective_root: root.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        Service::open_registered(repo_id, &root, None)
            .err()
            .unwrap()
            .code,
        "REPO_FORBIDDEN"
    );
    registry
        .register_with_selection(&report.snapshot, &selection)
        .unwrap();
    // Changing the originating config root must refuse, never open the new root's index.
    let selected_file = match mode.as_str() {
        "explicit" => explicit_file.clone(),
        "user" => kortex_config::user_config_path().unwrap(),
        _ => invocation.join(".kortex.toml"),
    };
    let alternate = invocation.join("alternate");
    fs::create_dir(&alternate).unwrap();
    fs::write(
        &selected_file,
        format!("[repository]\nroot={:?}\n", alternate.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(
        Service::open_registered(repo_id, &root, None)
            .err()
            .unwrap()
            .code,
        "REPO_FORBIDDEN"
    );
    fs::write(
        &selected_file,
        format!("[repository]\nroot={:?}\n", outside.to_str().unwrap()),
    )
    .unwrap();
    let error = Service::open_registered(repo_id, &root, None)
        .err()
        .unwrap();
    assert_eq!(
        error.code,
        if mode == "explicit" || mode == "user" {
            "REPO_FORBIDDEN"
        } else {
            "SECURITY_POLICY"
        }
    );
    fs::write(&selected_file, &text).unwrap();
    let (selected_root, selected_config) = resolve_selection(Some(repo_id), None).unwrap();
    assert_eq!(selected_root, root);
    assert_eq!(
        selected_config.origin.as_ref().unwrap().invocation_root,
        invocation
    );
    assert_eq!(
        selected_config
            .origin
            .as_ref()
            .unwrap()
            .explicit_config
            .as_deref(),
        explicit
    );
    let fingerprint = selected_config.fingerprint().unwrap();
    let db = Store::directory_for(&root, &selected_config.index.storage)
        .unwrap()
        .join("index.db");
    fs::write(&db, b"retained corrupt administrative regression").unwrap();
    assert!(Service::open_selected(Some(repo_id), None).is_err());
    let (resolved, reloaded) = resolve_selection(Some(repo_id), None).unwrap();
    assert_eq!(resolved, root);
    assert_eq!(reloaded.fingerprint().unwrap(), fingerprint);
    assert_eq!(
        fs::read(&db).unwrap(),
        b"retained corrupt administrative regression"
    );
    fs::remove_file(&db).unwrap();
    assert_eq!(resolve_selection(Some(repo_id), None).unwrap().0, root);
    assert!(
        !db.exists(),
        "configuration selection must not create index storage"
    );
}

#[test]
fn registered_reopen_child() {
    let Ok(repo_id) = std::env::var("KORTEX_SELECTION_REOPEN_ID") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("KORTEX_SELECTION_REOPEN_ROOT").unwrap());
    let service = Service::open_registered(&repo_id, &root, None).unwrap();
    assert_eq!(service.freshness().unwrap(), "fresh");
    assert!(service.lookup("selected").is_ok());
    service
        .execute("status", &json!({"require_fresh":true}))
        .unwrap();
}
#[test]
fn registered_ids_retain_relocated_explicit_user_and_cache_selection_after_restart() {
    for mode in ["repository", "explicit", "user", "cache"] {
        let temp = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "selection_tests::registered_selection_child",
                "--nocapture",
            ])
            .env("KORTEX_SELECTION_TEST_ROOT", temp.path())
            .env("KORTEX_SELECTION_TEST_MODE", mode)
            .env(
                "KORTEX_REGISTRY",
                temp.path().join("private-registry/registry.db"),
            )
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CACHE_HOME", temp.path().join("cache"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mode={mode}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn legacy_selection_child() {
    let Some(base) = std::env::var_os("KORTEX_LEGACY_TEST_ROOT") else {
        return;
    };
    let base = PathBuf::from(base).canonicalize().unwrap();
    let root = base.join("src");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("app.py"), "def legacy():\n    pass\n").unwrap();
    // A malicious ancestor config must not be discovered by a legacy ID opener.
    fs::write(base.join(".kortex.toml"), "[repository]\nroot='missing'\n").unwrap();
    let report =
        kortex_indexer::index_at_root(&root, &Config::default(), &Default::default()).unwrap();
    let registry = Registry::open().unwrap();
    registry.register(&report.snapshot).unwrap();
    assert!(
        registry
            .selection(&report.snapshot.repo_id)
            .unwrap()
            .is_none()
    );
    let service = Service::open_registered(&report.snapshot.repo_id, &root, None).unwrap();
    assert_eq!(service.freshness().unwrap(), "fresh");
    fs::write(root.join(".kortex.toml"), "[repository]\nroot='..'\n").unwrap();
    assert_eq!(
        Service::open_registered(&report.snapshot.repo_id, &root, None)
            .err()
            .unwrap()
            .code,
        "SECURITY_POLICY"
    );
}

#[test]
fn legacy_registered_ids_never_guess_ancestor_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "selection_tests::legacy_selection_child",
            "--nocapture",
        ])
        .env("KORTEX_LEGACY_TEST_ROOT", temp.path())
        .env(
            "KORTEX_REGISTRY",
            temp.path().join("private-registry/registry.db"),
        )
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
