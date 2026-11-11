use super::*;

fn init(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "--quiet", "--template="]).unwrap();
}

fn seed(path: &Path) {
    git(
        path,
        &[
            "-c",
            "user.name=Kortex Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "--no-verify",
            "-m",
            "seed",
        ],
    )
    .unwrap();
}

#[test]
fn configured_worktree_cannot_expand_into_markerless_parent() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    init(&repo);
    fs::write(d.path().join("private.py"), "SECRET = 1\n").unwrap();
    git(
        &repo,
        &["config", "core.worktree", d.path().to_str().unwrap()],
    )
    .unwrap();
    let reported = git(&repo, &["rev-parse", "--show-toplevel"]).unwrap();
    assert_eq!(
        Path::new(reported.trim_end_matches('\n')),
        d.path().canonicalize().unwrap()
    );
    assert!(!d.path().join(".git").exists());
    let error = root(&repo).expect_err("unsafe ancestor expansion accepted");
    assert_eq!(error.code, "PATH_REFUSED");
    assert_eq!(error.exit, 8);
    assert_eq!(
        fs::read_to_string(d.path().join("private.py")).unwrap(),
        "SECRET = 1\n"
    );
}

#[test]
fn configured_worktree_cannot_expand_into_an_unrelated_parent_repository() {
    let d = tempfile::tempdir().unwrap();
    init(d.path());
    let repo = d.path().join("nested-repository");
    init(&repo);
    git(
        &repo,
        &["config", "core.worktree", d.path().to_str().unwrap()],
    )
    .unwrap();
    assert!(d.path().join(".git").is_dir());
    assert_eq!(root(&repo).err().unwrap().code, "PATH_REFUSED");
}

#[test]
fn normal_linked_and_submodule_worktrees_remain_supported() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    init(&repo);
    seed(&repo);
    fs::create_dir(repo.join("nested")).unwrap();
    assert_eq!(
        root(&repo.join("nested")).unwrap(),
        repo.canonicalize().unwrap()
    );

    let linked = d.path().join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    )
    .unwrap();
    assert!(linked.join(".git").is_file());
    fs::create_dir(linked.join("nested")).unwrap();
    assert_eq!(
        root(&linked.join("nested")).unwrap(),
        linked.canonicalize().unwrap()
    );

    git(
        &repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            repo.to_str().unwrap(),
            "module",
        ],
    )
    .unwrap();
    let module = repo.join("module");
    assert!(module.join(".git").is_file());
    fs::create_dir(module.join("nested")).unwrap();
    assert_eq!(
        root(&module.join("nested")).unwrap(),
        module.canonicalize().unwrap()
    );
}
