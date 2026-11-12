//! Real SDK cancellation/saturation and EOF while native SQLite reads are contended.
mod support;
use rmcp::{ServiceExt, model::*, service::PeerRequestOptions, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
fn fixture() -> tempfile::TempDir {
    let root = support::tempdir().unwrap();
    fs::write(
        root.path().join("app.py"),
        "def bounded_entry():\n    return 3\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kortex"))
        .args(["index", ".", "--json"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    root
}
fn lock(root: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(root.join("registry.db")).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    connection
}
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    operation: &str,
    args: Value,
) -> Value {
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client.call_tool(
            CallToolRequestParams::new(operation.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    serde_json::to_value(result).unwrap()
}
#[tokio::test]
async fn sdk_cancellation_deadline_and_real_worker_saturation_recover() {
    let root = fixture();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kortex"));
    command
        .args(["mcp"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"));
    let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    let held = lock(root.path());
    let timed = call(
        &client,
        "kortex_query",
        json!({"text":"bounded_entry","timeout_ms":100}),
    )
    .await;
    assert_eq!(timed["isError"], true);
    assert_eq!(timed["structuredContent"]["error"]["exit_code"], 9);
    assert!(matches!(
        timed["structuredContent"]["error"]["code"].as_str(),
        Some("TIMEOUT" | "QUERY_TIMEOUT")
    ));
    held.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if call(&client, "kortex_query", json!({"text":"bounded_entry"})).await["isError"]
                != true
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let held = lock(root.path());
    let mut requests = vec![];
    for _ in 0..5 {
        let request: ClientRequest =
            CallToolRequest::new(CallToolRequestParams::new("kortex_status")).into();
        requests.push(
            client
                .send_request_with_option(request, PeerRequestOptions::no_options())
                .await
                .unwrap(),
        );
    }
    // Concurrent SDK handler admission is not FIFO: any one of five can be refused.
    let [a, b, c, d, e] = requests.as_mut_slice() else {
        unreachable!()
    };
    let busy = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            result = &mut a.rx => result,
            result = &mut b.rx => result,
            result = &mut c.rx => result,
            result = &mut d.rx => result,
            result = &mut e.rx => result,
        }
        .unwrap()
        .unwrap()
    })
    .await
    .unwrap();
    let busy = serde_json::to_value(busy).unwrap();
    assert_eq!(busy["structuredContent"]["error"]["code"], "SERVER_BUSY");
    // Actual protocol cancellation notifications, not just dropped client futures.
    for request in requests {
        request.cancel(Some("unused result".into())).await.unwrap();
    }
    // A queued worker may observe cancellation before entering SQLite and release
    // its permit immediately; do not assume all four had started native calls.
    // Protocol I/O must still respond while any already-running calls remain blocked.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), client.list_all_tools())
            .await
            .unwrap()
            .unwrap()
            .len(),
        10
    );
    held.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let result = call(&client, "kortex_query", json!({"text":"bounded_entry"})).await;
            if result["structuredContent"]["error"]["code"] == "SERVER_BUSY" {
                tokio::task::yield_now().await;
                continue;
            }
            assert_ne!(result["isError"], true, "{result}");
            assert_eq!(
                result["structuredContent"]["data"]["items"][0]["symbol"]["name"],
                "bounded_entry"
            );
            break;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), client.cancel())
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn sdk_actual_indexed_work_honors_deadline_and_recovers_with_real_evidence() {
    let root = support::tempdir().unwrap();
    for file in 0..80 {
        let source = (0..40)
            .map(|item| format!("def bounded_{file:02}_{item:02}():\n    return {item}\n\n"))
            .collect::<String>();
        fs::write(root.path().join(format!("module_{file:02}.py")), source).unwrap();
    }
    let indexed = Command::new(env!("CARGO_BIN_EXE_kortex"))
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .args(["index", ".", "--json"])
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stdout)
    );
    let indexed: Value = serde_json::from_slice(&indexed.stdout).unwrap();
    assert_eq!(indexed["files"], 80);
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kortex"));
    command
        .args(["mcp"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"));
    let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    let short = call(
        &client,
        "kortex_query",
        json!({"text":"bounded_","timeout_ms":1}),
    )
    .await;
    assert_eq!(short["isError"], true);
    assert_eq!(short["structuredContent"]["error"]["code"], "TIMEOUT");
    assert_eq!(short["structuredContent"]["error"]["exit_code"], 9);
    let recovered = call(
        &client,
        "kortex_query",
        json!({"text":"bounded_79_39","limit":1}),
    )
    .await;
    assert_ne!(recovered["isError"], true, "{recovered}");
    let result = &recovered["structuredContent"];
    assert_eq!(result["repo_id"], indexed["repo_id"]);
    assert_eq!(result["generation_id"], indexed["generation_id"]);
    assert_eq!(
        result["data"]["items"][0]["symbol"]["name"],
        "bounded_79_39"
    );
    assert_eq!(
        result["data"]["items"][0]["symbol"]["file_path"],
        "module_79.py"
    );
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn eof_cancels_active_workers_but_does_not_claim_to_interrupt_one_native_call() {
    let root = fixture();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_kortex"))
        .args(["mcp", "--json"])
        .current_dir(root.path())
        .env("KORTEX_REGISTRY", root.path().join("registry.db"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let initialize = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":ProtocolVersion::LATEST,"capabilities":{},"clientInfo":{"name":"kortex-active-eof","version":"1"}}});
    input
        .write_all(format!("{initialize}\n").as_bytes())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&response).unwrap()["id"], 0);
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    let held = lock(root.path());
    for id in 1..=5 {
        let request = json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"kortex_status","arguments":{}}});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    let response = tokio::time::timeout(Duration::from_secs(2), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["jsonrpc"], "2.0");
    assert!((1..=5).contains(&response["id"].as_u64().unwrap()));
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "SERVER_BUSY"
    );
    drop(input);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), child.wait())
            .await
            .is_err(),
        "runtime retains actual native workers; EOF is not forced termination"
    );
    held.execute_batch("ROLLBACK").unwrap();
    let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    while let Some(line) = output.next_line().await.unwrap() {
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value["jsonrpc"], "2.0",
            "stdout remains protocol-pure on active shutdown"
        );
        if let Some(result) = value.get("result") {
            assert_eq!(
                result["isError"], true,
                "cancelled work cannot become fabricated success"
            );
            assert_eq!(result["structuredContent"]["error"]["exit_code"], 9);
        }
    }
}
