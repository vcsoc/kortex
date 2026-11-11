use super::{fixture, git, service};
use serde_json::json;
use std::fs;

#[test]
fn submodule_status_cannot_execute_nested_clean_filters() {
    let child = fixture("code.rs", "fn child() {}\n");
    let parent = fixture("parent.rs", "fn parent() {}\n");
    git(
        parent.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--",
            child.path().to_str().unwrap(),
            "dependency",
        ],
    );
    git(parent.path(), &["commit", "-qam", "add local dependency"]);
    git(parent.path(), &["config", "diff.submodule", "diff"]);
    let nested = parent.path().join("dependency");
    let marker = parent.path().join("nested-filter-executed");
    git(
        &nested,
        &[
            "config",
            "filter.nested.clean",
            &format!("touch '{}'; cat", marker.display()),
        ],
    );
    fs::write(nested.join(".gitattributes"), "*.rs filter=nested\n").unwrap();
    fs::write(nested.join("code.rs"), "fn modified_child() {}\n").unwrap();
    let result = service(parent.path(), &[("parent.rs", "fn parent() {}\n")])
        .changes(&json!({}))
        .unwrap();
    assert!(
        !marker.exists(),
        "Git recursively executed a submodule clean filter"
    );
    assert_eq!(result["partial"], true);
    assert!(
        result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("submodule"))
    );
}
