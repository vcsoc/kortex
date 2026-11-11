//! Real production-router and loopback TCP contracts. Test registry is process-isolated.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kortex_http::{Config, app};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, future::IntoFuture, path::Path, process::Command};
use tower::ServiceExt;
const TOKEN: &str = "test-only-bearer-token-32-bytes-long";

async fn call(
    router: &axum::Router,
    path: &str,
    method: &str,
    body: Option<Value>,
    auth: bool,
    origin: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if auth {
        request = request.header("authorization", format!("Bearer {TOKEN}"));
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let body = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    if !value.is_null() && value.get("openapi").is_none() {
        assert_eq!(
            value["request_id"].as_str(),
            headers.get("x-request-id").and_then(|v| v.to_str().ok())
        );
    }
    (status, headers, value)
}
fn index(path: &Path) -> kortex_core::Snapshot {
    let report = kortex_indexer::index(
        path,
        &kortex_config::Config::default(),
        &kortex_indexer::Options {
            full: true,
            verify: true,
            dry_run: false,
        },
    )
    .unwrap();
    kortex_storage::registry::Registry::open()
        .unwrap()
        .register(&report.snapshot)
        .unwrap();
    report.snapshot
}
#[test]
fn production_http_contracts() {
    if std::env::var_os("KORTEX_HTTP_TEST_CHILD").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "production_http_contracts", "--nocapture"])
            .env("KORTEX_HTTP_TEST_CHILD", "1")
            .env("KORTEX_REGISTRY", temp.path().join("registry.db"))
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
    let root = tempfile::tempdir().unwrap();
    let hidden = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        root.path().join("app.py"),
        (0..8)
            .map(|i| format!("def item_{i}():\n    pass\n\n"))
            .collect::<String>()
            + "from fastapi import FastAPI\napp = FastAPI()\n@app.get('/items')\ndef entry():\n    item_0()\n",
    )
    .unwrap();
    fs::write(root.path().join("z.py"), "def other():\n    pass\n").unwrap();
    fs::write(hidden.path().join("secret.py"), "def secret():\n    pass\n").unwrap();
    let snapshot = index(root.path());
    let hidden_snapshot = index(hidden.path());
    let mut config = Config::loopback(TOKEN.into(), [snapshot.repo_id.clone()]);
    config.cors_origins = BTreeSet::from(["http://localhost:9000".into()]);
    let router = app(config.clone()).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for path in ["/api/v1/healthz","/api/v1/openapi.json","/api/v1/repos",&format!("/api/v1/repos/{}/status",snapshot.repo_id),"/unknown"] {
            let (s,h,v)=call(&router,path,"GET",None,false,None).await;assert_eq!(s,StatusCode::UNAUTHORIZED);assert_eq!(v["error"]["code"],"AUTH_REQUIRED");assert!(h.contains_key("www-authenticate"));
        }
        let (s,_,v)=call(&router,"/api/v1/healthz","GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(v["data"]["status"],"ok");
        let (s,_,v)=call(&router,"/api/v1/repos","GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(v["data"]["items"].as_array().unwrap().len(),1);assert_eq!(v["data"]["items"][0]["repo_id"],snapshot.repo_id);assert!(!v.to_string().contains(&hidden_snapshot.repo_id));assert!(!v.to_string().contains(root.path().to_str().unwrap()));
        for op in ["status","files","query","impact","trace","changes","graph","routes","check"]{
            let (s,_,v)=call(&router,&format!("/api/v1/repos/{}/{op}",hidden_snapshot.repo_id),"GET",None,true,None).await;assert_eq!(s,StatusCode::FORBIDDEN);assert_eq!(v["error"]["code"],"REPO_FORBIDDEN");
        }
        let base=format!("/api/v1/repos/{}",snapshot.repo_id);
        let target=snapshot.symbols().find(|s|s.name=="item_0").unwrap();
        for (suffix,body) in [("files",json!({"limit":1})),("query",json!({"text":"item","limit":1})),("impact",json!({"symbol":"item_0","depth":2})),("trace",json!({"from":"entry","to":"item_0"})),("graph",json!({"symbol":"entry","depth":2})),("changes",json!({"unstaged":true})),("routes",json!({"include_processes":true}))] {
            let (s,_,v)=call(&router,&format!("{base}/{suffix}"),"POST",Some(body),true,None).await;assert_eq!(s,StatusCode::OK,"{suffix}: {v}");assert_eq!(v["generation_id"],snapshot.generation_id);assert!(v["data"].is_object());
            let schema=kortex_analysis::response_schema(suffix);
            jsonschema::validator_for(&schema).unwrap().validate(&v).unwrap();
        }
        let rules=json!({"rules_version":"1.0","rules":[{"id":"no-call","kind":"forbid_dependency","from":{"path":"app.py","kinds":["function"]},"to":{"path":"app.py","kinds":["function"]},"edge_kinds":["CALLS"]}]});
        let(s,_,v)=call(&router,&format!("{base}/check"),"POST",Some(rules),true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(v["data"]["exit_code"],10);assert!(!v["data"]["violations"].as_array().unwrap().is_empty());jsonschema::validator_for(&kortex_analysis::response_schema("check")).unwrap().validate(&v).unwrap();
        let(s,_,_)=call(&router,&format!("{base}/check"),"POST",Some(json!({"rules_file":"/etc/passwd"})),true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);
        for suffix in ["status".to_string(),format!("symbols/{}",target.symbol_id)] {let(s,_,v)=call(&router,&format!("{base}/{suffix}"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK,"{v}");}
        let(s,_,file_page)=call(&router,&format!("{base}/files?limit=1"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(file_page["data"]["items"][0]["path"],"app.py");
        let file_cursor=file_page["page"]["next_cursor"].as_str().unwrap();
        let(s,_,file_next)=call(&router,&format!("{base}/files?limit=1&cursor={file_cursor}"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(file_next["data"]["items"][0]["path"],"z.py");
        let(s,_,v)=call(&router,&format!("{base}/files?limit=1&cursor={file_cursor}&path_prefix=z"),"GET",None,true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);assert_eq!(v["error"]["code"],"INVALID_CURSOR");
        let(s,_,_)=call(&router,&format!("{base}/files"),"GET",None,false,None).await;assert_eq!(s,StatusCode::UNAUTHORIZED);
        let(s,_,first)=call(&router,&format!("{base}/query?text=item&limit=1"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK);let cursor=first["page"]["next_cursor"].as_str().unwrap();
        let(s,_,second)=call(&router,&format!("{base}/query?text=item&limit=1&cursor={cursor}"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_ne!(first["data"]["items"][0],second["data"]["items"][0]);
        for request in [json!({"text":"entry","cursor":cursor}),json!({"text":"item","cursor":"1"}),json!({"text":"item","cursor":format!("v1:{}:99999999999999999999","a".repeat(64))}),json!({"text":"item","repo":root.path()}),json!({"text":"item","limit":501}),json!({"text":"item","bogus":true}),json!({"text":"item","limit":-1})] {
            let(s,_,v)=call(&router,&format!("{base}/query"),"POST",Some(request),true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST,"{v}");
        }
        let(s,_,_)=call(&router,&format!("{base}/query?text=item&text=entry"),"GET",None,true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);
        let(s,_,v)=call(&router,&format!("{base}/query"),"POST",Some(json!({"text":"x".repeat(17000)})),true,None).await;assert_eq!(s,StatusCode::PAYLOAD_TOO_LARGE);assert_eq!(v["error"]["code"],"REQUEST_BUDGET");
        for origin in ["https://evil.invalid","null","http://localhost:9000.evil.invalid"]{let(s,h,_)=call(&router,"/api/v1/healthz","GET",None,true,Some(origin)).await;assert_eq!(s,StatusCode::FORBIDDEN);assert!(!h.contains_key("access-control-allow-origin"));}
        let(s,h,_)=call(&router,"/api/v1/healthz","GET",None,true,Some("http://localhost:9000")).await;assert_eq!(s,StatusCode::OK);assert_eq!(h["access-control-allow-origin"],"http://localhost:9000");
        let preflight=Request::builder().method("OPTIONS").uri("/api/v1/repos").header("origin","http://localhost:9000").header("access-control-request-method","GET").header("access-control-request-headers","authorization").body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(preflight).await.unwrap().status(),StatusCode::NO_CONTENT);
        let(s,_,doc)=call(&router,"/api/v1/openapi.json","GET",None,true,None).await;assert_eq!(s,StatusCode::OK);assert_eq!(doc["openapi"],"3.1.0");
        for path in ["/healthz","/repos","/repos/{id}/status","/repos/{id}/files","/repos/{id}/query","/repos/{id}/symbols/{symbol}","/repos/{id}/impact","/repos/{id}/trace","/repos/{id}/changes","/repos/{id}/graph","/repos/{id}/routes"] { assert!(doc["paths"][format!("/api/v1{path}")]["get"]["security"].is_array()); }
        assert_eq!(doc["paths"]["/api/v1/repos/{id}/query"]["post"]["requestBody"]["content"]["application/json"]["schema"],kortex_http::input_schema("query"));
        let(s,_,query)=call(&router,&format!("{base}/query?text=item&limit=1"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK);
        let bundled=json!({"$ref":doc["paths"]["/api/v1/repos/{id}/query"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],"components":doc["components"]});jsonschema::validator_for(&bundled).unwrap().validate(&query).unwrap();
        // Real TCP execution uses the same router on loopback only, never a remote listener.
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(axum::serve(listener,router.clone()).into_future());
        let wire=tokio::task::spawn_blocking(move || {use std::io::{Read,Write};let mut stream=std::net::TcpStream::connect(address).unwrap();stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();write!(stream,"GET /api/v1/healthz HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n").unwrap();let mut wire=String::new();stream.read_to_string(&mut wire).unwrap();wire}).await.unwrap();assert!(wire.starts_with("HTTP/1.1 200"));assert!(wire.contains("\"status\":\"ok\""));server.abort();
        let mut small=config.clone();small.max_response_bytes=1024;let small=app(small).unwrap();let(s,_,v)=call(&small,&format!("{base}/graph?symbol=entry&depth=2"),"GET",None,true,None).await;assert_eq!(s,StatusCode::UNPROCESSABLE_ENTITY);assert_eq!(v["error"]["code"],"RESPONSE_BUDGET");
        let mut hostile=serde_json::Map::new();hostile.insert("🧪".repeat(512),json!(true));let(s,_,v)=call(&small,&format!("{base}/query"),"POST",Some(Value::Object(hostile)),true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);assert!(serde_json::to_vec(&v).unwrap().len()<=1024);
        fs::write(root.path().join("app.py"),"def replaced():\n    pass\n").unwrap();index(root.path());
        let(s,_,v)=call(&router,&format!("{base}/query?text=item&cursor={cursor}"),"GET",None,true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);assert_eq!(v["error"]["code"],"INVALID_CURSOR");
        let(s,_,v)=call(&router,&format!("{base}/files?cursor={file_cursor}"),"GET",None,true,None).await;assert_eq!(s,StatusCode::BAD_REQUEST);assert_eq!(v["error"]["code"],"INVALID_CURSOR");
        let nested=root.path().join("nested");fs::create_dir(&nested).unwrap();assert!(Command::new("git").args(["init","--quiet"]).current_dir(&nested).status().unwrap().success());fs::write(nested.join("secret.py"),"def inner_secret():\n    pass\n").unwrap();let secret=index(&nested);
        fs::write(root.path().join(".kortex.toml"),"[repository]\nroot='nested'\n").unwrap();
        let(s,_,v)=call(&router,&format!("{base}/status"),"GET",None,true,None).await;assert_eq!(s,StatusCode::OK,"{v}");assert_eq!(v["repo_id"],snapshot.repo_id);assert_ne!(v["repo_id"],secret.repo_id);assert_eq!(v["data"]["root"],snapshot.root);
    });
}
#[test]
fn startup_policy_and_schemas() {
    let id = "a".repeat(64);
    let mut c = Config::loopback(TOKEN.into(), [id]);
    assert!(c.validate().is_ok());
    c.bearer_token.clear();
    assert_eq!(c.validate().unwrap_err().code, "AUTH_REQUIRED");
    c.address = "0.0.0.0:7337".parse().unwrap();
    assert_eq!(c.validate().unwrap_err().code, "AUTH_REQUIRED");
    c.bearer_token = TOKEN.into();
    assert!(c.validate().is_ok()); // Validation only; no remote listener launched.
    c.allowed_repos = BTreeSet::from(["/tmp/secret".into()]);
    assert!(c.validate().is_err());
    c.allowed_repos = BTreeSet::from(["b".repeat(64)]);
    for origin in [
        "*",
        "http://*.example.com",
        "https://example.com/path",
        "https://user@example.com",
        "null",
    ] {
        c.cors_origins = BTreeSet::from([origin.into()]);
        assert!(c.validate().is_err(), "{origin}");
    }
}
