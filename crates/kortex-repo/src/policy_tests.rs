use super::*;
use std::fs;

fn control() -> RequestControl {
    RequestControl::new(std::time::Duration::from_secs(30)).unwrap()
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("safe.py"), "def safe():\n    return 1\n").unwrap();
    (temp, root)
}
fn paths(root: &Path, config: &Config, inputs: &PolicyInputs) -> Result<Vec<String>> {
    Ok(
        discover_authorized_with_control(root, config, inputs, &control())?
            .files
            .into_iter()
            .map(|f| f.path)
            .collect(),
    )
}
fn standard() -> PolicyContext {
    PolicyContext {
        complete: true,
        ..Default::default()
    }
}
fn assert_error(root: &Path, config: &Config, inputs: &PolicyInputs, code: &str) {
    match paths(root, config, inputs) {
        Err(error) => assert_eq!(error.code, code, "{error}"),
        Ok(paths) => panic!("unexpected successful inventory: {paths:?}"),
    }
}

#[test]
fn required_context_is_typed_and_root_only_is_explicit() {
    let (_temp, root) = fixture();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard(None),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(PolicyContext::default())),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    assert_eq!(
        paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap(),
        ["safe.py"]
    );
}

#[cfg(unix)]
#[test]
fn actual_safe_api_refuses_all_outside_policy_aliases_in_both_modes() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("dummy"), "safe.py\n").unwrap();
    for name in [".kortexignore", ".ignore", ".gitignore"] {
        for follow in [false, true] {
            let (_temp, root) = fixture();
            fs::create_dir(root.join(".git")).unwrap();
            let mut config = Config::default();
            config.repository.follow_symlinks = follow;
            fs::write(root.join(name), "safe.py\n").unwrap();
            assert!(
                paths(&root, &config, &PolicyInputs::root_only())
                    .unwrap()
                    .is_empty()
            );
            fs::remove_file(root.join(name)).unwrap();
            symlink(outside.path().join("dummy"), root.join(name)).unwrap();
            assert_error(&root, &config, &PolicyInputs::root_only(), "PATH_REFUSED");
            // Supplying the very same external bytes is NOT a repository alias grant.
            let mut context = standard();
            context.policies.push(AuthorizedPolicy {
                origin: outside.path().join("dummy"),
                category: PolicyCategory::Global,
                matching_base: root.clone(),
                bytes: b"safe.py\n".to_vec(),
            });
            assert_error(
                &root,
                &config,
                &PolicyInputs::standard(Some(context)),
                "PATH_REFUSED",
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn internal_policy_aliases_source_aliases_and_loops() {
    use std::os::unix::fs::symlink;
    for follow in [false, true] {
        let (_temp, root) = fixture();
        fs::write(root.join("policy-data"), "safe.py\n").unwrap();
        symlink("policy-data", root.join(".ignore")).unwrap();
        assert!(
            paths(&root, &Config::default(), &PolicyInputs::root_only())
                .unwrap()
                .is_empty()
        );
        fs::write(root.join("policy-data"), "unrelated.py\n").unwrap();
        symlink("safe.py", root.join("alias.py")).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/child.py"), "x=1\n").unwrap();
        symlink("nested", root.join("aliasdir")).unwrap();
        symlink("..", root.join("nested/loop")).unwrap();
        let mut config = Config::default();
        config.repository.follow_symlinks = follow;
        let inventory = discover_authorized_with_control(
            &root,
            &config,
            &PolicyInputs::root_only(),
            &control(),
        )
        .unwrap();
        assert_eq!(inventory.files.len(), 2);
        assert_eq!(
            inventory
                .files
                .iter()
                .filter(|f| f.source.contains("def safe"))
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn policy_absence_and_real_read_replacements_never_publish_changed_names() {
    use std::os::unix::fs::symlink;
    for action in [0, 1, 2] {
        let (_temp, root) = fixture();
        fs::create_dir(root.join("nested")).unwrap();
        let mut reader = ConfinedReader::new(&root).unwrap();
        let changed = root.clone();
        crate::reader::POLICY_OBSERVATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || match action {
                0 => fs::write(changed.join("nested/.ignore"), "safe.py\n").unwrap(),
                1 => {
                    fs::rename(changed.join("nested"), changed.join("saved")).unwrap();
                    fs::create_dir(changed.join("nested")).unwrap();
                }
                _ => {
                    let moved = changed.with_extension("moved");
                    fs::rename(&changed, &moved).unwrap();
                    fs::create_dir(&changed).unwrap();
                    fs::create_dir(changed.join("nested")).unwrap();
                }
            }))
        });
        assert!(
            reader
                .policy_bytes(Path::new("nested/.ignore"), 1, 100, &control())
                .is_err()
        );
        if action == 2 {
            fs::remove_dir_all(root.with_extension("moved")).unwrap();
        }
    }
    for action in [0, 1, 2] {
        let (_temp, root) = fixture();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/.ignore"), "unrelated.py\n").unwrap();
        let mut reader = ConfinedReader::new(&root).unwrap();
        let changed = root.clone();
        crate::confined::READ_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| match action {
                0 => {
                    fs::rename(changed.join("nested/.ignore"), changed.join("saved-policy"))
                        .unwrap();
                    fs::write(changed.join("nested/.ignore"), "unrelated.py\n").unwrap();
                }
                1 => {
                    fs::rename(changed.join("nested"), changed.join("saved")).unwrap();
                    fs::create_dir(changed.join("nested")).unwrap();
                    fs::write(changed.join("nested/.ignore"), "unrelated.py\n").unwrap();
                }
                _ => {
                    let moved = changed.with_extension("moved");
                    fs::rename(&changed, &moved).unwrap();
                    fs::create_dir(&changed).unwrap();
                }
            }))
        });
        let result = reader.policy_bytes(Path::new("nested/.ignore"), 1, 100, &control());
        crate::confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(result.is_err());
        assert_eq!(reader.stats().bytes_read, 13);
        if action == 2 {
            fs::remove_dir_all(root.with_extension("moved")).unwrap();
        }
    }
    let (_temp, root) = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("dummy-policy"), "safe.py\n").unwrap();
    symlink(outside.path().join("dummy-policy"), root.join(".ignore")).unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    assert!(
        reader
            .policy_bytes(Path::new(".ignore"), 0, 100, &control())
            .is_err()
    );
    assert_eq!(reader.stats().bytes_read, 0);
    assert_eq!(reader.stats().file_opens, 0);
}

#[cfg(windows)]
#[test]
fn windows_hidden_attributes_and_whitelists_use_the_safe_api() {
    use std::{io::Write, os::windows::fs::OpenOptionsExt};
    let (_temp, root) = fixture();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN)
        .open(root.join("attribute-hidden.py"))
        .unwrap();
    file.write_all(b"x=1\n").unwrap();
    drop(file);
    assert_eq!(
        paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap(),
        ["safe.py"]
    );
    fs::write(root.join(".ignore"), "!attribute-hidden.py\n").unwrap();
    assert_eq!(
        paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap(),
        ["attribute-hidden.py", "safe.py"]
    );
}

fn oracle(root: &Path, config: &Config) {
    let legacy = crate::discover(root, config)
        .unwrap()
        .files
        .into_iter()
        .map(|f| f.path)
        .collect::<Vec<_>>();
    let safe = paths(root, config, &PolicyInputs::root_only()).unwrap();
    assert_eq!(safe, legacy);
}

#[test]
fn category_precedence_nearest_last_line_bom_crlf_and_negation_match_locked_oracle() {
    let (_temp, root) = fixture();
    fs::create_dir(root.join(".git")).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    for name in [
        "custom.py",
        "ignore.py",
        "git.py",
        "nearest.py",
        "last.py",
        "kept.py",
    ] {
        fs::write(root.join("nested").join(name), "x=1\n").unwrap();
    }
    fs::write(
        root.join(".kortexignore"),
        b"\xef\xbb\xbfnested/custom.py\r\n!nested/ignore.py\r\n",
    )
    .unwrap();
    fs::write(
        root.join(".ignore"),
        "nested/ignore.py\nnested/nearest.py\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "nested/git.py\n").unwrap();
    fs::write(
        root.join("nested/.ignore"),
        "!custom.py\n!nearest.py\nlast.py\n!last.py\nlast.py\nkept.py\n!kept.py\n",
    )
    .unwrap();
    fs::write(root.join("nested/.gitignore"), "!ignore.py\n!git.py\n").unwrap();
    oracle(&root, &Config::default());
    let result = paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap();
    assert!(!result.contains(&"nested/custom.py".into()));
    assert!(!result.contains(&"nested/last.py".into()));
    for path in [
        "nested/ignore.py",
        "nested/git.py",
        "nested/nearest.py",
        "nested/kept.py",
    ] {
        assert!(result.contains(&path.into()), "{result:?}");
    }
}

#[cfg(unix)]
#[test]
fn ignored_child_does_not_load_rules_or_resurrect_itself_even_at_exhaustion() {
    use std::os::unix::fs::symlink;
    let (_temp, root) = fixture();
    fs::create_dir(root.join("zignored")).unwrap();
    fs::write(root.join(".ignore"), "zignored/\n").unwrap();
    symlink("/no/authority", root.join("zignored/.ignore")).unwrap();
    fs::write(root.join("zignored/child.py"), "x=1\n").unwrap();
    assert_eq!(
        paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap(),
        ["safe.py"]
    );
    fs::write(root.join(".ignore"), "").unwrap();
    // No selected entries follow the failed directory callback. next() can return None.
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "PATH_REFUSED",
    );
}

#[test]
fn hidden_override_includes_default_directories_and_git_required_gating() {
    let (_temp, root) = fixture();
    fs::write(root.join(".hidden.py"), "x=1\n").unwrap();
    fs::create_dir(root.join("target")).unwrap();
    fs::write(root.join("target/no.py"), "x=1\n").unwrap();
    fs::write(root.join(".gitignore"), "safe.py\n").unwrap();
    oracle(&root, &Config::default());
    fs::write(root.join(".ignore"), "!.hidden.py\n").unwrap();
    oracle(&root, &Config::default());
    fs::create_dir(root.join(".git")).unwrap();
    let mut config = Config::default();
    config.repository.include = vec!["*.py".into()];
    oracle(&root, &config);
    assert_eq!(
        paths(&root, &config, &PolicyInputs::root_only()).unwrap(),
        [".hidden.py", "safe.py"]
    );
    config.repository.exclude = vec!["safe.py".into()];
    oracle(&root, &config);
}

#[test]
fn git_exclude_and_nested_repository_boundaries_match_oracle() {
    let (_temp, root) = fixture();
    fs::create_dir_all(root.join(".git/info")).unwrap();
    fs::write(root.join(".git/info/exclude"), "safe.py\nnested/child.py\n").unwrap();
    fs::create_dir_all(root.join("nested/.git")).unwrap();
    fs::write(root.join("nested/child.py"), "x=1\n").unwrap();
    oracle(&root, &Config::default());
    assert_eq!(
        paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap(),
        ["nested/child.py"]
    );
    // A missing optional .git/info directory is not an inaccessible expected parent.
    fs::remove_file(root.join(".git/info/exclude")).unwrap();
    fs::remove_dir(root.join(".git/info")).unwrap();
    oracle(&root, &Config::default());
}

#[test]
fn malformed_text_nul_oversize_budget_and_redacted_diagnostics_abort() {
    let (_temp, root) = fixture();
    for bytes in [
        vec![255],
        b"safe.py\0".to_vec(),
        b"{not-a-valid,private-marker".to_vec(),
    ] {
        fs::write(root.join(".ignore"), bytes).unwrap();
        let error = paths(&root, &Config::default(), &PolicyInputs::root_only()).unwrap_err();
        assert_eq!(error.code, "INVALID_IGNORE_POLICY");
        assert!(!error.message.contains("private-marker"));
    }
    fs::write(
        root.join(".ignore"),
        vec![b'a'; MAX_POLICY_BYTES as usize + 1],
    )
    .unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "FILE_TOO_LARGE",
    );
    fs::write(root.join(".ignore"), vec![b'a'; MAX_LINE_BYTES + 1]).unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_BUDGET_EXCEEDED",
    );
    fs::write(root.join(".ignore"), "#\n".repeat(MAX_LINES + 1)).unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_BUDGET_EXCEEDED",
    );
}

#[cfg(unix)]
#[test]
fn typed_absence_fifo_dangling_loop_permissions_and_stale_parent() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (_temp, root) = fixture();
    let mut reader = ConfinedReader::new(&root).unwrap();
    assert!(
        reader
            .policy_bytes(Path::new(".ignore"), 0, 100, &control())
            .unwrap()
            .is_none()
    );
    assert!(
        reader
            .policy_bytes(Path::new("stale/.ignore"), 1, 100, &control())
            .is_err()
    );
    symlink("missing", root.join(".ignore")).unwrap();
    assert!(
        reader
            .policy_bytes(Path::new(".ignore"), 0, 100, &control())
            .is_err()
    );
    fs::remove_file(root.join(".ignore")).unwrap();
    symlink(".ignore", root.join(".ignore")).unwrap();
    assert!(paths(&root, &Config::default(), &PolicyInputs::root_only()).is_err());
    fs::remove_file(root.join(".ignore")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.join(".ignore"))
            .status()
            .unwrap()
            .success()
    );
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "PATH_REFUSED",
    );
    fs::remove_file(root.join(".ignore")).unwrap();
    fs::create_dir(root.join("denied")).unwrap();
    fs::set_permissions(root.join("denied"), fs::Permissions::from_mode(0o0)).unwrap();
    let error = reader
        .policy_bytes(Path::new("denied/.ignore"), 1, 100, &control())
        .unwrap_err();
    assert_eq!(error.code, "PATH_REFUSED");
    fs::set_permissions(root.join("denied"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn cancellation_before_and_during_policy_read_is_not_a_partial_inventory() {
    let (_temp, root) = fixture();
    let cancelled = control();
    cancelled.cancel();
    assert_eq!(
        discover_authorized_with_control(
            &root,
            &Config::default(),
            &PolicyInputs::root_only(),
            &cancelled
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
    fs::write(root.join(".ignore"), "#a\n".repeat(25000)).unwrap();
    let during = control();
    let cancel = during.clone();
    crate::confined::READ_HOOK
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let error = discover_authorized_with_control(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        &during,
    )
    .err()
    .unwrap();
    crate::confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(error.code, "CANCELLED");
}

#[test]
fn authorized_ancestry_global_bytes_never_open_origins_and_provenance_changes() {
    let (temp, root) = fixture();
    fs::create_dir(root.join(".git")).unwrap();
    let mut context = standard();
    let parent = root.parent().unwrap().to_owned();
    context.ancestors.push(PolicyAncestor {
        directory: parent.clone(),
        repository_boundary: true,
    });
    context.policies.push(AuthorizedPolicy {
        origin: parent.join("nonexistent-ignore-origin"),
        matching_base: parent,
        category: PolicyCategory::Ignore,
        bytes: b"safe.py\n".to_vec(),
    });
    context.policies.push(AuthorizedPolicy {
        origin: temp.path().join("NEVER_OPEN"),
        matching_base: root.clone(),
        category: PolicyCategory::Global,
        bytes: b"unrelated.py\n".to_vec(),
    });
    let (inventory, _, evidence) = discover_authorized_profiled(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(context.clone())),
        &control(),
    )
    .unwrap();
    assert!(inventory.files.is_empty());
    assert!(!evidence.root_only);
    assert!(evidence.evidence.iter().any(|p| p.caller_authorized));
    context.policies[0].bytes = b"unrelated.py\n".to_vec();
    let (inventory, _, changed) = discover_authorized_profiled(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(context)),
        &control(),
    )
    .unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_ne!(changed.digest, evidence.digest);
}

#[test]
fn gitfiles_commondir_and_tracked_names_need_explicit_complete_context() {
    let (_temp, root) = fixture();
    let marker = b"gitdir: /does/not/grant/outside/access\n";
    fs::write(root.join(".git"), marker).unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    let mut context = standard();
    context.repositories.push(PolicyRepository {
        directory: root.clone(),
        external_git_metadata: true,
        marker_digest: Some(kortex_core::hash(marker)),
    });
    context.policies.push(AuthorizedPolicy {
        origin: root.parent().unwrap().join("never-open/exclude"),
        matching_base: root.clone(),
        category: PolicyCategory::GitExclude,
        bytes: b"safe.py\n".to_vec(),
    });
    assert!(
        paths(
            &root,
            &Config::default(),
            &PolicyInputs::standard(Some(context.clone()))
        )
        .unwrap()
        .is_empty()
    );
    fs::write(root.join(".git"), "gitdir: changed\n").unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(context)),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    fs::remove_file(root.join(".git")).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/commondir"), "../outside\n").unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    fs::remove_file(root.join(".git/commondir")).unwrap();
    fs::write(root.join(".git/index"), "not opened").unwrap();
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    let mut context = standard();
    context.tracked_paths = vec!["Foo.py".into(), "foo.py".into()];
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(context)),
        "CASE_COLLISION",
    );
}

#[test]
fn external_global_exclude_and_all_category_overrides_have_exact_precedence() {
    let (_temp, root) = fixture();
    fs::create_dir_all(root.join(".git/info")).unwrap();
    let mut context = standard();
    context.policies.push(AuthorizedPolicy {
        origin: root.parent().unwrap().join("unopened-owner-ignore"),
        matching_base: root.clone(),
        category: PolicyCategory::Global,
        bytes: b"safe.py\n".to_vec(),
    });
    let inputs = PolicyInputs::standard(Some(context));
    assert!(
        paths(&root, &Config::default(), &inputs)
            .unwrap()
            .is_empty()
    );
    fs::write(root.join(".git/info/exclude"), "!safe.py\n").unwrap();
    assert_eq!(
        paths(&root, &Config::default(), &inputs).unwrap(),
        ["safe.py"]
    );
    fs::write(root.join(".gitignore"), "safe.py\n").unwrap();
    assert!(
        paths(&root, &Config::default(), &inputs)
            .unwrap()
            .is_empty()
    );
    fs::write(root.join(".ignore"), "!safe.py\n").unwrap();
    assert_eq!(
        paths(&root, &Config::default(), &inputs).unwrap(),
        ["safe.py"]
    );
    fs::write(root.join(".kortexignore"), "safe.py\n").unwrap();
    assert!(
        paths(&root, &Config::default(), &inputs)
            .unwrap()
            .is_empty()
    );
    let mut config = Config::default();
    config.repository.include = vec!["safe.py".into()];
    assert_eq!(paths(&root, &config, &inputs).unwrap(), ["safe.py"]);
    config.repository.exclude = vec!["safe.py".into()];
    assert!(paths(&root, &config, &inputs).unwrap().is_empty());
}

#[test]
fn aggregate_bytes_and_compilation_work_are_bounded_without_accumulated_clones() {
    let (_temp, root) = fixture();
    let mut state = State::new(&root, &Config::default(), None, &control()).unwrap();
    let comment_block = format!("#{}\n", "a".repeat(MAX_LINE_BYTES - 2)).repeat(256);
    assert_eq!(comment_block.len(), MAX_POLICY_BYTES as usize);
    loop {
        match state.compile(
            &root.join(".ignore"),
            &root,
            PolicyCategory::Ignore,
            comment_block.as_bytes(),
            false,
            &control(),
        ) {
            Ok(matcher) => assert!(matcher.is_empty()),
            Err(error) => {
                assert_eq!(error.code, "POLICY_BUDGET_EXCEEDED");
                break;
            }
        }
    }
    assert!(state.bytes > MAX_TOTAL_BYTES);
    let mut state = State::new(&root, &Config::default(), None, &control()).unwrap();
    for _ in 0..MAX_COMPILATIONS {
        state
            .compile(
                &root.join(".ignore"),
                &root,
                PolicyCategory::Ignore,
                b"",
                false,
                &control(),
            )
            .unwrap();
    }
    let error = state
        .compile(
            &root.join(".ignore"),
            &root,
            PolicyCategory::Ignore,
            b"",
            false,
            &control(),
        )
        .unwrap_err();
    assert_eq!(error.code, "POLICY_BUDGET_EXCEEDED");
}

#[test]
fn active_frames_are_bounded_for_wide_and_deep_trees() {
    let (_temp, root) = fixture();
    for index in 0..100 {
        fs::create_dir(root.join(format!("d{index:03}"))).unwrap();
        fs::write(root.join(format!("d{index:03}/.ignore")), "unrelated.py\n").unwrap();
    }
    let (_, _, provenance) = discover_authorized_profiled(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        &control(),
    )
    .unwrap();
    assert_eq!(provenance.peak_frames, 2);
    assert_eq!(provenance.compiled_policies, 100);
    let mut deep = root.clone();
    for _ in 0..MAX_DEPTH {
        deep.push("d");
        fs::create_dir(&deep).unwrap();
    }
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_BUDGET_EXCEEDED",
    );
}

#[path = "policy_local_tests.rs"]
mod standard_local_tests;
