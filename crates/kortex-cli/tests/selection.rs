//! Provenance survives transport boundaries; registered IDs never redirect to other roots.
mod support;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ReadResourceRequestParams},
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
fn command(cwd: &Path, registry: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kortex"));
    c.current_dir(cwd).env("KORTEX_REGISTRY", registry);
    c
}
#[tokio::test]
async fn explicit_config_registered_id_has_cli_mcp_resource_parity_and_refuses_tamper() {
    let d = support::tempdir().unwrap();
    let invocation = d.path().join("invocation");
    let unrelated = d.path().join("unrelated");
    fs::create_dir_all(invocation.join("src")).unwrap();
    fs::create_dir(&unrelated).unwrap();
    fs::write(
        invocation.join("src/app.py"),
        "def selected_entry():\n    pass\n",
    )
    .unwrap();
    fs::write(
        unrelated.join("other.py"),
        "def unauthorized_entry():\n    pass\n",
    )
    .unwrap();
    let config = invocation.join("chosen.toml");
    fs::write(&config, "[repository]\nroot='src'\n").unwrap();
    let registry = d.path().join("registry.db");
    let indexed = command(&invocation, &registry)
        .args(["--config", "chosen.toml", "index", ".", "--json"])
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let indexed: Value = serde_json::from_slice(&indexed.stdout).unwrap();
    let id = indexed["repo_id"].as_str().unwrap();
    let index_other = command(&unrelated, &registry)
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(index_other.status.success());
    // No explicit config at reopen and no matching ancestor config from this cwd.
    let status = command(&unrelated, &registry)
        .args(["--repo", id, "status", "--require-fresh", "--json"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["repo_id"], id);
    assert_eq!(status["index_status"], "fresh");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_kortex"));
    child
        .current_dir(&unrelated)
        .env("KORTEX_REGISTRY", &registry)
        .args(["--repo", id, "mcp"]);
    let client = tokio::time::timeout(
        Duration::from_secs(10),
        ().serve(TokioChildProcess::new(child).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let resource = client
        .read_resource(ReadResourceRequestParams::new("kortex://status"))
        .await
        .unwrap();
    let resource = serde_json::to_value(resource).unwrap();
    let value: Value =
        serde_json::from_str(resource["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(value["repo_id"], id);
    assert_eq!(value["index_status"], "fresh");
    for explicit in [false, true] {
        let args = if explicit {
            json!({"repo":id,"require_fresh":true})
        } else {
            json!({"require_fresh":true})
        };
        let reply = client
            .call_tool(
                CallToolRequestParams::new("kortex_status")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let reply = serde_json::to_value(reply).unwrap();
        assert_ne!(reply["isError"], true, "{reply}");
        assert_eq!(reply["structuredContent"]["repo_id"], id);
        assert_eq!(reply["structuredContent"]["index_status"], "fresh");
    }
    let resource = client
        .read_resource(ReadResourceRequestParams::new("kortex://schema"))
        .await
        .unwrap();
    let resource = serde_json::to_value(resource).unwrap();
    let metadata: Value =
        serde_json::from_str(resource["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(metadata["graph"], kortex_analysis::response_schema("graph"));
    assert!(
        metadata.get("edge_kinds").is_none(),
        "no transport-maintained capability list"
    );
    let graph = client
        .call_tool(
            CallToolRequestParams::new("kortex_graph").with_arguments(
                json!({"repo":id,"symbol":"selected_entry","depth":2})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let graph = serde_json::to_value(graph).unwrap();
    assert_ne!(graph["isError"], true, "{graph}");
    jsonschema::validator_for(&metadata["graph"])
        .unwrap()
        .validate(&graph["structuredContent"])
        .unwrap();
    fs::write(&config, "[repository]\nroot='../unrelated'\n").unwrap();
    let rejected = command(&unrelated, &registry)
        .args(["--repo", id, "query", "unauthorized_entry", "--json"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(8));
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "REPO_FORBIDDEN");
    let reply = client
        .call_tool(
            CallToolRequestParams::new("kortex_query").with_arguments(
                json!({"repo":id,"text":"unauthorized_entry"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let reply = serde_json::to_value(reply).unwrap();
    assert_eq!(reply["isError"], true);
    assert_eq!(
        reply["structuredContent"]["error"]["code"],
        "REPO_FORBIDDEN"
    );
    let error = client
        .read_resource(ReadResourceRequestParams::new("kortex://status"))
        .await
        .unwrap_err();
    let rmcp::service::ServiceError::McpError(error) = error else {
        panic!("structured protocol refusal required")
    };
    assert_eq!(error.data.unwrap()["error"]["code"], "REPO_FORBIDDEN");
    let denied_delete = command(&unrelated, &registry)
        .args(["repos", "remove", id, "--delete-index", "--yes", "--json"])
        .output()
        .unwrap();
    assert_eq!(denied_delete.status.code(), Some(8));
    assert!(invocation.join("src/.kortex/index.db").is_file());
    fs::write(&config, "[repository]\nroot='src'\n").unwrap();
    client.cancel().await.unwrap();
    let deleted = command(&unrelated, &registry)
        .args(["repos", "remove", id, "--delete-index", "--yes", "--json"])
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stdout)
    );
    assert!(!invocation.join("src/.kortex/index.db").exists());
    assert!(invocation.join("src/app.py").is_file());
    assert!(unrelated.join(".kortex/index.db").is_file());
}
