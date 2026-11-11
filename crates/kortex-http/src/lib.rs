//! Optional, read-only HTTP adapter. Repository authorization precedes every index access.
mod assets;
mod contract;
#[cfg(test)]
mod error_tests;
mod listener;
pub use contract::{input_schema, openapi};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use kortex_analysis::Service;
use kortex_core::{Error, Result, hash};
use kortex_storage::registry::Registry;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_BODY: usize = 16 * 1024;
const MAX_QUERY: usize = 8192;
const MAX_OFFSET: usize = 100_000;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Secrets are deliberately not Debug/Serialize. An empty allowlist denies startup.
#[derive(Clone)]
pub struct Config {
    pub address: SocketAddr,
    pub bearer_token: String,
    pub allowed_repos: BTreeSet<String>,
    pub cors_origins: BTreeSet<String>,
    pub max_response_bytes: usize,
    pub concurrency: usize,
    pub deadline: Duration,
    /// Explicit owner-supplied prebuilt assets. Never populated from repository configuration.
    pub web_assets: Option<std::path::PathBuf>,
}
impl Config {
    pub fn loopback(token: String, repos: impl IntoIterator<Item = String>) -> Self {
        Self {
            address: SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 7337),
            bearer_token: token,
            allowed_repos: repos.into_iter().collect(),
            cors_origins: BTreeSet::new(),
            max_response_bytes: 262144,
            concurrency: 4,
            deadline: Duration::from_secs(10),
            web_assets: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.web_assets.is_some() && !self.address.ip().is_loopback() {
            return Err(Error::new(
                "ASSET_POLICY",
                "private web assets may only be served on loopback",
                8,
            ));
        }
        if self.bearer_token.len() < 32
            || self.bearer_token.len() > 4096
            || !self.bearer_token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error::new(
                "AUTH_REQUIRED",
                "HTTP requires an explicit bearer token of 32..4096 printable ASCII bytes",
                8,
            ));
        }
        if self.allowed_repos.is_empty()
            || self.allowed_repos.len() > 64
            || self.allowed_repos.iter().any(|id| !valid_id(id))
        {
            return Err(Error::new(
                "REPO_POLICY",
                "explicitly allow 1..64 registered repository IDs (64 lowercase hex digits)",
                8,
            ));
        }
        if self.address.port() == 0 {
            return Err(Error::new(
                "INVALID_CONFIG",
                "HTTP port must be nonzero so the configured same-origin authority is stable",
                2,
            ));
        }
        if !(1024..=1_048_576).contains(&self.max_response_bytes)
            || !(1..=32).contains(&self.concurrency)
            || self.deadline.is_zero()
            || self.deadline > Duration::from_secs(30)
        {
            return Err(Error::new(
                "INVALID_CONFIG",
                "HTTP budgets exceed allowed bounds",
                2,
            ));
        }
        if self.cors_origins.len() > 64 {
            return Err(Error::new(
                "INVALID_CONFIG",
                "at most 64 exact CORS origins are permitted",
                2,
            ));
        }
        for origin in &self.cors_origins {
            let uri: axum::http::Uri = origin
                .parse()
                .map_err(|_| Error::new("INVALID_CONFIG", "invalid CORS origin", 2))?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri
                    .authority()
                    .is_some_and(|a| a.as_str().contains(['@', '*']))
                || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
                || origin.ends_with('/')
            {
                return Err(Error::new(
                    "INVALID_CONFIG",
                    "CORS requires exact http(s) origins without paths, credentials, or wildcards",
                    2,
                ));
            }
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone)]
struct App {
    config: Config,
    slots: Arc<Semaphore>,
    assets: Option<Arc<assets::Assets>>,
}
#[derive(Clone)]
struct Admission(Arc<OwnedSemaphorePermit>);

/// Builds actual production handlers, also used by executable contract tests.
pub fn app(config: Config) -> Result<Router> {
    config.validate()?;
    let assets = config
        .web_assets
        .as_deref()
        .map(assets::Assets::load)
        .transpose()?
        .map(Arc::new);
    let state = App {
        slots: Arc::new(Semaphore::new(config.concurrency)),
        assets,
        config,
    };
    let mut router = Router::new();
    for (path, _) in contract::ROUTES {
        router = router.route(&format!("/api/v1{path}"), any(dispatch));
    }
    Ok(router
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state))
}

/// Explicit caller opt-in only. This function never makes outbound connections.
pub fn serve(config: Config) -> Result<()> {
    config.validate()?;
    let address = config.address;
    if !address.ip().is_loopback() {
        eprintln!(
            "Warning: binding Kortex HTTP beyond loopback; bearer authentication and repository policy are enforced."
        );
    }
    let router = app(config)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let listener = tokio::net::TcpListener::bind(address).await?;
            eprintln!("Kortex read-only HTTP listening on {address}; bearer credentials required");
            axum::serve(
                listener::BoundedListener::new(listener, 64, Duration::from_secs(30)),
                router,
            )
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
        })?;
    Ok(())
}

fn request_id() -> String {
    hash(format!(
        "{}:{}:{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}
fn error_response(e: Error, id: &str) -> Response {
    let status = match e.code {
        "AUTH_REQUIRED" => StatusCode::UNAUTHORIZED,
        "REPO_FORBIDDEN" | "CORS_REFUSED" => StatusCode::FORBIDDEN,
        "NOT_FOUND" | "REPO_NOT_FOUND" | "SYMBOL_NOT_FOUND" => StatusCode::NOT_FOUND,
        "METHOD_NOT_ALLOWED" => StatusCode::METHOD_NOT_ALLOWED,
        "REQUEST_BUDGET" => StatusCode::PAYLOAD_TOO_LARGE,
        "RESPONSE_BUDGET" => StatusCode::UNPROCESSABLE_ENTITY,
        "SERVER_BUSY" => StatusCode::TOO_MANY_REQUESTS,
        "TIMEOUT" | "CANCELLED" => StatusCode::GATEWAY_TIMEOUT,
        _ if e.exit == 9 => StatusCode::GATEWAY_TIMEOUT,
        _ if e.exit == 3 => StatusCode::NOT_FOUND,
        _ if e.exit == 2 || e.exit == 4 || e.exit == 5 => StatusCode::BAD_REQUEST,
        _ if e.exit == 8 => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    // Do not leak local filesystem locations or database diagnostics to HTTP clients.
    let message = if status.is_server_error() && e.exit != 9 {
        "local service operation failed".into()
    } else {
        e.message.chars().take(256).collect::<String>()
    };
    let mut value = json!({"schema_version":"1.0","request_id":id,"error":{"code":e.code,"message":message,"exit_code":e.exit}});
    // Errors must fit even the minimum configured body budget, including UTF-8 and JSON escaping.
    while serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() > 1024) {
        let text = value["error"]["message"].as_str().unwrap_or("");
        if text.is_empty() {
            break;
        }
        value["error"]["message"] = json!(
            text.chars()
                .take(text.chars().count() / 2)
                .collect::<String>()
        );
    }
    let mut response = (status, axum::Json(value)).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

async fn guard(State(state): State<App>, mut request: Request, next: Next) -> Response {
    let id = request_id();
    request.extensions_mut().insert(id.clone());
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let same_origin = origin
        .as_deref()
        .is_some_and(|origin| same_origin(&state, &request, origin));
    let result = async {
        if request.headers().get_all(header::ORIGIN).iter().count() > 1
            || request.headers().contains_key(header::ORIGIN)
                && origin
                    .as_ref()
                    .is_none_or(|o| !same_origin && !state.config.cors_origins.contains(o))
        {
            return error_response(
                Error::new("CORS_REFUSED", "origin is not explicitly allowed", 8),
                &id,
            );
        }
        if request.method() == Method::OPTIONS {
            // Browser preflight is not a data endpoint and cannot contain bearer credentials.
            // It discloses no repository data and never performs index/registry access.
            if origin.is_none() {
                return error_response(
                    Error::new("CORS_REFUSED", "preflight requires allowed Origin", 8),
                    &id,
                );
            }
            let method = request
                .headers()
                .get(header::ACCESS_CONTROL_REQUEST_METHOD)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let headers = request
                .headers()
                .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !matches!(method, "GET" | "POST")
                || headers
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .any(|s| {
                        !matches!(
                            s.trim().to_ascii_lowercase().as_str(),
                            "authorization" | "content-type" | "x-request-timeout-ms"
                        )
                    })
            {
                return error_response(
                    Error::new("CORS_REFUSED", "preflight method or headers refused", 8),
                    &id,
                );
            }
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST"),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("authorization, content-type, x-request-timeout-ms"),
            );
            return response;
        }
        if state.assets.is_none() || request.uri().path().starts_with("/api/") {
            let auth = request
                .headers()
                .get_all(header::AUTHORIZATION)
                .iter()
                .collect::<Vec<_>>();
            let supplied = if auth.len() == 1 {
                auth[0]
                    .to_str()
                    .ok()
                    .and_then(|a| a.strip_prefix("Bearer "))
            } else {
                None
            };
            let authorized = supplied
                .filter(|token| token.len() <= 4096)
                .is_some_and(|token| {
                    hash(token)
                        .as_bytes()
                        .ct_eq(hash(&state.config.bearer_token).as_bytes())
                        .into()
                });
            if !authorized {
                return error_response(
                    Error::new("AUTH_REQUIRED", "valid bearer authorization required", 8),
                    &id,
                );
            }
        }
        if request.uri().path().len() > MAX_QUERY
            || request.uri().query().is_some_and(|q| q.len() > MAX_QUERY)
        {
            return error_response(
                Error::new("REQUEST_BUDGET", "request URI exceeds budget", 2),
                &id,
            );
        }
        let permit = match state.slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                return error_response(
                    Error::new("SERVER_BUSY", "HTTP concurrency budget exhausted", 9),
                    &id,
                );
            }
        };
        request.extensions_mut().insert(Admission(Arc::new(permit)));
        let deadline = match request.headers().get("x-request-timeout-ms") {
            None => state.config.deadline,
            Some(v) => match v
                .to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0)
            {
                Some(n) => state.config.deadline.min(Duration::from_millis(n)),
                None => {
                    return error_response(
                        Error::new("INVALID_INPUT", "invalid request deadline", 2),
                        &id,
                    );
                }
            },
        };
        match tokio::time::timeout(deadline, next.run(request)).await {
            Ok(response) => response,
            Err(_) => error_response(Error::new("TIMEOUT", "request deadline exceeded", 9), &id),
        }
    }
    .await;
    let mut response = result;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).unwrap_or_else(|_| HeaderValue::from_static("unavailable")),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Origin"));
    response.headers_mut().insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
    if let Some(origin) = origin.filter(|o| same_origin || state.config.cors_origins.contains(o)) {
        if let Ok(value) = HeaderValue::from_str(&origin) {
            response
                .headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
        response.headers_mut().insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("x-request-id"),
        );
    }
    response
}

fn same_origin(state: &App, request: &Request, origin: &str) -> bool {
    if state.assets.is_none() || !state.config.address.ip().is_loopback() {
        return false;
    }
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("http")
        && uri.authority().map(|a| a.as_str())
            == request
                .headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
        && uri.port_u16().unwrap_or(80) == state.config.address.port()
        && uri.host().is_some_and(|host| {
            host == "localhost"
                || host.trim_matches(['[', ']']).parse::<IpAddr>().ok()
                    == Some(state.config.address.ip())
        })
        && uri.path_and_query().is_none_or(|p| p.as_str() == "/")
}
async fn fallback(State(state): State<App>, request: Request) -> Response {
    if state.assets.is_some() && !request.uri().path().starts_with("/api/") {
        assets::dispatch(State(state), request).await
    } else {
        dispatch(State(state), request).await
    }
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn dispatch(State(state): State<App>, mut request: Request) -> Response {
    let id = request
        .extensions()
        .get::<String>()
        .cloned()
        .unwrap_or_else(request_id);
    let result = dispatch_inner(&state, &mut request, &id).await;
    match result {
        Ok(mut value) => {
            // OpenAPI's top-level shape is the standard document, not a Service envelope.
            if value.get("openapi").is_none() {
                value["request_id"] = json!(id);
            }
            match serde_json::to_vec(&value) {
                Ok(bytes) if bytes.len() <= state.config.max_response_bytes => {
                    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
                }
                _ => error_response(
                    Error::new(
                        "RESPONSE_BUDGET",
                        "response exceeds byte budget; reduce limit/depth/max_nodes",
                        2,
                    ),
                    &id,
                ),
            }
        }
        Err(e) => error_response(e, &id),
    }
}

async fn dispatch_inner(state: &App, request: &mut Request, id: &str) -> Result<Value> {
    let path = request
        .uri()
        .path()
        .strip_prefix("/api/v1")
        .ok_or_else(|| Error::new("NOT_FOUND", "unknown endpoint", 3))?;
    let segments: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (operation, repo, symbol) = match segments.as_slice() {
        ["healthz"] => ("healthz", None, None),
        ["openapi.json"] => ("openapi", None, None),
        ["repos"] => ("repos", None, None),
        ["repos", repo, "symbols", symbol] if valid_id(repo) && valid_id(symbol) => (
            "context",
            Some((*repo).to_owned()),
            Some((*symbol).to_owned()),
        ),
        ["repos", repo, operation]
            if valid_id(repo)
                && contract::ROUTES.iter().any(|(_, op)| op == operation)
                && !matches!(*operation, "healthz" | "openapi" | "repos" | "context") =>
        {
            (*operation, Some((*repo).to_owned()), None)
        }
        _ => {
            return Err(Error::new(
                "NOT_FOUND",
                "unknown endpoint or invalid identifier",
                3,
            ));
        }
    };
    let operation = contract::ROUTES
        .iter()
        .find(|(_, op)| *op == operation)
        .map(|(_, op)| *op)
        .ok_or_else(|| Error::new("NOT_FOUND", "unknown endpoint", 3))?;
    if repo
        .as_ref()
        .is_some_and(|r| !state.config.allowed_repos.contains(r))
    {
        return Err(Error::new(
            "REPO_FORBIDDEN",
            "repository is not authorized",
            8,
        ));
    }
    let post_allowed = !matches!(
        operation,
        "healthz" | "openapi" | "repos" | "status" | "context"
    );
    if !(request.method() == Method::GET && operation != "check"
        || post_allowed && request.method() == Method::POST)
    {
        return Err(Error::new("METHOD_NOT_ALLOWED", "unsupported method", 2));
    }
    let mut args = parse_arguments(request, operation).await?;
    contract::validate(operation, &args)?;
    if args["max_nodes"].as_u64().is_some_and(|n| n > 500)
        || args["depth"].as_u64().is_some_and(|n| n > 10)
    {
        return Err(Error::new(
            "QUERY_LIMIT",
            "HTTP traversal bounds are max_nodes <=500 and depth <=10",
            2,
        ));
    }
    if matches!(operation, "query" | "routes" | "files")
        && args.get("cursor").is_some_and(|v| {
            !v.is_null()
                && v.as_str()
                    .is_none_or(|s| !s.starts_with("kx1.") || s.len() > 512)
        })
    {
        return Err(Error::new(
            "INVALID_CURSOR",
            "expected shared generation-pinned cursor",
            2,
        ));
    }
    if let Some(symbol) = symbol {
        args["symbol"] = json!(symbol);
    }
    match operation {
        "healthz" => {
            return Ok(
                json!({"schema_version":"1.0","data":{"status":"ok"},"partial":false,"truncated":false}),
            );
        }
        "openapi" => return Ok(openapi()),
        _ => {}
    }
    let admission = request
        .extensions_mut()
        .remove::<Admission>()
        .ok_or_else(|| Error::new("INTERNAL", "missing admission", 1))?;
    let policy = state.config.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancelled.clone());
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = admission.0; // A timed-out/disconnected worker retains its slot until it actually finishes.
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "request cancelled", 9));
        }
        let registry = Registry::open()?.list()?;
        if operation == "repos" {
            return repos_page(registry, &policy, &args, &id);
        }
        let repo = repo.ok_or_else(|| Error::new("REPO_FORBIDDEN", "repository required", 8))?;
        let (_, root, _) = registry
            .iter()
            .find(|(rid, _, _)| rid == &repo)
            .ok_or_else(|| {
                Error::new(
                    "REPO_NOT_FOUND",
                    "authorized repository is not registered",
                    3,
                )
            })?;
        // Registry roots are already effective repository identities. Never let a repository
        // config redirect an authorized remote request to another filesystem root.
        let config = kortex_config::Config::load(Path::new(root), None)?;
        let service = Service::open_at_root(Path::new(root), config)?;
        if service.snapshot.repo_id != repo {
            return Err(Error::new(
                "REPO_FORBIDDEN",
                "registry/index identity mismatch",
                8,
            ));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "request cancelled", 9));
        }
        let mut result = service.execute(operation, &args)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "request cancelled", 9));
        }
        result["request_id"] = json!(id);
        Ok(result)
    })
    .await
    .map_err(|_| Error::new("INTERNAL", "HTTP worker failed", 1))?
}

async fn parse_arguments(request: &mut Request, operation: &str) -> Result<Value> {
    let mut args = json!({});
    let schema = input_schema(operation);
    if request.method() == Method::POST {
        if request.uri().query().is_some() {
            return Err(Error::new(
                "INVALID_INPUT",
                "POST query parameters are not supported",
                2,
            ));
        }
        if request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_none_or(|s| s.split(';').next() != Some("application/json"))
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "application/json is required",
                2,
            ));
        }
    } else if let Some(query) = request.uri().query() {
        let pairs: Vec<(String, String)> = serde_urlencoded::from_str(query)
            .map_err(|_| Error::new("INVALID_INPUT", "malformed query", 2))?;
        for (key, value) in pairs {
            if args.get(&key).is_some() {
                return Err(Error::new("INVALID_INPUT", "duplicate query parameter", 2));
            }
            args[&key] = match schema["properties"][&key]["type"].as_str() {
                Some("integer") => json!(value.parse::<u64>().map_err(|_| Error::new(
                    "INVALID_INPUT",
                    "invalid integer",
                    2
                ))?),
                Some("boolean") => json!(value.parse::<bool>().map_err(|_| Error::new(
                    "INVALID_INPUT",
                    "invalid boolean",
                    2
                ))?),
                _ => json!(value),
            };
        }
    }
    let body = std::mem::replace(request.body_mut(), Body::empty());
    let bytes = to_bytes(body, MAX_BODY).await.map_err(|_| {
        Error::new(
            "REQUEST_BUDGET",
            "body exceeds 16 KiB or could not be read",
            2,
        )
    })?;
    if request.method() == Method::POST {
        args = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("INVALID_JSON", "malformed JSON body", 2))?;
    } else if !bytes.is_empty() {
        return Err(Error::new("INVALID_INPUT", "GET body is not supported", 2));
    }
    Ok(args)
}
fn encode_cursor(pin: &str, offset: usize) -> String {
    format!("v1:{pin}:{offset}")
}
fn decode_cursor(cursor: &str, pin: &str) -> Result<usize> {
    let parts: Vec<_> = cursor.split(':').collect();
    if parts.len() != 3
        || parts[0] != "v1"
        || parts[1] != pin
        || parts[2].is_empty()
        || !parts[2].bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Error::new(
            "INVALID_CURSOR",
            "cursor does not match repository generation/request; restart pagination",
            2,
        ));
    }
    parts[2]
        .parse::<usize>()
        .ok()
        .filter(|n| *n <= MAX_OFFSET)
        .ok_or_else(|| Error::new("INVALID_CURSOR", "pagination offset exceeds policy", 2))
}
fn repos_page(
    registry: Vec<(String, String, String)>,
    policy: &Config,
    args: &Value,
    id: &str,
) -> Result<Value> {
    let items = registry
        .into_iter()
        .filter(|(rid, _, _)| policy.allowed_repos.contains(rid))
        .map(|(repo_id, _, generation_id)| json!({"repo_id":repo_id,"generation_id":generation_id}))
        .collect::<Vec<_>>();
    let pin = hash(serde_json::to_vec(&items)?);
    let offset = args["cursor"]
        .as_str()
        .map(|c| decode_cursor(c, &pin))
        .transpose()?
        .unwrap_or(0);
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let more = offset.saturating_add(limit) < items.len();
    let page = items
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    Ok(
        json!({"schema_version":"1.0","request_id":id,"data":{"items":page},"partial":false,"truncated":more,"page":{"next_cursor":if more {Some(encode_cursor(&pin,offset+limit))}else{None}}}),
    )
}
