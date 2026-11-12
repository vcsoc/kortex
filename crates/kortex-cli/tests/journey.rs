//! Production-binary integration tests. Synthetic code is data, never executed.
mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kortex")
}
fn run(root: &Path, args: &[&str]) -> Value {
    let out = Command::new(binary())
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn fixture() -> tempfile::TempDir {
    let d = support::tempdir().unwrap();
    fs::write(d.path().join(".gitignore"), ".kortex/\nregistry.db*\n").unwrap();
    fs::write(
        d.path().join("app.py"),
        "from auth import check\ndef entry():\n    check()\ndef test_entry():\n    entry()\n",
    )
    .unwrap();
    fs::write(d.path().join("auth.py"), "def check():\n    pass\n").unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::create_dir(d.path().join("auth")).unwrap();
    for (path, source) in [
        (
            "src/lib.rs",
            include_str!("../../../fixtures/polyglot/src/lib.rs"),
        ),
        (
            "src/security.rs",
            include_str!("../../../fixtures/polyglot/src/security.rs"),
        ),
        (
            "main.ts",
            include_str!("../../../fixtures/polyglot/main.ts"),
        ),
        (
            "security.ts",
            include_str!("../../../fixtures/polyglot/security.ts"),
        ),
        (
            "main.go",
            include_str!("../../../fixtures/polyglot/main.go"),
        ),
        (
            "auth/auth.go",
            include_str!("../../../fixtures/polyglot/auth/auth.go"),
        ),
        ("go.mod", include_str!("../../../fixtures/polyglot/go.mod")),
    ] {
        fs::write(d.path().join(path), source).unwrap();
    }
    let out = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    git(d.path(), &["add", "."]);
    git(
        d.path(),
        &[
            "-c",
            "user.name=Kortex Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    run(d.path(), &["index", ".", "--verify"]);
    d
}
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn complete_cli_core_journey() {
    let d = fixture();
    let p = d.path();
    let q = run(p, &["query", "check", "--limit", "1"]);
    assert_eq!(q["data"]["items"][0]["symbol"]["name"], "check");
    assert_eq!(q["index_status"], "fresh");
    let c = run(p, &["context", "check"]);
    assert_eq!(c["data"]["callers"].as_array().unwrap().len(), 1);
    assert_eq!(c["data"]["symbol"]["span"]["start_line"], 1);
    let impact = run(p, &["impact", "check", "--depth", "3"]);
    assert_eq!(impact["data"]["direct"], 1);
    assert_eq!(impact["data"]["indirect"], 1);
    assert_eq!(impact["data"]["likely_tests"].as_array().unwrap().len(), 1);
    let trace = run(p, &["trace", "--from", "test_entry", "--to", "check"]);
    assert_eq!(trace["data"]["found"], true);
    assert_eq!(trace["data"]["path"].as_array().unwrap().len(), 2);
    let graph = run(p, &["graph", "entry"]);
    assert!(graph["data"]["nodes"].as_array().unwrap().len() >= 3);
    let generation = run(p, &["status"])["generation_id"].clone();
    fs::write(p.join("auth.py"), "def check():\n    return True\n").unwrap();
    assert_eq!(run(p, &["status"])["index_status"], "stale");
    let changes = run(p, &["changes", "--unstaged"]);
    assert_eq!(changes["data"]["changes"][0]["path"], "auth.py");
    assert!(
        !changes["data"]["affected_symbols"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let update = run(p, &["index", ".", "--incremental", "--verify"]);
    assert_eq!(update["parsed"], 1);
    assert_eq!(update["reused"], 7);
    assert_ne!(generation, update["generation_id"]);
    assert_eq!(run(p, &["doctor"])["data"]["integrity"], "ok");
    assert_eq!(run(p, &["status"])["index_status"], "fresh");
    let out = Command::new(binary())
        .current_dir(p)
        .env("KORTEX_REGISTRY", p.join("registry.db"))
        .args(["context", "absent", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "SYMBOL_NOT_FOUND"
    );
}
#[tokio::test]
async fn real_sdk_stdio_client_calls_every_tool() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
    let d = fixture();
    let mut cmd = tokio::process::Command::new(binary());
    cmd.arg("mcp")
        .current_dir(d.path())
        .env("KORTEX_REGISTRY", d.path().join("registry.db"));
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        ().serve(TokioChildProcess::new(cmd).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 10);
    let resources = client.list_all_resources().await.unwrap();
    assert_eq!(resources.len(), 3);
    for uri in [
        "kortex://repositories",
        "kortex://schema",
        "kortex://status",
    ] {
        let resource = client
            .read_resource(rmcp::model::ReadResourceRequestParams::new(uri))
            .await
            .unwrap();
        assert!(
            !serde_json::to_value(resource).unwrap()["contents"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    for (tool, args) in [
        ("list_repos", json!({})),
        ("query", json!({"text":"check"})),
        ("context", json!({"symbol":"check"})),
        ("impact", json!({"symbol":"check"})),
        ("trace", json!({"from":"entry","to":"check"})),
        ("changes", json!({"unstaged":true})),
        ("graph", json!({"symbol":"entry"})),
        ("status", json!({})),
    ] {
        let response = client
            .call_tool(
                CallToolRequestParams::new(format!("kortex_{tool}"))
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let v = serde_json::to_value(response).unwrap();
        assert_ne!(v["isError"], true, "{tool}: {v}");
        assert_eq!(
            v["structuredContent"]["schema_version"], "1.0",
            "{tool}: {v}"
        );
        if tool == "context" {
            assert_eq!(
                v["structuredContent"]["data"]["symbol"]["file_path"],
                "auth.py"
            );
        }
    }
    let response = client
        .call_tool(
            CallToolRequestParams::new("kortex_context")
                .with_arguments(json!({"symbol":"absent"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["isError"], true);
    client.cancel().await.unwrap();
}
#[test]
fn maintenance_exports_and_owned_data_deletion() {
    let d = fixture();
    let p = d.path();
    let source = fs::read(p.join("auth.py")).unwrap();
    let initialized = run(p, &["init", "."]);
    assert!(initialized["config_path"].is_string());
    assert_eq!(run(p, &["config", "validate"])["valid"], true);
    assert!(run(p, &["config", "path"])["registry"].is_string());
    assert_eq!(run(p, &["doctor", "--repair"])["data"]["integrity"], "ok");
    let generation = run(p, &["status"])["generation_id"].clone();
    assert_eq!(
        run(p, &["index", ".", "--dry-run", "--full"])["committed"],
        false
    );
    assert_eq!(run(p, &["status"])["generation_id"], generation);
    for (format, marker) in [("dot", "digraph kortex"), ("graphml", "<graphml")] {
        let output = Command::new(binary())
            .current_dir(p)
            .env("KORTEX_REGISTRY", p.join("registry.db"))
            .args(["graph", "entry", "--format", format])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().contains(marker));
    }
    let complete = Command::new(binary())
        .args(["completion", "bash"])
        .output()
        .unwrap();
    assert!(complete.status.success());
    assert!(
        String::from_utf8(complete.stdout)
            .unwrap()
            .contains("kortex")
    );
    let help = Command::new(binary()).arg("--help").output().unwrap();
    assert!(help.status.success());
    let invalid = Command::new(binary())
        .args(["impact", "--depth", "bad", "--json"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "INVALID_USAGE"
    );
    let list = run(p, &["repos", "list"]);
    let id = list["data"]["items"][0]["repo_id"].as_str().unwrap();
    let refused = Command::new(binary())
        .current_dir(p)
        .env("KORTEX_REGISTRY", p.join("registry.db"))
        .args(["repos", "remove", id, "--delete-index", "--json"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(8));
    assert!(p.join(".kortex/index.db").exists());
    run(p, &["repos", "remove", id, "--delete-index", "--yes"]);
    assert!(!p.join(".kortex/index.db").exists());
    assert_eq!(fs::read(p.join("auth.py")).unwrap(), source);
    assert!(p.join(".kortex.toml").is_file());
    assert!(
        run(p, &["repos", "list"])["data"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn environment_config_precedence_and_policy() {
    let d = support::tempdir().unwrap();
    fs::write(
        d.path().join(".kortex.toml"),
        "[analysis]\nmax_nodes = 100\n",
    )
    .unwrap();
    let output = Command::new(binary())
        .current_dir(d.path())
        .env("KORTEX_ANALYSIS__MAX_NODES", "50")
        .args(["config", "show", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["analysis"]["max_nodes"],
        50
    );
    let output = Command::new(binary())
        .current_dir(d.path())
        .env("KORTEX_SEARCH__EMBEDDINGS", "remote")
        .args(["config", "validate", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(8));
    let output = Command::new(binary())
        .current_dir(d.path())
        .env("KORTEX_ANALYSIS__TYPO", "50")
        .args(["config", "validate", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
#[test]
fn json_errors_and_remote_refusal() {
    let d = support::tempdir().unwrap();
    let out = Command::new(binary())
        .args(["serve", "--host", "0.0.0.0", "--json"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(8));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "AUTH_REQUIRED");
}
