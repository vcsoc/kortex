use super::*;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn local() -> PolicyInputs {
    // Fixture attestation only, not a production acquisition shortcut.
    PolicyInputs::standard_local(Some(standard()))
}

fn git(root: &Path, home: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_CONFIG_SYSTEM", null)
        .env("GIT_TEMPLATE_DIR", home)
        .env("LC_ALL", "C");
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", system_root);
    }
    let mut child = command
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            &format!("core.hooksPath={null}"),
            "-C",
        ])
        .arg(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn index(root: &Path, names: &[&str]) -> Vec<u8> {
    let home = tempfile::tempdir().unwrap();
    git(root, home.path(), &["init", "--quiet"], b"");
    let oid = String::from_utf8(git(
        root,
        home.path(),
        &["hash-object", "-w", "--stdin"],
        b"inert fixture bytes\n",
    ))
    .unwrap();
    let mut input = Vec::new();
    for name in names {
        write!(input, "100644 {}\t{name}\0", oid.trim()).unwrap();
    }
    if names.is_empty() {
        git(root, home.path(), &["read-tree", "--empty"], b"");
    } else {
        git(
            root,
            home.path(),
            &["update-index", "-z", "--index-info"],
            &input,
        );
    }
    fs::read(root.join(".git/index")).unwrap()
}

#[test]
fn ordinary_git_index_is_demand_acquired_only_in_the_new_mode() {
    let (_temp, root) = fixture();
    let bytes = index(&root, &["safe.py"]);
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::root_only(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    let (_, _, old) = discover_authorized_profiled(
        &root,
        &Config::default(),
        &PolicyInputs::standard(Some(standard())),
        &control(),
    )
    .unwrap();
    let (inventory, stats, provenance) =
        discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(stats.reader.root_opens, 1);
    assert_eq!(old.version, 1);
    assert_eq!(provenance.version, 2);
    assert!(!provenance.root_only);
    assert_ne!(old.digest, provenance.digest);
    assert!(
        provenance
            .evidence
            .iter()
            .any(|e| e.origin == root.join(".git/index")
                && e.digest.as_deref() == Some(kortex_core::hash(&bytes).as_str())
                && !e.caller_authorized)
    );
    index(&root, &["another.py"]);
    let (_, _, changed) =
        discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
    assert_ne!(changed.digest, provenance.digest);
}

#[test]
fn context_must_be_complete_and_must_not_predeclare_repositories() {
    let (_temp, root) = fixture();
    for context in [None, Some(PolicyContext::default())] {
        assert_error(
            &root,
            &Config::default(),
            &PolicyInputs::standard_local(context),
            "POLICY_AUTHORIZATION_REQUIRED",
        );
    }
    let mut context = standard();
    context.repositories.push(PolicyRepository {
        directory: root.clone(),
        external_git_metadata: false,
        marker_digest: None,
    });
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard_local(Some(context)),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    let mut context = standard();
    context.policies.push(AuthorizedPolicy {
        origin: root.join("NEVER_OPEN"),
        matching_base: root.clone(),
        category: PolicyCategory::Ignore,
        bytes: b"safe.py\n".to_vec(),
    });
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard_local(Some(context)),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    let mut context = standard();
    let parent = root.parent().unwrap().to_owned();
    context.ancestors.push(PolicyAncestor {
        directory: parent.clone(),
        repository_boundary: true,
    });
    context.policies.push(AuthorizedPolicy {
        origin: parent.join("NEVER_OPEN"),
        matching_base: parent,
        category: PolicyCategory::Ignore,
        bytes: b"safe.py\n".to_vec(),
    });
    assert!(
        paths(
            &root,
            &Config::default(),
            &PolicyInputs::standard_local(Some(context))
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn ignored_nested_metadata_is_neither_loaded_nor_decoded_and_terminal_failure_aborts() {
    for gitfile in [false, true] {
        let (_temp, root) = fixture();
        fs::create_dir(root.join("zignored")).unwrap();
        if gitfile {
            fs::write(root.join("zignored/.git"), "gitdir: /outside/never-open\n").unwrap();
        } else {
            index(&root.join("zignored"), &["child.py"]);
            fs::write(root.join("zignored/.git/index"), "malformed index").unwrap();
        }
        fs::write(root.join(".ignore"), "zignored/\n").unwrap();
        let (inventory, _, provenance) =
            discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
        assert_eq!(inventory.files.len(), 1);
        assert!(
            !provenance
                .evidence
                .iter()
                .any(|e| e.origin.starts_with(root.join("zignored")))
        );
        fs::write(root.join(".ignore"), "").unwrap();
        // zignored is last: its failed callback can make next() return None.
        let error = discover_authorized_profiled(&root, &Config::default(), &local(), &control())
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            if gitfile {
                "POLICY_AUTHORIZATION_REQUIRED"
            } else {
                "GIT_INDEX_CORRUPT"
            }
        );
    }
}

#[test]
fn nested_names_are_rebased_and_checked_with_inherited_names_not_current_files() {
    let (_temp, root) = fixture();
    fs::create_dir(root.join("nested")).unwrap();
    index(&root, &["nested/Foo.py"]);
    index(&root.join("nested"), &["foo.py"]);
    assert_error(&root, &Config::default(), &local(), "CASE_COLLISION");
    fs::remove_file(root.join(".git/index")).unwrap();
    let mut context = standard();
    context.tracked_paths = vec!["nested/FOO.py".into()];
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard_local(Some(context)),
        "CASE_COLLISION",
    );
    let mut context = standard();
    context.tracked_paths = vec!["nested/foo.py".into(), "nested/foo.py".into()];
    assert_eq!(
        paths(
            &root,
            &Config::default(),
            &PolicyInputs::standard_local(Some(context))
        )
        .unwrap(),
        ["safe.py"]
    );
    fs::remove_file(root.join("nested/.git/index")).unwrap();
    index(&root.join("nested"), &["k.py"]);
    let mut context = standard();
    context.tracked_paths = vec!["nested/\u{212a}.py".into()];
    assert_error(
        &root,
        &Config::default(),
        &PolicyInputs::standard_local(Some(context)),
        "CASE_COLLISION",
    );
    fs::remove_file(root.join("nested/.git/index")).unwrap();
    index(&root, &["SAFE.py"]);
    // Old tracked spelling and current safe.py are intentionally not compared.
    assert_eq!(
        paths(&root, &Config::default(), &local()).unwrap(),
        ["safe.py"]
    );
}

#[test]
fn actual_git_conflict_stage_duplicates_are_preserved() {
    let (_temp, root) = fixture();
    index(&root, &["safe.py"]);
    let home = tempfile::tempdir().unwrap();
    let oid = String::from_utf8(git(
        &root,
        home.path(),
        &["hash-object", "-w", "--stdin"],
        b"conflict bytes\n",
    ))
    .unwrap();
    let mut input = format!("0 {}\tsafe.py\0", "0".repeat(40));
    for stage in 1..=3 {
        input.push_str(&format!("100644 {} {stage}\tsafe.py\0", oid.trim()));
    }
    git(
        &root,
        home.path(),
        &["update-index", "-z", "--index-info"],
        input.as_bytes(),
    );
    let decoded =
        crate::git_index::decode(&fs::read(root.join(".git/index")).unwrap(), None).unwrap();
    assert_eq!(decoded.paths, ["safe.py", "safe.py", "safe.py"]);
    assert_eq!(
        paths(&root, &Config::default(), &local()).unwrap(),
        ["safe.py"]
    );
}

#[test]
fn missing_index_is_observed_absence_but_nonregular_index_is_not() {
    let (_temp, root) = fixture();
    fs::create_dir(root.join(".git")).unwrap();
    let (_, _, provenance) =
        discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
    assert!(
        provenance
            .evidence
            .iter()
            .any(|e| e.origin == root.join(".git/index") && e.digest.is_none())
    );
    fs::create_dir(root.join(".git/index")).unwrap();
    assert_error(&root, &Config::default(), &local(), "PATH_REFUSED");
    fs::remove_dir(root.join(".git/index")).unwrap();
    index(&root, &[]);
    let (_, _, present) =
        discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
    assert_ne!(present.digest, provenance.digest);
    fs::write(root.join(".git/commondir"), "../never-follow\n").unwrap();
    assert_error(
        &root,
        &Config::default(),
        &local(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
}

#[cfg(unix)]
#[test]
fn aliased_metadata_dangling_outside_and_lexical_selected_aliases() {
    use std::os::unix::fs::symlink;
    let (_temp, root) = fixture();
    let bytes = index(&root, &["safe.py"]);
    fs::remove_file(root.join(".git/index")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("index"), &bytes).unwrap();
    fs::write(root.join("index-copy"), &bytes).unwrap();
    for target in [
        root.join("missing"),
        outside.path().join("index"),
        root.join("index-copy"),
    ] {
        symlink(target, root.join(".git/index")).unwrap();
        assert_error(&root, &Config::default(), &local(), "PATH_REFUSED");
        fs::remove_file(root.join(".git/index")).unwrap();
    }
    fs::rename(root.join(".git"), root.join("git-copy")).unwrap();
    symlink("git-copy", root.join(".git")).unwrap();
    assert_error(
        &root,
        &Config::default(),
        &local(),
        "POLICY_AUTHORIZATION_REQUIRED",
    );
    fs::remove_file(root.join(".git")).unwrap();
    fs::remove_dir_all(root.join("git-copy")).unwrap();
    fs::create_dir(root.join("zreal")).unwrap();
    index(&root.join("zreal"), &["foo.py"]);
    symlink("zreal", root.join("Alias")).unwrap();
    fs::write(root.join(".ignore"), "zreal/\n").unwrap();
    let mut config = Config::default();
    config.repository.follow_symlinks = true;
    let mut context = standard();
    context.tracked_paths = vec!["Alias/FOO.py".into()];
    assert_error(
        &root,
        &config,
        &PolicyInputs::standard_local(Some(context)),
        "CASE_COLLISION",
    );
}

#[test]
fn malformed_unsupported_size_and_control_errors_propagate_and_retry_is_fresh() {
    let (_temp, root) = fixture();
    let bytes = index(&root, &["safe.py"]);
    for (version, code) in [(0u32, "GIT_INDEX_UNSUPPORTED"), (2, "GIT_INDEX_CORRUPT")] {
        let mut bad = bytes.clone();
        bad[4..8].copy_from_slice(&version.to_be_bytes());
        if version == 2 {
            bad[0] = b'!';
        }
        fs::write(root.join(".git/index"), bad).unwrap();
        assert_error(&root, &Config::default(), &local(), code);
    }
    let file = fs::File::create(root.join(".git/index")).unwrap();
    file.set_len(MAX_TRACKED_BYTES as u64 + 1).unwrap();
    drop(file);
    assert_error(&root, &Config::default(), &local(), "FILE_TOO_LARGE");
    fs::write(root.join(".git/index"), &bytes).unwrap();
    let during = control();
    let cancel = during.clone();
    crate::confined::READ_HOOK
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let error = discover_authorized_profiled(&root, &Config::default(), &local(), &during)
        .err()
        .unwrap();
    crate::confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(
        paths(&root, &Config::default(), &local()).unwrap(),
        ["safe.py"]
    );
}

#[test]
fn aggregate_remaining_budgets_bound_actual_index_reads_and_names_separately() {
    let (_temp, root) = fixture();
    let bytes = index(&root, &["safe.py"]);
    let context = standard();
    let mut state =
        State::with_local(&root, &Config::default(), Some(&context), &control(), true).unwrap();
    state.index_bytes = MAX_TRACKED_BYTES - bytes.len();
    state
        .tracked_index(&root, Path::new(".git"), 0, &control())
        .unwrap();
    assert_eq!(state.index_bytes, MAX_TRACKED_BYTES);
    let before = state.reader.stats().bytes_read;
    let error = state
        .tracked_index(&root, Path::new(".git"), 0, &control())
        .unwrap_err();
    assert_eq!(error.code, "FILE_TOO_LARGE");
    // The retained reader uses one oversize-detection byte, not an unbounded read.
    assert_eq!(state.reader.stats().bytes_read, before + 1);
    for (name_bytes, names) in [(MAX_TRACKED_BYTES - 1, 0), (0, MAX_TRACKED_NAMES)] {
        let mut state =
            State::with_local(&root, &Config::default(), Some(&context), &control(), true).unwrap();
        state.tracked_bytes = name_bytes;
        state.tracked_names = names;
        assert_eq!(
            state
                .tracked_index(&root, Path::new(".git"), 0, &control())
                .unwrap_err()
                .code,
            "POLICY_BUDGET_EXCEEDED"
        );
    }
}

#[test]
fn selected_repositories_and_inherited_names_share_one_request_budget() {
    let (_temp, root) = fixture();
    let root_index = index(&root, &["safe.py"]);
    fs::create_dir(root.join("nested")).unwrap();
    let nested_index = index(&root.join("nested"), &["child.py"]);
    let mut context = standard();
    context.tracked_paths = vec!["inherited.py".into()];
    let mut state =
        State::with_local(&root, &Config::default(), Some(&context), &control(), true).unwrap();
    state.push_directory(&root, &control()).unwrap();
    state
        .push_directory(&root.join("nested"), &control())
        .unwrap();
    assert_eq!(state.index_bytes, root_index.len() + nested_index.len());
    assert_eq!(state.tracked_names, 3);
    assert_eq!(
        state.tracked_bytes,
        "inherited.py".len() + "safe.py".len() + "nested/child.py".len()
    );
    assert_eq!(
        state.tracked_cases.get("nested/child.py").unwrap(),
        "nested/child.py"
    );
    let mut state =
        State::with_local(&root, &Config::default(), Some(&context), &control(), true).unwrap();
    state.index_bytes = MAX_TRACKED_BYTES - root_index.len() - nested_index.len() + 1;
    state.push_directory(&root, &control()).unwrap();
    assert_eq!(
        state
            .push_directory(&root.join("nested"), &control())
            .unwrap_err()
            .code,
        "FILE_TOO_LARGE"
    );
    assert_eq!(
        paths(&root, &Config::default(), &local()).unwrap(),
        ["safe.py"]
    );
}

#[cfg(unix)]
#[test]
fn actual_index_read_refuses_leaf_parent_and_root_replacements() {
    for action in 0..3 {
        let (_temp, root) = fixture();
        index(&root, &["safe.py"]);
        let changed = root.clone();
        crate::confined::READ_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| match action {
                0 => {
                    fs::rename(changed.join(".git/index"), changed.join(".git/saved-index"))
                        .unwrap();
                    fs::copy(changed.join(".git/saved-index"), changed.join(".git/index")).unwrap();
                }
                1 => {
                    fs::rename(changed.join(".git"), changed.join("saved-git")).unwrap();
                    fs::create_dir(changed.join(".git")).unwrap();
                    fs::copy(changed.join("saved-git/index"), changed.join(".git/index")).unwrap();
                }
                _ => {
                    fs::rename(&changed, changed.with_extension("moved")).unwrap();
                    fs::create_dir(&changed).unwrap();
                }
            }));
        });
        let error = discover_authorized_profiled(&root, &Config::default(), &local(), &control())
            .err()
            .unwrap();
        crate::confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(error.code, "PATH_REFUSED");
        if action == 2 {
            fs::remove_dir_all(root.with_extension("moved")).unwrap();
        }
    }
}

#[test]
fn actual_split_index_refuses_instead_of_publishing_partial_names() {
    let (_temp, root) = fixture();
    let bytes = index(&root, &["safe.py"]);
    let home = tempfile::tempdir().unwrap();
    git(&root, home.path(), &["update-index", "--split-index"], b"");
    assert_error(&root, &Config::default(), &local(), "GIT_INDEX_UNSUPPORTED");
    fs::write(root.join(".git/index"), bytes).unwrap();
    assert_eq!(
        paths(&root, &Config::default(), &local()).unwrap(),
        ["safe.py"]
    );
}

#[test]
fn one_hundred_thousand_tracked_names_do_not_consume_the_policy_budget() {
    let (_temp, root) = fixture();
    let names: Vec<_> = (0..100_000)
        .map(|n| {
            format!("tracked/long-enough-directory-name-to-cross-the-policy-budget/file{n:06}.py")
        })
        .collect();
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    let bytes = index(&root, &refs);
    assert!(bytes.len() > MAX_TOTAL_BYTES);
    let (inventory, _, provenance) =
        discover_authorized_profiled(&root, &Config::default(), &local(), &control()).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert!(provenance.policy_bytes < MAX_POLICY_BYTES as usize);
}
