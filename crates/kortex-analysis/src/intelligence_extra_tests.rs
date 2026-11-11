use super::*;

#[test]
fn explicit_unittest_and_rust_test_attributes_have_reachable_association_proof() {
    let (_temp, service) = indexed(&[
        (
            "api.py",
            "from fastapi import FastAPI\nimport unittest\napp = FastAPI()\n@app.get('/value')\ndef value():\n    return 1\nclass ApiTests(unittest.TestCase):\n    def test_value(self):\n        value()\n",
        ),
        (
            "src/lib.rs",
            "fn value() {}\n#[test]\nfn exercises_value() { value(); }\n",
        ),
    ]);
    let routes = service.execute("routes", &json!({})).unwrap();
    let tests = routes["data"]["items"][0]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 1, "{routes:#}");
    assert_eq!(
        tests[0]["test_evidence"]["framework"],
        "unittest-testcase-method"
    );
    assert!(!tests[0]["edge_path"].as_array().unwrap().is_empty());
    let context = service
        .execute("context", &json!({"symbol":"src/lib.rs:value"}))
        .unwrap();
    assert_eq!(
        context["data"]["test_associations"][0]["test_evidence"]["framework"],
        "rust-test-attribute",
        "{context:#}"
    );
}
#[test]
fn opaque_route_pages_and_budget_shrink_never_skip_independent_items() {
    let source = format!(
        "from fastapi import FastAPI\napp = FastAPI()\n{}",
        (0..12)
            .map(|n| format!("@app.get('/r{n:02}')\ndef route_{n}():\n    pass\n"))
            .collect::<String>()
    );
    let (_temp, service) = indexed(&[("api.py", &source)]);
    let mut cursor = Value::Null;
    let mut ids = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let result=service.execute("routes",&json!({"limit":12,"max_response_bytes":7000,"include_processes":false,"cursor":cursor})).unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() <= 7000);
        for item in result["data"]["items"].as_array().unwrap() {
            assert!(ids.insert(item["route_id"].as_str().unwrap().to_owned()));
        }
        pages += 1;
        cursor = result["page"]["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
        assert!(pages < 20);
    }
    assert!(pages > 1);
    assert_eq!(ids.len(), 12);
}
#[test]
fn architecture_truncation_ambiguity_and_stale_generations_cannot_be_clean() {
    let (temp, service) = indexed(&[
        (
            "ui/a.py",
            "from db.a import save\ndef render():\n    save()\ndef other():\n    save()\n",
        ),
        ("db/a.py", "def save():\n    pass\n"),
    ]);
    let report = service
        .execute("check", &json!({"rules":[rule()],"max_violations":1}))
        .unwrap();
    assert_eq!(report["data"]["exit_code"], 10);
    assert_eq!(report["data"]["complete"], false);
    assert_eq!(report["truncated"], true);
    assert_eq!(report["data"]["evaluated"][0]["complete"], false);
    let mut safe = rule();
    safe["to"]["path"] = json!("other/**");
    fs::write(temp.path().join("db/a.py"), "def changed():\n    pass\n").unwrap();
    let stale = service.execute("check", &json!({"rules":[safe]})).unwrap();
    assert_eq!(stale["index_status"], "stale");
    assert_eq!(stale["data"]["exit_code"], 7);
    assert_eq!(stale["data"]["passed"], false);
    let (_ambiguous_temp, ambiguous) = indexed(&[
        (
            "ui/view.py",
            "from db.a import save\nfrom db.b import save\ndef render():\n    save()\n",
        ),
        ("db/a.py", "def save():\n    pass\n"),
        ("db/b.py", "def save():\n    pass\n"),
    ]);
    let uncertain = ambiguous
        .execute("check", &json!({"rules":[rule()]}))
        .unwrap();
    assert_eq!(uncertain["data"]["exit_code"], 7, "{uncertain:#}");
    assert!(
        uncertain["data"]["uncertain"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["resolution"] == "ambiguous")
    );
    assert!(
        uncertain["data"]["violations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn common_deadline_and_policy_overrides_are_validated() {
    let source = (0..4000)
        .map(|n| format!("def item_{n}():\n    pass\n"))
        .collect::<String>();
    let (_temp, service) = indexed(&[("many.py", &source)]);
    let error = service
        .execute("query", &json!({"text":"item","timeout_ms":1}))
        .unwrap_err();
    assert_eq!(error.exit, 9);
    for args in [
        json!({"text":"item","timeout_ms":10001}),
        json!({"text":"item","max_response_bytes":16777217}),
        json!({"text":"item","limit":0}),
    ] {
        assert_eq!(service.execute("query", &args).unwrap_err().exit, 2);
    }
}
#[test]
fn common_context_has_actual_reference_and_type_edges() {
    let (_temp, service) = indexed(&[(
        "a.py",
        "class Item:\n    pass\ndef consume(value: Item):\n    return value\ndef call():\n    consume(Item())\n",
    )]);
    let context = service
        .execute("context", &json!({"symbol":"consume"}))
        .unwrap();
    assert!(!context["data"]["callers"].as_array().unwrap().is_empty());
    assert!(context["data"]["references"].is_array());
    assert!(context["data"]["types"].is_array());
    for edge in context["data"]["types"].as_array().unwrap() {
        assert_eq!(edge["kind"], "USES_TYPE");
        assert!(!edge["evidence"].as_str().unwrap().is_empty());
    }
}

#[test]
fn framework_binding_order_shadowing_and_dynamic_method_lists_do_not_become_routes() {
    let (_temp, service) = indexed(&[
        (
            "late.py",
            "from fastapi import FastAPI\n@app.get('/late')\ndef handler():\n    pass\napp = FastAPI()\n",
        ),
        (
            "flask_api.py",
            "from flask import Flask\napp = Flask(__name__)\n@app.route('/dynamic', methods=['GET', METHODS])\ndef handler():\n    pass\n",
        ),
        (
            "late.js",
            "import express from 'express';\nconst app = express();\napp.get('/late-arrow', handler);\nconst handler = () => {};\n",
        ),
        (
            "nested.py",
            "from fastapi import FastAPI\napp = FastAPI()\ndef factory():\n    @app.get('/nested')\n    def handler():\n        pass\n",
        ),
        (
            "overwritten.py",
            "from fastapi import FastAPI\napp = FastAPI()\nfrom fake import app\n@app.get('/overwritten')\ndef handler():\n    pass\n",
        ),
    ]);
    let routes = service.execute("routes", &json!({})).unwrap();
    assert!(
        routes["data"]["items"].as_array().unwrap().is_empty(),
        "{routes:#}"
    );
    assert_eq!(routes["partial"], true);
}
#[test]
fn test_framework_shadowing_is_not_a_test_association() {
    let (_temp, service) = indexed(&[(
        "test_shadow.py",
        "import unittest\nunittest = Fake\ndef target():\n    pass\nclass LooksLikeTests(unittest.TestCase):\n    def test_target(self):\n        target()\n",
    )]);
    let context = service
        .execute("context", &json!({"symbol":"target"}))
        .unwrap();
    assert!(
        context["data"]["tests"].as_array().unwrap().is_empty(),
        "{context:#}"
    );
    assert!(
        context["data"]["test_associations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn framework_module_alias_commonjs_and_typescript_are_ast_evidenced() {
    let (_temp, service) = indexed(&[
        (
            "module.py",
            "import fastapi as framework\napp = framework.FastAPI()\n@app.post('/python')\ndef handler():\n    pass\n",
        ),
        (
            "common.js",
            "const express = require('express');\nconst app = express();\nfunction handler() {}\napp.put('/common', handler);\n",
        ),
        (
            "typed.ts",
            "import express from 'express';\nconst app = express();\nfunction handler(): void {}\napp.delete('/typed', handler);\n",
        ),
    ]);
    let routes = service.execute("routes", &json!({})).unwrap();
    assert_eq!(
        routes["data"]["items"].as_array().unwrap().len(),
        3,
        "{routes:#}"
    );
    for item in routes["data"]["items"].as_array().unwrap() {
        assert_eq!(item["resolution"], "exact");
        assert_eq!(
            item["provenance"]["generation_id"],
            service.snapshot.generation_id
        );
    }
}
