//! Owner-opted-in immutable assets use the same production HTTP router as the private install.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kortex_http::{Config, app};
use std::fs;
use tower::ServiceExt;
const TOKEN: &str = "test-only-bearer-token-32-bytes-long";
fn config(root: &std::path::Path) -> Config {
    let mut c = Config::loopback(TOKEN.into(), ["a".repeat(64)]);
    c.web_assets = Some(root.to_path_buf());
    c
}
fn assets() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("assets")).unwrap();
    fs::write(d.path().join("index.html"),"<!doctype html><title>Private build</title><script type=module src='/assets/app.js'></script>").unwrap();
    fs::write(
        d.path().join("assets/app.js"),
        "document.title='Private Kortex';",
    )
    .unwrap();
    fs::write(d.path().join("assets/style.css"), "body { color: black; }").unwrap();
    fs::write(d.path().join("assets/app.js.map"), "private source map").unwrap();
    d
}
#[tokio::test]
async fn static_mode_has_correct_mime_security_headers_and_unchanged_api_auth() {
    let d = assets();
    let router = app(config(d.path())).unwrap();
    for (path, mime) in [
        ("/", "text/html"),
        ("/index.html", "text/html"),
        ("/assets/app.js", "text/javascript"),
        ("/assets/style.css", "text/css"),
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(mime)
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            response.headers()["cross-origin-resource-policy"],
            "same-origin"
        );
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("connect-src 'self'"));
        assert!(!csp.contains("unsafe-eval"));
        assert!(
            !to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let head = router
        .clone()
        .oneshot(
            Request::builder()
                .method("HEAD")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert!(
        head.headers()["content-length"]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap()
            > 0
    );
    assert!(to_bytes(head.into_body(), 1024).await.unwrap().is_empty());
    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let same = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/healthz")
                .header("host", "127.0.0.1:7337")
                .header("origin", "http://127.0.0.1:7337")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(same.status(), StatusCode::OK);
    let evil = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/healthz")
                .header("host", "127.0.0.1:7337")
                .header("origin", "https://evil.invalid")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(evil.status(), StatusCode::FORBIDDEN);
    for path in [
        "/../index.html",
        "/%2e%2e/index.html",
        "/assets%2fapp.js",
        "//index.html",
        "/assets/",
        "/assets/app.js.map",
        "/missing-route",
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let post = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    let body = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::from("unexpected body"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body.status(), StatusCode::BAD_REQUEST);
    fs::write(
        d.path().join("index.html"),
        "changed after explicit startup snapshot",
    )
    .unwrap();
    let original = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        String::from_utf8(to_bytes(original.into_body(), 1024).await.unwrap().to_vec())
            .unwrap()
            .contains("Private build")
    );
}
#[test]
fn explicit_assets_are_loopback_only_bounded_and_fail_closed() {
    let d = assets();
    let mut remote = config(d.path());
    remote.address = "0.0.0.0:7337".parse().unwrap();
    assert_eq!(remote.validate().unwrap_err().code, "ASSET_POLICY"); // Validation only; no remote listener.
    let mut zero = config(d.path());
    zero.address.set_port(0);
    assert_eq!(zero.validate().unwrap_err().code, "INVALID_CONFIG");
    let missing = tempfile::tempdir().unwrap();
    assert!(app(config(missing.path())).is_err());
    fs::write(
        d.path().join("assets/large.js"),
        vec![b'x'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    assert!(app(config(d.path())).is_err());
    let many = assets();
    for i in 0..513 {
        fs::write(many.path().join(format!("{i}.txt")), "x").unwrap();
    }
    assert!(app(config(many.path())).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let links = assets();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.js"), "secret").unwrap();
        symlink(
            outside.path().join("secret.js"),
            links.path().join("assets/linked.js"),
        )
        .unwrap();
        assert!(app(config(links.path())).is_err());
        let alias = outside.path().join("linked-root");
        symlink(links.path(), &alias).unwrap();
        assert!(app(config(&alias)).is_err());
    }
}
