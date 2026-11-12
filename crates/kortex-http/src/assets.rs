//! Immutable owner-supplied build assets. Never activated by indexed repository configuration.
use crate::{App, error_response, request_id};
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{Method, header},
    response::{IntoResponse, Response},
};
use kortex_core::{Error, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_TOTAL: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;
#[derive(Clone)]
struct Asset {
    bytes: Arc<[u8]>,
    mime: &'static str,
}
#[derive(Clone)]
pub(crate) struct Assets {
    files: BTreeMap<String, Asset>,
}
fn mime(path: &str) -> Option<&'static str> {
    match path.rsplit_once('.')?.1 {
        "html" => Some("text/html; charset=utf-8"),
        "js" | "mjs" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        "svg" => Some("image/svg+xml"),
        "txt" => Some("text/plain; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "ico" => Some("image/vnd.microsoft.icon"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "otf" => Some("font/otf"),
        _ => None,
    }
}
fn normal(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
impl Assets {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(Error::new(
                "ASSET_POLICY",
                "asset root cannot be a symlink",
                8,
            ));
        }
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err(Error::new(
                "ASSET_POLICY",
                "web assets must be an explicit prebuilt directory",
                8,
            ));
        }
        let mut files = BTreeMap::new();
        let mut queue = vec![(PathBuf::new(), 0)];
        let mut entries = 0;
        let mut total = 0;
        while let Some((relative, depth)) = queue.pop() {
            if depth > 8 {
                return Err(Error::new(
                    "ASSET_POLICY",
                    "asset directory nesting exceeds eight levels",
                    8,
                ));
            }
            for entry in std::fs::read_dir(root.join(&relative))? {
                let entry = entry?;
                entries += 1;
                if entries > MAX_ENTRIES {
                    return Err(Error::new(
                        "ASSET_POLICY",
                        "asset inventory exceeds 512 entries",
                        8,
                    ));
                }
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    return Err(Error::new(
                        "ASSET_POLICY",
                        "symlinked assets are refused",
                        8,
                    ));
                }
                let path = relative.join(entry.file_name());
                if file_type.is_dir() {
                    queue.push((path, depth + 1));
                    continue;
                }
                if !file_type.is_file() {
                    return Err(Error::new(
                        "ASSET_POLICY",
                        "assets must be regular files",
                        8,
                    ));
                }
                let public = path
                    .to_str()
                    .ok_or_else(|| Error::new("ASSET_POLICY", "asset names must be UTF-8", 8))?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                if !normal(&public) {
                    return Err(Error::new(
                        "ASSET_POLICY",
                        "asset names require normal ASCII relative segments",
                        8,
                    ));
                }
                let Some(mime) = mime(&public) else { continue }; // Never serve source maps, directory contents or arbitrary file types.
                let bytes = if mime.starts_with("text/")
                    || matches!(mime, "application/json; charset=utf-8" | "image/svg+xml")
                {
                    // Keep text validation (including NUL/invalid UTF-8 refusal) unchanged.
                    kortex_repo::read_confined(&root, &public, MAX_FILE)?.into_bytes()
                } else {
                    // Binary assets use the shared descriptor-relative reader, never an
                    // ambient std::fs::read fallback. Bytes are opaque, not decoded/executed.
                    kortex_repo::read_confined_bytes(&root, &public, MAX_FILE)?
                };
                total += bytes.len();
                if total > MAX_TOTAL {
                    return Err(Error::new(
                        "ASSET_POLICY",
                        "asset snapshot exceeds 16 MiB",
                        8,
                    ));
                }
                files.insert(
                    public,
                    Asset {
                        bytes: bytes.into(),
                        mime,
                    },
                );
            }
        }
        if !files.contains_key("index.html") {
            return Err(Error::new(
                "ASSET_POLICY",
                "prebuilt web assets require index.html",
                8,
            ));
        }
        Ok(Self { files })
    }
}
pub(crate) async fn dispatch(State(state): State<App>, request: Request) -> Response {
    let id = request
        .extensions()
        .get::<String>()
        .cloned()
        .unwrap_or_else(request_id);
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return error_response(
            Error::new(
                "METHOD_NOT_ALLOWED",
                "static assets support GET and HEAD only",
                2,
            ),
            &id,
        );
    }
    let path = request.uri().path().strip_prefix('/').unwrap_or("");
    let path = if path.is_empty() { "index.html" } else { path };
    if !normal(path) {
        return error_response(Error::new("NOT_FOUND", "asset not found", 3), &id);
    }
    let Some(asset) = state
        .assets
        .as_ref()
        .and_then(|assets| assets.files.get(path))
    else {
        return error_response(Error::new("NOT_FOUND", "asset not found", 3), &id);
    };
    // GET/HEAD bodies are never consumed into unbounded buffers or interpreted.
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .is_some_and(|value| value != "0")
        || request.headers().contains_key(header::TRANSFER_ENCODING)
    {
        return error_response(
            Error::new("INVALID_INPUT", "static request bodies are refused", 2),
            &id,
        );
    }
    let head = request.method() == Method::HEAD;
    match to_bytes(request.into_body(), crate::MAX_BODY).await {
        Ok(bytes) if bytes.is_empty() => {}
        _ => {
            return error_response(
                Error::new("INVALID_INPUT", "static request bodies are refused", 2),
                &id,
            );
        }
    }
    let body = if head {
        Body::empty()
    } else {
        Body::from(Bytes::from_owner(asset.bytes.clone()))
    };
    (
        [
            (header::CONTENT_TYPE, asset.mime.to_owned()),
            (header::CONTENT_LENGTH, asset.bytes.len().to_string()),
        ],
        body,
    )
        .into_response()
}
