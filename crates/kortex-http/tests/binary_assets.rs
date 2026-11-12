//! Binary assets remain opaque immutable bytes, not UTF-8 conversions or codec execution.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kortex_http::{Config, app};
use std::{fs, path::Path};
use tower::ServiceExt;

fn config(root: &Path) -> Config {
    let mut c = Config::loopback(
        "test-only-bearer-token-32-bytes-long".into(),
        ["a".repeat(64)],
    );
    c.web_assets = Some(root.to_owned());
    c
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("index.html"),
        "<!doctype html><title>Private</title>",
    )
    .unwrap();
    d
}

#[tokio::test]
async fn binary_allowlist_preserves_all_bytes_head_lengths_and_startup_snapshot() {
    let d = fixture();
    // Deliberately includes NUL and invalid UTF-8. The asset server is not an image/font
    // decoder: even malformed owner-supplied codecs must stay byte-exact and nosniff.
    let bytes = [0x89, b'P', b'N', b'G', 0, 0xff, 0x80, b'\n'];
    let types = [
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("ico", "image/vnd.microsoft.icon"),
        ("woff", "font/woff"),
        ("woff2", "font/woff2"),
        ("ttf", "font/ttf"),
        ("otf", "font/otf"),
    ];
    for (extension, _) in types {
        fs::write(d.path().join(format!("asset.{extension}")), bytes).unwrap();
    }
    let router = app(config(d.path())).unwrap();
    for (extension, mime) in types {
        // Replacement cannot mutate the loaded response or its length.
        fs::write(d.path().join(format!("asset.{extension}")), b"replaced").unwrap();
        for method in ["GET", "HEAD"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(format!("/asset.{extension}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], mime);
            assert_eq!(
                response.headers()["content-length"],
                bytes.len().to_string()
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let body = to_bytes(response.into_body(), 1024).await.unwrap();
            assert_eq!(
                body.as_ref(),
                if method == "HEAD" {
                    &[][..]
                } else {
                    &bytes[..]
                }
            );
        }
    }
    for path in [
        "/%2e%2e/asset.png",
        "/../asset.png",
        "/asset.png/extra",
        "/asset.exe",
        "/asset.WOFF2",
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[test]
fn binary_assets_keep_file_total_and_confinement_caps_and_text_corruption_refusal() {
    let oversized = fixture();
    fs::write(
        oversized.path().join("asset.woff2"),
        vec![0xff; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    assert!(app(config(oversized.path())).is_err());
    let total = fixture();
    for i in 0..8 {
        fs::write(
            total.path().join(format!("{i}.png")),
            vec![0x80; 2 * 1024 * 1024],
        )
        .unwrap();
    }
    assert!(app(config(total.path())).is_err()); // HTML plus eight files exceeds 16 MiB.
    for corrupt in [&[0xff, 0x80][..], &[b'a', 0, b'b'][..]] {
        let text = fixture();
        fs::write(text.path().join("asset.js"), corrupt).unwrap();
        assert!(app(config(text.path())).is_err());
    }
    let nested = fixture();
    let deep = nested.path().join("a/b/c/d/e/f/g/h/i");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("asset.png"), [0, 0xff]).unwrap();
    assert!(app(config(nested.path())).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let links = fixture();
        let outside = fixture();
        fs::write(outside.path().join("secret.woff2"), [0, 0xff]).unwrap();
        symlink(
            outside.path().join("secret.woff2"),
            links.path().join("asset.woff2"),
        )
        .unwrap();
        assert!(app(config(links.path())).is_err());
        let directory = fixture();
        symlink(outside.path(), directory.path().join("fonts")).unwrap();
        assert!(app(config(directory.path())).is_err());
    }
}
