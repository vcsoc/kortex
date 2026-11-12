//! A byte-limited result must remain usable through the actual MCP client.
mod support;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, process::Command, time::Duration};

#[tokio::test]
async fn byte_truncated_pages_keep_all_hits_and_opaque_cursors() {
    let root = support::tempdir().unwrap();
    fs::write(root.path().join(".gitignore"), ".kortex/\nregistry.db*\n").unwrap();
    fs::write(
        root.path().join(".kortex.toml"),
        "[mcp]\nmax_response_bytes = 8192\n",
    )
    .unwrap();
    let source: String = (0..30)
        .map(|i| format!("def item_{i:02}():\n    pass\n"))
        .collect();
    fs::write(root.path().join("app.py"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kortex"))
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kortex"));
    command
        .arg("mcp")
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"));
    let client = tokio::time::timeout(
        Duration::from_secs(10),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let mut seen = BTreeSet::new();
    let mut cursor = None;
    let mut truncated = false;
    let mut finished = false;
    for _ in 0..31 {
        let mut args = json!({"text":"item","limit":30});
        if let Some(cursor) = cursor.take() {
            args["cursor"] = cursor;
        }
        let result = client
            .call_tool(
                CallToolRequestParams::new("kortex_query")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() + 512 <= 8192);
        let result = serde_json::to_value(result).unwrap();
        assert_ne!(result["isError"], true, "{result}");
        let result = &result["structuredContent"];
        truncated |= result["truncated"] == true;
        let items = result["data"]["items"].as_array().unwrap();
        assert!(!items.is_empty());
        for item in items {
            assert!(seen.insert(item["symbol"]["symbol_id"].as_str().unwrap().to_owned()));
        }
        cursor = result["page"]["next_cursor"]
            .as_str()
            .map(|s| Value::String(s.to_owned()));
        if cursor.is_none() {
            finished = true;
            break;
        }
    }
    assert!(finished);
    assert!(truncated);
    assert_eq!(seen.len(), 30);
    client.cancel().await.unwrap();
}
