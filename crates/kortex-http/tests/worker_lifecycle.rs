//! Real SQLite contention, actual application queries and TCP disconnects; no successful-work mock.
mod support;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kortex_http::{Config, app_with_shutdown};
use serde_json::Value;
use std::{fs, future::IntoFuture, process::Command, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const TOKEN: &str = "worker-lifecycle-test-only-token-32-bytes";
fn request(path: &str, timeout: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"));
    if let Some(timeout) = timeout {
        request = request.header("x-request-timeout-ms", timeout);
    }
    request.body(Body::empty()).unwrap()
}
async fn status(router: &axum::Router) -> StatusCode {
    router
        .clone()
        .oneshot(request("/api/v1/healthz", None))
        .await
        .unwrap()
        .status()
}
async fn wait_status(router: &axum::Router, expected: StatusCode) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if status(router).await == expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 262144).await.unwrap()).unwrap()
}
fn lock_registry() -> rusqlite::Connection {
    let connection =
        rusqlite::Connection::open(std::env::var_os("KORTEX_REGISTRY").unwrap()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    connection
}
async fn tcp(address: std::net::SocketAddr, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    response
}
#[test]
fn real_workers_keep_admission_until_completion_on_timeout_disconnect_and_shutdown() {
    if std::env::var_os("KORTEX_WORKER_LIFECYCLE_CHILD").is_none() {
        let directory = support::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "real_workers_keep_admission_until_completion_on_timeout_disconnect_and_shutdown",
                "--nocapture",
            ])
            .env("KORTEX_WORKER_LIFECYCLE_CHILD", "1")
            .env("KORTEX_REGISTRY", directory.path().join("registry.db"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = support::tempdir().unwrap();
    fs::write(
        root.path().join("app.py"),
        "def real_entry():\n    return 7\n",
    )
    .unwrap();
    let report = kortex_indexer::index_at_root(
        root.path(),
        &kortex_config::Config::default(),
        &Default::default(),
    )
    .unwrap();
    kortex_storage::registry::Registry::open()
        .unwrap()
        .register(&report.snapshot)
        .unwrap();
    let query = format!(
        "/api/v1/repos/{}/query?text=real_entry",
        report.snapshot.repo_id
    );
    let mut config = Config::loopback(TOKEN.into(), [report.snapshot.repo_id.clone()]);
    config.concurrency = 1;
    config.deadline = Duration::from_secs(3);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (router, _) = app_with_shutdown(config.clone()).unwrap();
        for (path, header) in [(query.clone(), Some("100")), (format!("{query}&timeout_ms=100"), None), (format!("{query}&timeout_ms=5000"), Some("100"))] {
            let lock = lock_registry();
            let response = tokio::time::timeout(Duration::from_secs(1), router.clone().oneshot(request(&path, header))).await.unwrap().unwrap();
            let response_status = response.status();
            let response_value = value(response).await;
            assert_eq!(response_status, StatusCode::GATEWAY_TIMEOUT, "{response_value}");
            assert_eq!(response_value["error"]["code"], "TIMEOUT");
            // A timeout cancels cooperatively, NOT by claiming to interrupt SQLite's current call.
            assert_eq!(status(&router).await, StatusCode::TOO_MANY_REQUESTS);
            lock.execute_batch("ROLLBACK").unwrap();
            wait_status(&router, StatusCode::OK).await;
        }
        let response = router.clone().oneshot(request(&query, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result = value(response).await;
        assert_eq!(result["generation_id"], report.snapshot.generation_id);
        assert_eq!(result["data"]["items"][0]["symbol"]["name"], "real_entry");

        // A real TCP close must drop the request guard, not merely abort a client-side future.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(axum::serve(listener, router.clone()).with_graceful_shutdown(async { let _ = stopped.await; }).into_future());
        let lock = lock_registry();
        let mut disconnected = tokio::net::TcpStream::connect(address).await.unwrap();
        disconnected.write_all(format!("GET {query} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n").as_bytes()).await.unwrap();
        // Wait on the actual socket before probing admission, so a competing health
        // probe cannot take the only permit before this request is admitted.
        assert!(tokio::time::timeout(Duration::from_millis(100), disconnected.read_u8()).await.is_err());
        assert_eq!(status(&router).await, StatusCode::TOO_MANY_REQUESTS);
        drop(disconnected);
        assert!(tcp(address, "/api/v1/healthz").await.starts_with("HTTP/1.1 429"));
        lock.execute_batch("ROLLBACK").unwrap();
        wait_status(&router, StatusCode::OK).await;
        assert!(tcp(address, &query).await.starts_with("HTTP/1.1 200"));
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap().unwrap();

        let (router, shutdown) = app_with_shutdown(config).unwrap();
        let lock = lock_registry();
        let mut pending = tokio::spawn(router.clone().oneshot(request(&query, None)));
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut pending).await.is_err());
        assert_eq!(status(&router).await, StatusCode::TOO_MANY_REQUESTS);
        shutdown.cancel();
        let response = tokio::time::timeout(Duration::from_secs(1), pending).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(value(response).await["error"]["code"], "CANCELLED");
        assert_eq!(status(&router).await, StatusCode::TOO_MANY_REQUESTS);
        lock.execute_batch("ROLLBACK").unwrap();
        wait_status(&router, StatusCode::GATEWAY_TIMEOUT).await;
        // Shutdown is permanent: a later request cannot start new application work.
        assert_eq!(value(router.oneshot(request(&query, None)).await.unwrap()).await["error"]["code"], "CANCELLED");
    });
}
