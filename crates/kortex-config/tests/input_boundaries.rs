use kortex_config::Config;

#[test]
fn configuration_byte_boundary_utf8_and_deep_malformed_inputs_fail_closed() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("trusted.toml");
    let exact = format!("#{}", "x".repeat(1024 * 1024 - 1));
    std::fs::write(&path, &exact).unwrap();
    Config::load(d.path(), Some(&path)).unwrap();
    std::fs::write(&path, format!("{exact}x")).unwrap();
    assert!(Config::load(d.path(), Some(&path)).is_err());
    for depth in [1, 16, 128, 1024] {
        // Real TOML deserialization, no synthetic success or indexed code execution.
        let text = format!("schema_version={}1{}", "[".repeat(depth), "]".repeat(depth));
        std::fs::write(&path, text).unwrap();
        assert!(Config::load(d.path(), Some(&path)).is_err());
    }
    for bytes in [vec![0xff], vec![0xc3, 0x28], b"schema_version=1\0".to_vec()] {
        std::fs::write(&path, bytes).unwrap();
        assert!(Config::load(d.path(), Some(&path)).is_err());
    }
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn fifo_and_directory_config_are_refused_and_trusted_regular_alias_still_works() {
    let d = tempfile::tempdir().unwrap();
    let fifo = d.path().join("fifo.toml");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(Config::load(d.path(), Some(&fifo)).is_err());
    assert!(Config::load(d.path(), Some(d.path())).is_err());
    let regular = d.path().join("trusted.toml");
    let alias = d.path().join("alias.toml");
    std::fs::write(&regular, "schema_version=1").unwrap();
    std::os::unix::fs::symlink(&regular, &alias).unwrap();
    Config::load(d.path(), Some(&alias)).unwrap();
    assert_eq!(
        std::fs::read_to_string(regular).unwrap(),
        "schema_version=1"
    );
}
