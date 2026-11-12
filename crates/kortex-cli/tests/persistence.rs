//! CLI integration of the persistence owner's public workspace and preservation APIs.
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
fn command(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kortex"));
    c.current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"));
    c
}
#[test]
fn explicit_workspace_indexes_real_repositories_and_deduplicates() {
    let d = support::tempdir().unwrap();
    for repo in ["one", "two"] {
        fs::create_dir(d.path().join(repo)).unwrap();
        fs::write(
            d.path().join(repo).join("app.py"),
            format!("def {repo}():\n    pass\n"),
        )
        .unwrap();
    }
    fs::write(
        d.path().join("workspace.toml"),
        "schema_version = 1\nrepositories = ['one','two','one']\n",
    )
    .unwrap();
    let out = command(d.path())
        .args(["index", "--workspace", "workspace.toml", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["partial"], false);
    assert_eq!(value["items"].as_array().unwrap().len(), 2);
    assert_ne!(
        value["items"][0]["result"]["repo_id"],
        value["items"][1]["result"]["repo_id"]
    );
    let out = command(d.path())
        .args(["repos", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn relocated_effective_root_is_indexed_once_and_read_with_original_config() {
    let d = support::tempdir().unwrap();
    let root = d.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/app.py"), "def inside():\n    pass\n").unwrap();
    fs::write(root.join("outside.py"), "def outside():\n    pass\n").unwrap();
    fs::write(root.join(".kortex.toml"), "[repository]\nroot='src'\n").unwrap();
    let out = command(root)
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let indexed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(indexed["files"], 1);
    assert!(root.join("src/.kortex/index.db").is_file());
    assert!(!root.join(".kortex/index.db").exists());
    let out = command(root)
        .args([
            "--repo",
            root.to_str().unwrap(),
            "status",
            "--require-fresh",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["repo_id"], indexed["repo_id"]);
    assert_eq!(status["index_status"], "fresh");
    assert_eq!(
        status["data"]["root"],
        root.join("src").canonicalize().unwrap().to_str().unwrap()
    );
    let out = command(root)
        .args([
            "--repo",
            indexed["repo_id"].as_str().unwrap(),
            "status",
            "--require-fresh",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "registered-ID selector must preserve the relocated index configuration: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn user_cache_index_status_and_owned_deletion_use_the_same_configured_store() {
    let d = support::tempdir().unwrap();
    let root = d.path();
    let cache = root.join("private-cache");
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&cache).unwrap();
    fs::write(root.join(".gitignore"), "private-cache/\nregistry.db*\n").unwrap();
    fs::write(root.join("app.py"), "def entry():\n    pass\n").unwrap();
    fs::write(root.join(".kortex.toml"), "[index]\nstorage='user-cache'\n").unwrap();
    let cmd = |args: &[&str]| {
        command(root)
            .env("XDG_CACHE_HOME", cache.canonicalize().unwrap())
            .args(args)
            .output()
            .unwrap()
    };
    let out = cmd(&["index", ".", "--json"]);
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = report["repo_id"].as_str().unwrap();
    assert!(!root.join(".kortex/index.db").exists());
    let out = cmd(&["--repo", id, "status", "--require-fresh", "--json"]);
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["repo_id"],
        id
    );
    let out = cmd(&["repos", "remove", id, "--delete-index", "--yes", "--json"]);
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("app.py").is_file());
    let out = cmd(&["repos", "list", "--json"]);
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
#[test]
fn doctor_repair_preserves_actual_corrupt_bytes_and_rebuilds() {
    let d = support::tempdir().unwrap();
    fs::write(d.path().join("app.py"), "def entry():\n    pass\n").unwrap();
    let out = command(d.path())
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let db = d.path().join(".kortex/index.db");
    let corrupt = b"corrupt sqlite bytes preserved exactly";
    fs::write(&db, corrupt).unwrap();
    let out = command(d.path())
        .args([
            "--repo",
            d.path().to_str().unwrap(),
            "doctor",
            "--repair",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let backup = Path::new(report["repair"]["backup_directory"].as_str().unwrap());
    assert_eq!(fs::read(backup.join("index.db")).unwrap(), corrupt);
    assert_eq!(report["data"]["integrity"], "ok");
    assert!(d.path().join("app.py").is_file());
}
