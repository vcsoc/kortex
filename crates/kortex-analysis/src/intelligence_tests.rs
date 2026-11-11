use super::*;
use std::fs;
use tempfile::TempDir;
#[path = "intelligence_extra_tests.rs"]
mod extra;
#[path = "inventory_tests.rs"]
mod inventory;
#[path = "schema_tests.rs"]
mod schema;
fn indexed(files: &[(&str, &str)]) -> (TempDir, Service) {
    let temp = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let p = temp.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, source).unwrap();
    }
    kortex_indexer::index(temp.path(), &Config::default(), &Default::default()).unwrap();
    let service = Service::open(temp.path()).unwrap();
    (temp, service)
}
#[test]
fn actual_fastapi_flask_express_routes_and_negative_names() {
    let (_temp, service) = indexed(&[
        (
            "api.py",
            "from fastapi import FastAPI as API\napp = API()\ndef persist():\n    pass\n@app.get('/users')\ndef users():\n    persist()\ndef test_users():\n    users()\n",
        ),
        (
            "flask_api.py",
            "from flask import Flask\nweb = Flask(__name__)\n@web.route('/save', methods=['POST', 'PUT'])\ndef save():\n    pass\n",
        ),
        (
            "api.js",
            "import express from 'express';\nconst app = express();\nfunction users() {}\napp.get('/js', users);\n",
        ),
        (
            "fake.py",
            "class Fake:\n    def get(self, x):\n        pass\napp = Fake()\n@app.get('/not-a-route')\ndef fake():\n    pass\n",
        ),
    ]);
    let result = service.execute("routes", &json!({"limit":20})).unwrap();
    let routes = result["data"]["items"].as_array().unwrap();
    assert_eq!(routes.len(), 4, "{result:#}");
    assert!(routes.iter().all(|r| r["path"] != "/not-a-route"));
    let users = routes.iter().find(|r| r["path"] == "/users").unwrap();
    assert_eq!(users["framework"], "fastapi");
    assert_eq!(users["consumers"].as_array().unwrap().len(), 1);
    assert!(
        users["tests"].as_array().unwrap().is_empty(),
        "a test_ name alone is not test proof"
    );
    assert!(!users["process"]["items"].as_array().unwrap().is_empty());
    assert_eq!(
        users["provenance"]["source_hash"],
        service
            .snapshot
            .files
            .iter()
            .find(|f| f.path == "api.py")
            .unwrap()
            .content_hash
    );
    assert_eq!(
        service
            .execute("routes", &json!({"framework":"flask"}))
            .unwrap()["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn route_shadowing_dynamic_ambiguity_and_changed_source_refuse_facts() {
    let (temp, service) = indexed(&[
        (
            "shadow.py",
            "from fastapi import FastAPI\napp = FastAPI()\napp = Fake()\n@app.get('/fake')\ndef wrong():\n    pass\n",
        ),
        (
            "dynamic.py",
            "from fastapi import FastAPI\napp = FastAPI()\nPATH = '/dynamic'\n@app.get(PATH)\ndef dynamic():\n    pass\n",
        ),
        (
            "ambiguous.js",
            "import express from 'express';\nconst app = express();\nfunction route() {}\nfunction route() {}\napp.get('/ambiguous', route);\n",
        ),
        (
            "valid.py",
            "from fastapi import FastAPI\napp = FastAPI()\n@app.get('/valid')\ndef valid():\n    pass\n",
        ),
    ]);
    fs::write(temp.path().join("valid.py"), "# stale source\n").unwrap();
    let response = service.execute("routes", &json!({})).unwrap();
    assert!(
        response["data"]["items"].as_array().unwrap().is_empty(),
        "{response:#}"
    );
    assert_eq!(response["partial"], true);
    assert!(response["data"]["diagnostics"].as_array().unwrap().len() >= 3);
}
fn rule() -> Value {
    json!({"id":"no-ui-db","kind":"forbid_dependency","from":{"path":"ui/**","kinds":["function"]},"to":{"path":"db/**"},"edge_kinds":["CALLS"],"max_depth":2})
}
#[test]
fn architecture_violations_are_real_paths_and_ci_failure_is_not_transport_error() {
    let (_temp, service) = indexed(&[
        ("db/store.py", "def save():\n    pass\n"),
        (
            "ui/view.py",
            "from db.store import save\ndef render():\n    save()\n",
        ),
    ]);
    let result = service
        .execute("check", &json!({"rules":[rule()]}))
        .unwrap();
    assert_eq!(result["data"]["exit_code"], 10, "{result:#}");
    assert_eq!(result["data"]["passed"], false);
    let violation = &result["data"]["violations"][0];
    assert_eq!(violation["edges"][0]["kind"], "CALLS");
    assert_eq!(violation["edges"][0]["resolution"], "exact");
    let mut allowed = rule();
    allowed["to"]["path"] = json!("other/**");
    let clean = service
        .execute("check", &json!({"rules":[allowed]}))
        .unwrap();
    assert_eq!(clean["data"]["exit_code"], 0, "{clean:#}");
    assert_eq!(clean["data"]["complete"], true);
}
#[test]
fn architecture_unknown_targets_and_bounded_reports_cannot_pass_ci() {
    let (_temp, service) = indexed(&[("ui/view.py", "def render():\n    unavailable()\n")]);
    let result = service
        .execute("check", &json!({"rules":[rule()]}))
        .unwrap();
    assert_eq!(result["data"]["exit_code"], 7, "{result:#}");
    assert_eq!(result["data"]["complete"], false);
    assert!(!result["data"]["uncertain"].as_array().unwrap().is_empty());
    for invalid in [
        json!({"rules":[]}),
        json!({"rules":[rule(),rule()]}),
        json!({"rules_file":"../../remote.json","rules":[rule()]}),
        json!({"rules_version":"2.0","rules":[rule()]}),
    ] {
        assert!(service.execute("check", &invalid).is_err());
    }
    let mut traversal = rule();
    traversal["from"]["path"] = json!("../escape");
    assert!(
        service
            .execute("check", &json!({"rules":[traversal]}))
            .is_err()
    );
}
#[test]
fn shared_cursors_pin_generation_request_and_filters() {
    let (_temp, mut service) = indexed(&[
        (
            "a.py",
            "def thing_one():\n    pass\ndef thing_two():\n    pass\n",
        ),
        ("b.rs", "fn thing_three() {}\n"),
    ]);
    let first = service
        .execute(
            "query",
            &json!({"text":"thing","limit":1,"language":"python","kind":"function"}),
        )
        .unwrap();
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    assert!(cursor.starts_with("kx1."));
    let args =
        json!({"text":"thing","limit":1,"language":"python","kind":"function","cursor":cursor});
    let second = service.execute("query", &args).unwrap();
    assert_ne!(
        first["data"]["items"][0]["symbol"]["symbol_id"],
        second["data"]["items"][0]["symbol"]["symbol_id"]
    );
    let mut changed = args.clone();
    changed["language"] = json!("rust");
    assert_eq!(
        service.execute("query", &changed).unwrap_err().code,
        "INVALID_CURSOR"
    );
    service.snapshot.generation_id = "different".into();
    assert_eq!(
        service.execute("query", &args).unwrap_err().code,
        "INVALID_CURSOR"
    );
    assert_eq!(
        service
            .execute("query", &json!({"text":"thing","cursor":"0"}))
            .unwrap()["data"]["operation"],
        "query"
    );
}
#[test]
fn operation_schema_exports_and_validation_are_not_generic_data_placeholders() {
    let (_temp, service) = indexed(&[("a.py", "def a():\n    pass\n")]);
    for op in [
        "status", "query", "context", "impact", "trace", "graph", "changes", "routes", "check",
    ] {
        let input = input_schema(op);
        let output = response_schema(op);
        assert_eq!(input["additionalProperties"], false);
        assert!(
            output["properties"]["data"]["properties"].is_object(),
            "{op}"
        );
        assert_eq!(
            output["properties"]["data"]["properties"]["operation"]["const"],
            op
        );
    }
    assert_eq!(
        service
            .execute("context", &json!({"symbol":"a","unknown":true}))
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        service
            .execute("status", &json!({"generation_id":"wrong"}))
            .unwrap_err()
            .code,
        "GENERATION_MISMATCH"
    );
    assert_eq!(
        service
            .execute("context", &json!({"symbol":"a","max_response_bytes":1024}))
            .unwrap_err()
            .code,
        "RESPONSE_BUDGET"
    );
}
