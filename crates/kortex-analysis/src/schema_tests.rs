use super::*;
#[test]
fn real_outputs_and_inputs_validate_against_exported_json_schemas() {
    let (temp, _service) = indexed(&[(
        "api.py",
        "from fastapi import FastAPI\napp = FastAPI()\ndef worker():\n    pass\n@app.get('/value')\ndef value():\n    worker()\n",
    )]);
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    fs::write(temp.path().join(".gitignore"), ".kortex/\n").unwrap();
    git(&["init", "-q"]);
    git(&["add", "--all"]);
    git(&[
        "-c",
        "user.name=Schema Test",
        "-c",
        "user.email=schema@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "fixture",
    ]);
    kortex_indexer::index(temp.path(), &Config::default(), &Default::default()).unwrap();
    let service = Service::open(temp.path()).unwrap();
    for (operation, args) in [
        ("status", json!({})),
        ("files", json!({})),
        ("query", json!({"text":"value"})),
        ("context", json!({"symbol":"value"})),
        ("impact", json!({"symbol":"worker"})),
        ("trace", json!({"from":"value","to":"worker"})),
        ("graph", json!({"symbol":"value"})),
        ("changes", json!({"unstaged":true})),
        ("routes", json!({})),
        (
            "check",
            json!({"rules":[{"id":"no-other","kind":"forbid_dependency","from":{"path":"api.py","kinds":["function"]},"to":{"path":"api.py","kinds":["function"]}}]}),
        ),
    ] {
        let input = input_schema(operation);
        let input_validator = jsonschema::validator_for(&input).unwrap();
        input_validator.validate(&args).unwrap();
        let original = fs::read_to_string(temp.path().join("api.py")).unwrap();
        if operation == "changes" {
            fs::write(
                temp.path().join("api.py"),
                format!("{original}# modified fixture\n"),
            )
            .unwrap();
        }
        let result = service.execute(operation, &args).unwrap();
        if operation == "changes" {
            fs::write(temp.path().join("api.py"), original).unwrap();
            assert_eq!(result["data"]["changes"].as_array().unwrap().len(), 1);
        }
        if operation == "check" {
            assert_eq!(result["data"]["exit_code"], 10);
            assert!(!result["data"]["violations"].as_array().unwrap().is_empty());
        }
        let output = response_schema(operation);
        let validator = jsonschema::validator_for(&output).unwrap();
        assert!(
            validator.is_valid(&result),
            "{operation}: {}\n{result:#}",
            validator
                .iter_errors(&result)
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
        let mut broken = result.clone();
        broken["data"]["operation"] = json!("not-the-operation");
        assert!(!validator.is_valid(&broken));
    }
    for operation in [
        "query",
        "context",
        "impact",
        "trace",
        "graph",
        "changes",
        "status",
        "routes",
        "check",
        "list_repos",
        "files",
    ] {
        assert!(jsonschema::validator_for(&input_schema(operation)).is_ok());
        assert!(jsonschema::validator_for(&response_schema(operation)).is_ok());
    }
}
