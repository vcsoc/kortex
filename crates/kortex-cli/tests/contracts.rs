//! Transport contracts exercised through the real CLI and rmcp client.
mod support;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kortex")
}
fn index(root: &Path) {
    let output = Command::new(binary())
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture(budget: usize) -> tempfile::TempDir {
    let root = support::tempdir().unwrap();
    fs::write(root.path().join(".gitignore"), ".kortex/\nregistry.db*\n").unwrap();
    fs::write(
        root.path().join(".kortex.toml"),
        format!("[mcp]\nmax_response_bytes = {budget}\n"),
    )
    .unwrap();
    let source: String = (0..30)
        .map(|i| format!("def item_{i:02}():\n    pass\n"))
        .collect();
    fs::write(root.path().join("app.py"), source).unwrap();
    index(root.path());
    root
}
async fn client(root: &Path, args: &[&str]) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = tokio::process::Command::new(binary());
    command
        .args(args)
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"));
    tokio::time::timeout(
        Duration::from_secs(10),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> Value {
    let response = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    serde_json::to_value(response).unwrap()
}

#[tokio::test]
async fn stdio_alias_initializes_and_strict_errors_are_structured() {
    let root = fixture(262144);
    let client = client(root.path(), &["serve", "--mcp", "stdio", "--json"]).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 10);
    for tool in tools {
        let value = serde_json::to_value(tool).unwrap();
        assert_eq!(value["annotations"]["readOnlyHint"], true);
        assert_eq!(value["annotations"]["openWorldHint"], false);
        assert_eq!(value["inputSchema"]["additionalProperties"], false);
        assert!(value["outputSchema"].is_object());
    }
    for (name, args, code) in [
        (
            "kortex_query",
            json!({"text":"item","unknown":1}),
            "INVALID_INPUT",
        ),
        ("kortex_query", json!({"text":""}), "INVALID_INPUT"),
        (
            "kortex_query",
            json!({"text":"item","limit":"1"}),
            "INVALID_INPUT",
        ),
        (
            "kortex_query",
            json!({"text":"item","limit":501}),
            "QUERY_LIMIT",
        ),
        (
            "kortex_query",
            json!({"text":"item","cursor":"1"}),
            "INVALID_CURSOR",
        ),
        ("kortex_context", json!({}), "INVALID_INPUT"),
        (
            "kortex_impact",
            json!({"symbol":"item_00","direction":"sideways"}),
            "INVALID_INPUT",
        ),
        (
            "kortex_changes",
            json!({"base":"x".repeat(1025)}),
            "INVALID_INPUT",
        ),
        (
            "kortex_changes",
            json!({"staged":true,"unstaged":true}),
            "INVALID_DIFF",
        ),
        (
            "kortex_status",
            json!({"require_fresh":"yes"}),
            "INVALID_INPUT",
        ),
        (
            "kortex_list_repos",
            json!({"repo":"ignored"}),
            "INVALID_INPUT",
        ),
    ] {
        let response = call(&client, name, args).await;
        assert_eq!(response["isError"], true, "{name}: {response}");
        assert_eq!(
            response["structuredContent"]["error"]["code"], code,
            "{response}"
        );
        assert_eq!(response["structuredContent"]["schema_version"], "1.0");
        assert!(serde_json::to_vec(&response).unwrap().len() + 512 <= 1024);
    }
    let unknown = client
        .call_tool(CallToolRequestParams::new("kortex_shell"))
        .await;
    let rmcp::service::ServiceError::McpError(error) = unknown.unwrap_err() else {
        panic!("expected a protocol error for an unknown tool");
    };
    assert_eq!(error.data.unwrap()["error"]["code"], "UNKNOWN_TOOL");
    let resource = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "kortex://unknown",
        ))
        .await;
    let rmcp::service::ServiceError::McpError(error) = resource.unwrap_err() else {
        panic!("expected a protocol error for an unknown resource");
    };
    assert_eq!(error.data.unwrap()["error"]["code"], "UNKNOWN_RESOURCE");
    // A malformed call does not terminate the session.
    assert_ne!(
        call(&client, "kortex_status", json!({})).await["isError"],
        true
    );
    tokio::time::timeout(Duration::from_secs(5), client.cancel())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn sdk_results_and_resources_enforce_encoded_budgets() {
    let root = fixture(1024);
    let client = client(root.path(), &["mcp"]).await;
    let response = call(&client, "kortex_query", json!({"text":"item","limit":30})).await;
    assert_eq!(response["isError"], true);
    assert_eq!(
        response["structuredContent"]["error"]["code"],
        "RESPONSE_BUDGET"
    );
    assert!(serde_json::to_vec(&response).unwrap().len() + 512 <= 1024);
    let result = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "kortex://status",
        ))
        .await;
    let rmcp::service::ServiceError::McpError(error) = result.unwrap_err() else {
        panic!("status resource must obey the selected repository budget");
    };
    assert_eq!(error.data.unwrap()["error"]["code"], "RESPONSE_BUDGET");
    let schema = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "kortex://schema",
        ))
        .await
        .unwrap();
    assert!(serde_json::to_vec(&schema).unwrap().len() + 512 <= 131072);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn cursors_pin_generation_query_and_repository() {
    let root = fixture(262144);
    let client = client(root.path(), &["mcp"]).await;
    let first = call(&client, "kortex_query", json!({"text":"item","limit":1})).await;
    let cursor = first["structuredContent"]["page"]["next_cursor"]
        .as_str()
        .unwrap();
    assert!(cursor.starts_with("kx1."));
    let next = call(
        &client,
        "kortex_query",
        json!({"text":"item","limit":1,"cursor":cursor}),
    )
    .await;
    assert_ne!(next["isError"], true, "{next}");
    assert_ne!(
        first["structuredContent"]["data"]["items"],
        next["structuredContent"]["data"]["items"]
    );
    let changed_query = call(
        &client,
        "kortex_query",
        json!({"text":"item_00","cursor":cursor}),
    )
    .await;
    assert_eq!(
        changed_query["structuredContent"]["error"]["code"],
        "INVALID_CURSOR"
    );
    fs::write(root.path().join("new.py"), "def item_new():\n    pass\n").unwrap();
    index(root.path());
    let changed_generation = call(
        &client,
        "kortex_query",
        json!({"text":"item","cursor":cursor}),
    )
    .await;
    assert_eq!(
        changed_generation["structuredContent"]["error"]["code"],
        "INVALID_CURSOR"
    );
    let other = fixture(262144);
    let changed_repo = call(
        &client,
        "kortex_query",
        json!({"text":"item","cursor":cursor,"repo":other.path().to_str().unwrap()}),
    )
    .await;
    assert_eq!(
        changed_repo["structuredContent"]["error"]["code"],
        "INVALID_CURSOR"
    );
    client.cancel().await.unwrap();
}

#[test]
fn alias_rejects_network_flags_and_protocol_parse_errors_stay_off_stdout() {
    for args in [
        vec!["serve", "--mcp", "stdio", "--host", "0.0.0.0", "--json"],
        vec!["serve", "--mcp", "stdio", "--port", "7337", "--json"],
        vec!["serve", "--mcp", "stdio", "--web-assets", "dist", "--json"],
        vec!["serve", "--mcp", "http", "--json"],
        vec!["mcp", "--transport", "http", "--json"],
        vec!["--json", "mcp", "--unknown"],
        vec!["--json", "--unknown", "mcp"],
    ] {
        let output = Command::new(binary()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {:?}", output.stdout);
        assert!(!output.stderr.is_empty());
    }
    let output = Command::new(binary())
        .args(["serve", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(8));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "AUTH_REQUIRED"
    );
}

#[tokio::test]
async fn alias_stdout_is_only_protocol_and_eof_shutdown_is_clean() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = fixture(262144);
    let mut child = tokio::process::Command::new(binary())
        .args(["serve", "--mcp", "stdio", "--json"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":rmcp::model::ProtocolVersion::LATEST,
        "capabilities":{},"clientInfo":{"name":"kortex-contract-test","version":"1"}
    }});
    stdin
        .write_all(format!("{initialize}\n").as_bytes())
        .await
        .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["result"]["capabilities"]["tools"].is_object());
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 10);
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    while let Some(line) = stdout.next_line().await.unwrap() {
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
    }
}

#[tokio::test]
async fn encoded_rpc_id_bound_makes_minimum_wire_budget_real_and_closes_hostile_ids() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = fixture(1024);
    let mut child = tokio::process::Command::new(binary())
        .args(["serve", "--mcp", "stdio"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":rmcp::model::ProtocolVersion::LATEST,"capabilities":{},"clientInfo":{"name":"bounded-wire-test","version":"1"}}});
    input
        .write_all(format!("{initialize}\n").as_bytes())
        .await
        .unwrap();
    let initialized = tokio::time::timeout(Duration::from_secs(5), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&initialized).unwrap()["id"],
        1
    );
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    let id = "\"".repeat(63);
    assert_eq!(serde_json::to_vec(&id).unwrap().len(), 128);
    let request = json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"kortex_context","arguments":{"symbol":"item_00"}}});
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        line.len() < 1024,
        "whole wire frame: {} bytes",
        line.len() + 1
    );
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "RESPONSE_BUDGET"
    );
    let hostile = json!({"jsonrpc":"2.0","id":"x".repeat(20_000),"method":"tools/list"});
    input
        .write_all(format!("{hostile}\n").as_bytes())
        .await
        .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), output.next_line())
        .await
        .unwrap()
        .unwrap();
    assert!(
        line.is_none(),
        "oversized request ID must close transport without echoing an unbounded ID"
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn sdk_oversized_input_closes_transport_without_synthetic_success() {
    let root = fixture(262144);
    let client = client(root.path(), &["mcp"]).await;
    let request = CallToolRequestParams::new("kortex_query").with_arguments(
        json!({"text":"x".repeat(65537)})
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = tokio::time::timeout(Duration::from_secs(5), client.call_tool(request))
        .await
        .unwrap();
    assert!(
        result.is_err(),
        "oversized protocol frames must not produce a tool success"
    );
    let _ = client.cancel().await;
}
