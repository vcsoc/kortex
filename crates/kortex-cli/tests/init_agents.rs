use std::{fs, process::Command};

fn init(root: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kortex"))
        .args(["init", "--json"])
        .env("KORTEX_REGISTRY", root.join("registry/registry.db"))
        .arg(root)
        .output()
        .unwrap()
}

#[test]
fn creates_and_reuses_config_and_instructions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("example.py"),
        "def init_example():\n    return 42\n",
    )
    .unwrap();
    let first = init(dir.path());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(result["config_status"], "created");
    assert_eq!(result["agents_status"], "created");
    assert_eq!(result["index"]["status"], "created");
    let generation = result["index"]["generation_id"].clone();
    assert!(dir.path().join(".kortex/index.db").is_file());
    assert!(result["index"]["symbols"].as_u64().unwrap() > 0);
    let agents = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("index freshness"));
    assert!(agents.contains("fall back to ordinary search"));
    let config = fs::read(dir.path().join(".kortex.toml")).unwrap();
    let second = init(dir.path());
    assert!(second.status.success());
    let result: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(result["config_status"], "existing");
    assert_eq!(result["agents_status"], "current");
    assert_eq!(result["index"]["status"], "existing");
    assert_eq!(result["index"]["generation_id"], generation);
    assert_eq!(fs::read(dir.path().join(".kortex.toml")).unwrap(), config);
    assert_eq!(
        fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
        agents
    );
}

#[test]
fn appends_preserves_and_does_not_update_noninteractively() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("AGENTS.md");
    fs::write(&path, "# Existing\nkeep this").unwrap();
    assert!(init(dir.path()).status.success());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .starts_with("# Existing\nkeep this\n\n")
    );
    let old = "before\n<!-- kortex:instructions:start version=0 -->\nold\n<!-- kortex:instructions:end -->\nafter\n";
    fs::write(&path, old).unwrap();
    let output = init(dir.path());
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["agents_status"], "update_declined");
    assert_eq!(fs::read_to_string(&path).unwrap(), old);
    fs::write(&path, "<!-- kortex:instructions:start broken -->").unwrap();
    assert!(!init(dir.path()).status.success());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "<!-- kortex:instructions:start broken -->"
    );
}

#[test]
fn corrupt_existing_index_is_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    assert!(init(dir.path()).status.success());
    let db = dir.path().join(".kortex/index.db");
    fs::write(&db, b"corrupt index evidence").unwrap();
    let result = init(dir.path());
    assert!(!result.status.success());
    assert_eq!(fs::read(db).unwrap(), b"corrupt index evidence");
}

#[cfg(unix)]
#[test]
fn refuses_symlink_without_touching_target() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "outside").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("AGENTS.md")).unwrap();
    assert!(!init(dir.path()).status.success());
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "outside");
}
