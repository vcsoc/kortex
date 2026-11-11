use kortex_repo::{read_confined, workspace};

#[test]
fn workspace_manifest_byte_count_utf8_and_nesting_boundaries_are_real() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("repo")).unwrap();
    let path = root.join("workspace.toml");
    let prefix = "schema_version=1\nrepositories=['repo']\n#";
    let exact = format!("{prefix}{}", "x".repeat(1024 * 1024 - prefix.len()));
    std::fs::write(&path, &exact).unwrap();
    assert_eq!(workspace::load(&path).unwrap(), [root.join("repo")]);
    std::fs::write(&path, format!("{exact}x")).unwrap();
    assert_eq!(
        workspace::load(&path).unwrap_err().code,
        "INVALID_WORKSPACE"
    );
    for count in [1024, 1025] {
        let text = format!(
            "schema_version=1\nrepositories=[{}]",
            vec!["'repo'"; count].join(",")
        );
        std::fs::write(&path, text).unwrap();
        let result = workspace::load(&path);
        if count == 1024 {
            assert_eq!(result.unwrap(), [root.join("repo")]);
        } else {
            assert_eq!(result.unwrap_err().code, "INVALID_WORKSPACE");
        }
    }
    for depth in [1, 16, 128, 1024] {
        std::fs::write(
            &path,
            format!(
                "schema_version=1\nrepositories={}{}",
                "[".repeat(depth),
                "]".repeat(depth)
            ),
        )
        .unwrap();
        assert!(workspace::load(&path).is_err());
    }
    std::fs::write(&path, [0xff]).unwrap();
    assert!(workspace::load(&path).is_err());
    std::fs::write(&path, "schema_version=1\nrepositories=['']").unwrap();
    assert_eq!(
        workspace::load(&path).unwrap_err().code,
        "INVALID_WORKSPACE"
    );
}

#[test]
fn real_confined_file_handles_and_generated_escape_paths_are_bounded() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("source.rs"), "safe source").unwrap();
    std::fs::create_dir(root.join("directory.rs")).unwrap();
    assert_eq!(
        read_confined(&root, "directory.rs", 100).unwrap_err().code,
        "PATH_REFUSED"
    );
    for depth in 1..=256 {
        let traversal = format!("{}source.rs", "../".repeat(depth));
        assert_eq!(
            read_confined(&root, &traversal, 100).unwrap_err().code,
            "PATH_REFUSED"
        );
    }
    for length in [0, 1, 2, 31, 1024, 65536] {
        let text = "é".repeat(length);
        std::fs::write(root.join("source.rs"), &text).unwrap();
        let exact = text.len() as u64;
        assert_eq!(read_confined(&root, "source.rs", exact).unwrap(), text);
        assert_eq!(read_confined(&root, "source.rs", u64::MAX).unwrap(), text);
        if exact > 0 {
            assert_eq!(
                read_confined(&root, "source.rs", exact - 1)
                    .unwrap_err()
                    .code,
                "FILE_TOO_LARGE"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn fifo_manifest_is_refused_without_a_writer_or_blocking_read() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("workspace.toml");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        workspace::load(&path).unwrap_err().code,
        "INVALID_WORKSPACE"
    );
}
