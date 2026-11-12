//! P1 executable CLI and actual MCP SDK contracts, using real indexed source.
mod support;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
const TOKEN: &str = "test-only-bearer-token-32-bytes-long";
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kortex")
}
fn command(root: &Path) -> Command {
    let mut c = Command::new(binary());
    c.current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"));
    c
}
fn fixture() -> tempfile::TempDir {
    let d = support::tempdir().unwrap();
    fs::write(d.path().join(".gitignore"), ".kortex/\nregistry.db*\n").unwrap();
    fs::write(d.path().join("app.py"),"from fastapi import FastAPI\napp = FastAPI()\n\ndef leaf():\n    return True\n\n@app.get('/items')\ndef entry():\n    return leaf()\n\ndef uncertain():\n    return mystery()\n").unwrap();
    let out = command(d.path())
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    d
}
fn rules(to: &str) -> Value {
    json!({"rules_version":"1.0","rules":[{"id":"layering","kind":"forbid_dependency","from":{"path":"app.py","kinds":["function"]},"to":{"path":to,"kinds":["function"]},"edge_kinds":["CALLS"],"max_depth":1}]})
}
#[tokio::test]
async fn routes_and_inline_rules_have_cli_and_real_sdk_parity() {
    let d = fixture();
    let root = d.path();
    let out = command(root)
        .args(["routes", "--include-processes", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let cli: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        cli["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == "/items")
    );
    for (to, exit) in [("app.py", 10), ("never.py", 7)] {
        let out = command(root)
            .args(["check", "--rules-json", &rules(to).to_string(), "--json"])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let report: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(report["data"]["exit_code"], exit);
        assert!(report["data"]["violations"].as_array().is_some());
    }
    let mut process = tokio::process::Command::new(binary());
    process
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .args(["serve", "--mcp", "stdio"]);
    let client = ().serve(TokioChildProcess::new(process).unwrap()).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "kortex_routes"));
    assert!(tools.iter().any(|t| t.name == "kortex_check"));
    for (name, args) in [
        ("kortex_list_repos", json!({"limit":1})),
        ("kortex_routes", json!({"include_processes":true})),
        ("kortex_check", rules("app.py")),
    ] {
        let reply = client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let reply = serde_json::to_value(reply).unwrap();
        assert_ne!(reply["isError"], true, "{reply}");
        let result = &reply["structuredContent"];
        let metadata =
            serde_json::to_value(tools.iter().find(|tool| tool.name == name).unwrap()).unwrap();
        jsonschema::validator_for(&metadata["outputSchema"])
            .unwrap()
            .validate(result)
            .unwrap();
        if name == "kortex_check" {
            assert_eq!(result["data"]["exit_code"], 10);
            assert!(!result["data"]["violations"].as_array().unwrap().is_empty());
        } else if name == "kortex_routes" {
            assert_eq!(result["data"]["items"], cli["data"]["items"]);
        } else {
            assert_eq!(result["data"]["items"].as_array().unwrap().len(), 1);
        }
    }
    let invalid = client
        .call_tool(
            CallToolRequestParams::new("kortex_check").with_arguments(
                json!({"rules_file":"/etc/passwd"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(invalid).unwrap()["isError"], true);
    client.cancel().await.unwrap();
}
#[test]
fn http_startup_requires_credentials_and_remote_refuses_without_them() {
    for args in [
        vec!["serve", "--host", "0.0.0.0", "--json"],
        vec![
            "serve",
            "--auth-token-env",
            "KORTEX_TEST_MISSING_TOKEN",
            "--json",
        ],
    ] {
        let out = Command::new(binary())
            .args(args)
            .env_remove("KORTEX_TEST_MISSING_TOKEN")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(8));
        assert_eq!(
            serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
            "AUTH_REQUIRED"
        );
    }
}
#[test]
fn private_assets_refuse_nonloopback_before_startup() {
    let out = Command::new(binary())
        .args([
            "serve",
            "--host",
            "0.0.0.0",
            "--web-assets",
            "owner-supplied-dist",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(8));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "ASSET_POLICY"
    );
}
#[cfg(not(feature = "http"))]
#[test]
fn default_cli_has_no_http_runtime() {
    let out = Command::new(binary())
        .args([
            "serve",
            "--auth-token-env",
            "KORTEX_TEST_TOKEN",
            "--allow-repo",
            &"a".repeat(64),
            "--json",
        ])
        .env("KORTEX_TEST_TOKEN", TOKEN)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "FEATURE_DISABLED"
    );
}
#[cfg(feature = "http")]
#[test]
fn opt_in_cli_serves_real_authorized_loopback_api() {
    use std::io::{BufRead, Read, Write};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let d = fixture();
    let root = d.path();
    let out = command(root).args(["status", "--json"]).output().unwrap();
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = status["repo_id"].as_str().unwrap();
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Child(
        command(root)
            .args([
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--auth-token-env",
                "KORTEX_TEST_TOKEN",
                "--allow-repo",
                id,
            ])
            .env("KORTEX_TEST_TOKEN", TOKEN)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut diagnostics = std::io::BufReader::new(child.0.stderr.take().unwrap());
    let mut ready = String::new();
    diagnostics.read_line(&mut ready).unwrap();
    assert!(ready.contains("listening"), "{ready}");
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    write!(stream,"GET /api/v1/repos/{id}/status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n").unwrap();
    let mut wire = String::new();
    stream.read_to_string(&mut wire).unwrap();
    assert!(wire.starts_with("HTTP/1.1 200"), "{wire}");
    assert!(wire.contains(id));
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let mut stdout = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.is_empty());
}
