//! Executable checks for filter, freshness, generation and response-budget options.
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kortex"))
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}
#[test]
fn filters_cursors_freshness_generation_and_budgets_reach_the_shared_service() {
    let d = support::tempdir().unwrap();
    let root = d.path();
    fs::write(root.join(".gitignore"), ".kortex/\nregistry.db*\n").unwrap();
    fs::write(
        root.join("app.py"),
        "def alpha():\n    pass\ndef beta():\n    alpha()\ndef gamma():\n    beta()\n",
    )
    .unwrap();
    assert!(run(root, &["index", "."]).status.success());
    let query = [
        "query",
        "app.py",
        "--kind",
        "function",
        "--language",
        "python",
        "--path-prefix",
        "app.py",
        "--limit",
        "1",
        "--max-response-bytes",
        "8192",
        "--timeout-ms",
        "10000",
        "--require-fresh",
    ];
    let out = run(root, &query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let first: Value = serde_json::from_slice(&out.stdout).unwrap();
    let hit = &first["data"]["items"][0]["symbol"];
    assert_eq!(hit["kind"], "function");
    assert_eq!(hit["language"], "python");
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    assert!(cursor.starts_with("kx1."));
    let mut next = query.to_vec();
    next.extend(["--cursor", cursor]);
    let out = run(root, &next);
    assert!(out.status.success());
    let second: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_ne!(first["data"]["items"][0], second["data"]["items"][0]);
    let out = run(root, &["query", "app.py", "--generation-id", "wrong"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "GENERATION_MISMATCH"
    );
    let out = run(root, &["query", "app.py", "--cursor", "1"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "INVALID_CURSOR"
    );
    let out = run(root, &["context", "beta", "--max-response-bytes", "1024"]);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "RESPONSE_BUDGET"
    );
    fs::write(root.join("app.py"), "def changed():\n    pass\n").unwrap();
    let out = run(root, &["query", "app.py", "--require-fresh"]);
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "STALE_INDEX"
    );
    let out = run(root, &["--timeout-ms", "1", "mcp", "--unknown"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}
