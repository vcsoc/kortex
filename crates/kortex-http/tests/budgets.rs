//! Deterministic cancellation/saturation tests use an unfinished request body, not sleeps.
use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode},
};
use kortex_http::{Config, app};
use std::{sync::Arc, task::Poll, time::Duration};
use tokio::sync::Notify;
use tower::ServiceExt;
const TOKEN: &str = "test-only-bearer-token-32-bytes-long";
fn request(body: Body, deadline: Option<&str>) -> Request<Body> {
    let mut r = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/repos/{}/query", "a".repeat(64)))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json");
    if let Some(deadline) = deadline {
        r = r.header("x-request-timeout-ms", deadline);
    }
    r.body(body).unwrap()
}
fn pending_body(notify: Arc<Notify>) -> Body {
    Body::from_stream(futures_util::stream::poll_fn(move |_| {
        notify.notify_one();
        Poll::<Option<Result<Bytes, std::convert::Infallible>>>::Pending
    }))
}
#[tokio::test]
async fn saturation_disconnect_and_deadline_are_bounded() {
    let mut config = Config::loopback(TOKEN.into(), ["a".repeat(64)]);
    config.concurrency = 1;
    config.deadline = Duration::from_secs(2);
    let router = app(config).unwrap();
    let notify = Arc::new(Notify::new());
    let worker = tokio::spawn(
        router
            .clone()
            .oneshot(request(pending_body(notify.clone()), None)),
    );
    tokio::time::timeout(Duration::from_secs(1), notify.notified())
        .await
        .unwrap();
    let busy = router
        .clone()
        .oneshot(request(Body::from("{}"), None))
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let healthy = Request::builder()
        .uri("/api/v1/healthz")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(healthy).await.unwrap().status(),
        StatusCode::OK
    );
    let timeout = router
        .clone()
        .oneshot(request(pending_body(Arc::new(Notify::new())), Some("5")))
        .await
        .unwrap();
    assert_eq!(timeout.status(), StatusCode::GATEWAY_TIMEOUT);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(timeout.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(value["error"]["code"], "TIMEOUT");
    assert_eq!(value["error"]["exit_code"], 9);
    let bad = router
        .oneshot(request(Body::empty(), Some("0")))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn malformed_json_and_hostile_auth_never_dispatch() {
    let config = Config::loopback(TOKEN.into(), ["a".repeat(64)]);
    let router = app(config).unwrap();
    for body in [
        "{",
        &"[".repeat(512),
        "null",
        "true",
        "{\"repo\":\"/etc/passwd\"}",
    ] {
        let response = router
            .clone()
            .oneshot(request(Body::from(body.to_owned()), None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    for auth in [
        "",
        "Basic test",
        "Bearer wrong",
        "bearer test-only-bearer-token-32-bytes-long",
    ] {
        let r = Request::builder()
            .uri("/api/v1/healthz")
            .header("authorization", auth)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.clone().oneshot(r).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let duplicate = Request::builder()
        .uri("/api/v1/healthz")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(duplicate).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
