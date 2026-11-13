//! Administrative selection must retain provenance even when the index cannot be opened.
mod support;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn command(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kortex"))
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .args(args)
        .output()
        .unwrap()
}
fn fixture() -> (tempfile::TempDir, String) {
    let d = support::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::write(
        d.path().join("src/app.py"),
        "def admin_entry():\n    pass\n",
    )
    .unwrap();
    fs::write(d.path().join("chosen.toml"), "[repository]\nroot='src'\n").unwrap();
    let out = command(
        d.path(),
        &["--config", "chosen.toml", "index", ".", "--json"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    (d, report["repo_id"].as_str().unwrap().to_owned())
}
#[cfg(unix)]
#[test]
fn repository_config_alias_cannot_echo_outside_dummy_contents() {
    let parent = support::tempdir().unwrap();
    let root = parent.path().join("repo");
    fs::create_dir(&root).unwrap();
    let outside = parent.path().join("outside.toml");
    let marker = "OUTSIDE_CLI_CONFIG_DUMMY_MARKER";
    fs::write(&outside, format!("{marker} = [\"unterminated\n")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join(".kortex.toml")).unwrap();
    let output = command(&root, &["config", "show", "--json"]);
    assert_eq!(output.status.code(), Some(8));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(marker));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(marker));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "SECURITY_POLICY");
    assert!(!root.join(".kortex").exists());
}

#[test]
fn registered_config_show_and_path_use_original_explicit_origin_without_opening_index() {
    let (d, id) = fixture();
    // Configuration administration is valid even if the storage is unreadable.
    fs::write(
        d.path().join("src/.kortex/index.db"),
        b"corrupt db for config-only operation",
    )
    .unwrap();
    let show = command(d.path(), &["--repo", &id, "config", "show", "--json"]);
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stdout)
    );
    let show: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert!(
        command(d.path(), &["--repo", &id, "config", "validate", "--json"])
            .status
            .success()
    );
    assert_eq!(
        show["repository"]["root"], "src",
        "registered config must not silently become effective-root defaults"
    );
    let paths = command(d.path(), &["--repo", &id, "config", "path", "--json"]);
    assert!(paths.status.success());
    let paths: Value = serde_json::from_slice(&paths.stdout).unwrap();
    assert_eq!(
        paths["repository"],
        d.path().join("chosen.toml").to_str().unwrap()
    );
}
#[test]
fn registered_corrupt_repair_preserves_explicit_origin_and_freshness() {
    let (d, id) = fixture();
    let db = d.path().join("src/.kortex/index.db");
    let corrupt = b"registered explicit-config database corruption";
    fs::write(&db, corrupt).unwrap();
    let repaired = command(d.path(), &["--repo", &id, "doctor", "--repair", "--json"]);
    assert!(
        repaired.status.success(),
        "{}",
        String::from_utf8_lossy(&repaired.stdout)
    );
    let repaired: Value = serde_json::from_slice(&repaired.stdout).unwrap();
    let backup = repaired["repair"]["backup_directory"].as_str().unwrap();
    assert_eq!(
        fs::read(Path::new(backup).join("index.db")).unwrap(),
        corrupt
    );
    let original = command(
        d.path(),
        &[
            "--repo",
            d.path().to_str().unwrap(),
            "status",
            "--require-fresh",
            "--config",
            "chosen.toml",
            "--json",
        ],
    );
    assert!(
        original.status.success(),
        "repair must preserve originally selected configuration, not silently rebuild with defaults: {}",
        String::from_utf8_lossy(&original.stdout)
    );
    let by_id = command(
        d.path(),
        &["--repo", &id, "status", "--require-fresh", "--json"],
    );
    assert!(
        by_id.status.success(),
        "{}",
        String::from_utf8_lossy(&by_id.stdout)
    );
}
#[test]
fn administrative_registered_redirection_is_refused_before_corrupt_bytes_are_touched() {
    let (d, id) = fixture();
    let db = d.path().join("src/.kortex/index.db");
    let corrupt = b"must remain unchanged on unauthorized administrative selection";
    fs::write(&db, corrupt).unwrap();
    fs::create_dir(d.path().join("unauthorized")).unwrap();
    fs::write(
        d.path().join("chosen.toml"),
        "[repository]\nroot='unauthorized'\n",
    )
    .unwrap();
    for arguments in [
        vec!["config", "show"],
        vec!["config", "validate"],
        vec!["config", "path"],
        vec!["doctor", "--repair"],
    ] {
        let mut args = vec!["--repo", id.as_str(), "--json"];
        args.extend(arguments);
        let response = command(d.path(), &args);
        assert_eq!(
            response.status.code(),
            Some(8),
            "{}",
            String::from_utf8_lossy(&response.stdout)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&response.stdout).unwrap()["error"]["code"],
            "REPO_FORBIDDEN"
        );
        assert_eq!(fs::read(&db).unwrap(), corrupt);
        assert!(!d.path().join("unauthorized/.kortex").exists());
    }
}
#[test]
fn config_administration_does_not_require_a_registered_or_readable_index() {
    let d = support::tempdir().unwrap();
    fs::write(d.path().join("chosen.toml"), "[analysis]\nmax_nodes=37\n").unwrap();
    let show = command(
        d.path(),
        &["--config", "chosen.toml", "config", "show", "--json"],
    );
    assert!(show.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&show.stdout).unwrap()["analysis"]["max_nodes"],
        37
    );
    assert!(
        command(
            d.path(),
            &["--config", "chosen.toml", "config", "validate", "--json"]
        )
        .status
        .success()
    );
    let paths = command(
        d.path(),
        &["--config", "chosen.toml", "config", "path", "--json"],
    );
    assert!(paths.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&paths.stdout).unwrap()["repository"],
        d.path().join("chosen.toml").to_str().unwrap()
    );
    assert!(!d.path().join(".kortex").exists());
    assert!(!d.path().join("registry.db").exists());
}
