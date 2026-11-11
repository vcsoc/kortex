use super::*;

#[test]
fn discovery_reports_limits_filters_and_unreadable_source_types() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("a.py"), "a=1\n").unwrap();
    fs::write(root.join("b.rs"), "fn b(){}\n").unwrap();
    fs::write(root.join("binary.py"), [0]).unwrap();
    fs::write(root.join("encoding.py"), [0xff]).unwrap();
    fs::write(root.join("unknown.txt"), "unindexed").unwrap();
    let inventory = discover(&root, &Config::default()).unwrap();
    assert_eq!(
        inventory
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["a.py", "b.rs"]
    );
    for reason in ["BINARY_FILE", "INVALID_ENCODING", "unsupported language"] {
        assert!(
            inventory.diagnostics.iter().any(|d| d.contains(reason)),
            "{reason}: {:?}",
            inventory.diagnostics
        );
    }
    let mut config = Config::default();
    config.repository.max_files = 1;
    let inventory = discover(&root, &config).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].path, "a.py");
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|d| d.contains("max_files reached"))
    );
    config.repository.max_files = 100;
    config.repository.max_total_bytes = 4;
    let inventory = discover(&root, &config).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].source, "a=1\n");
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|d| d.contains("max_total_bytes reached"))
    );
    config.repository.max_total_bytes = 100;
    config.repository.languages = vec!["rust".into()];
    let inventory = discover(&root, &config).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].path, "b.rs");
    config.repository.include = vec!["[".into()];
    assert_eq!(discover(&root, &config).err().unwrap().code, "INVALID_GLOB");
    config.repository.include.clear();
    config.repository.exclude = vec!["[".into()];
    assert_eq!(discover(&root, &config).err().unwrap().code, "INVALID_GLOB");
}

#[test]
fn confined_read_handles_limit_boundaries_without_overflow() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("source.py"), "é").unwrap();
    assert_eq!(read_confined(&root, "source.py", u64::MAX).unwrap(), "é");
    assert_eq!(read_confined(&root, "source.py", 2).unwrap(), "é");
    assert_eq!(
        read_confined(&root, "source.py", 1).unwrap_err().code,
        "FILE_TOO_LARGE"
    );
    assert_eq!(
        read_confined(&root, "source.py", 0).unwrap_err().code,
        "FILE_TOO_LARGE"
    );
    fs::write(root.join("empty.py"), "").unwrap();
    assert_eq!(read_confined(&root, "empty.py", 0).unwrap(), "");
}

#[test]
fn resolution_inputs_change_fingerprint_but_source_does_not() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let original = inputs::resolution_fingerprint(&root);
    fs::write(root.join("source.py"), "x=1\n").unwrap();
    assert_eq!(inputs::resolution_fingerprint(&root), original);
    fs::write(root.join("go.mod"), "module example.invalid/one\n").unwrap();
    let module = inputs::resolution_fingerprint(&root);
    assert_ne!(module, original);
    fs::write(root.join("go.mod"), "module example.invalid/two\n").unwrap();
    assert_ne!(inputs::resolution_fingerprint(&root), module);
    fs::write(root.join(".kortexignore"), "source.py\n").unwrap();
    let ignored = inputs::resolution_fingerprint(&root);
    fs::write(root.join(".kortexignore"), "other.py\n").unwrap();
    assert_ne!(inputs::resolution_fingerprint(&root), ignored);
}

#[cfg(unix)]
#[test]
fn skipped_alias_does_not_hide_a_supported_source_file() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("source.py"), "x=1\n").unwrap();
    std::os::unix::fs::symlink("source.py", root.join("a.txt")).unwrap();
    std::os::unix::fs::symlink("source.py", root.join("b.rs")).unwrap();
    let mut config = Config::default();
    config.repository.follow_symlinks = true;
    config.repository.languages = vec!["python".into()];
    let inventory = discover(&root, &config).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].path, "source.py");
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|d| d.contains("a.txt: unsupported language"))
    );
}

#[cfg(unix)]
#[test]
fn internal_links_are_deduplicated_and_cycles_do_not_escape() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("source.py"), "x=1\n").unwrap();
    std::os::unix::fs::symlink("source.py", root.join("alias.py")).unwrap();
    std::os::unix::fs::symlink(".", root.join("cycle")).unwrap();
    let inventory = discover(&root, &Config::default()).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].path, "source.py");
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|d| d.contains("symlink skipped"))
    );
    let mut config = Config::default();
    config.repository.follow_symlinks = true;
    let inventory = discover(&root, &config).unwrap();
    assert_eq!(inventory.files.len(), 1);
    assert_eq!(inventory.files[0].source, "x=1\n");
    assert!(inventory.diagnostics.iter().any(|d| d.contains("loop")));
}
