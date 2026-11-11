use super::{fixture, git, names, service};
use serde_json::json;
use std::fs;

#[test]
fn encoded_worktree_does_not_claim_raw_source_hunk_coordinates() {
    let old = "fn old() {}\n";
    let new = "fn current() {}\n";
    let temp = fixture("code.rs", old);
    fs::write(
        temp.path().join(".gitattributes"),
        "*.rs working-tree-encoding=UTF-8\n",
    )
    .unwrap();
    fs::write(temp.path().join("code.rs"), new).unwrap();
    let result = service(temp.path(), &[("code.rs", new)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(result["partial"], true);
    assert_eq!(names(&result), Vec::<&str>::new());
    assert!(
        result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("DIFF_ENCODING"))
    );
    let before = service(temp.path(), &[("code.rs", old)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&before), ["old"]);
}

#[test]
fn quoted_unicode_patch_paths_preserve_hunk_mapping() {
    let path = "quote\"-\u{e9}-\t-name.rs";
    let old = "fn old() {}\n";
    let temp = fixture(path, old);
    fs::write(temp.path().join(path), "fn new() {}\n").unwrap();
    let result = service(temp.path(), &[(path, old)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&result), ["old"]);
    assert_eq!(result["changes"][0]["hunks"].as_array().unwrap().len(), 1);
}

#[test]
fn truncated_change_analysis_is_reflected_in_public_envelope() {
    let old = "fn old() {}\n";
    let temp = fixture("code.rs", old);
    fs::write(temp.path().join("code.rs"), "fn new() {}\n").unwrap();
    let result = service(temp.path(), &[("code.rs", old)])
        .execute("changes", &json!({"max_nodes":0}))
        .unwrap();
    assert_eq!(result["truncated"], true);
    assert_eq!(result["data"]["truncated"], true);
    assert_eq!(result["data"]["affected_symbols"], json!([]));
}

#[test]
fn fresh_worktree_index_still_reports_partial_for_unrepresented_commit_pair() {
    let old = "fn old() {}\n";
    let temp = fixture("code.rs", old);
    let base = git(temp.path(), &["rev-parse", "HEAD"]).trim().to_owned();
    fs::write(temp.path().join("code.rs"), "fn committed() {}\n").unwrap();
    git(temp.path(), &["add", "--all"]);
    git(temp.path(), &["commit", "-qm", "second"]);
    let worktree = "fn indexed_worktree() {}\n";
    fs::write(temp.path().join("code.rs"), worktree).unwrap();
    let result = service(temp.path(), &[("code.rs", worktree)])
        .execute("changes", &json!({"base":base,"head":"HEAD"}))
        .unwrap();
    assert_eq!(result["index_status"], "fresh");
    assert_eq!(result["partial"], true);
    assert_eq!(names(&result["data"]), Vec::<&str>::new());
    assert!(!result["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn configured_clean_and_process_filters_are_never_executed() {
    for kind in ["clean", "process"] {
        let old = "fn old() {}\n";
        let temp = fixture("code.rs", old);
        let marker = temp.path().join("filter-was-executed");
        git(
            temp.path(),
            &[
                "config",
                &format!("filter.hostile.{kind}"),
                &format!("touch '{}'", marker.display()),
            ],
        );
        git(temp.path(), &["config", "filter.hostile.required", "true"]);
        fs::write(temp.path().join(".gitattributes"), "*.rs filter=hostile\n").unwrap();
        fs::write(temp.path().join("code.rs"), "fn new() {}\n").unwrap();
        let result = service(temp.path(), &[("code.rs", old)])
            .changes(&json!({}))
            .unwrap();
        assert_eq!(names(&result), ["old"]);
        assert!(!marker.exists(), "Git executed a {kind} filter");
    }
}

#[test]
fn edited_rename_uses_coordinates_from_indexed_path_side() {
    let old = "fn a() {}\n\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
    let new = "fn a() {}\n\nfn inserted() {}\n\n\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
    let temp = fixture("old.rs", old);
    fs::remove_file(temp.path().join("old.rs")).unwrap();
    fs::write(temp.path().join("new.rs"), new).unwrap();
    git(temp.path(), &["add", "--all"]);
    for (path, source, expected) in [("old.rs", old, "file"), ("new.rs", new, "inserted")] {
        let result = service(temp.path(), &[(path, source)])
            .changes(&json!({"staged":true}))
            .unwrap();
        assert!(
            result["changes"][0]["status"]
                .as_str()
                .unwrap()
                .starts_with('R')
        );
        assert_eq!(names(&result), [expected]);
    }
}

#[cfg(unix)]
#[test]
fn backslash_filename_and_symlink_replacement_remain_explicit() {
    let path = "literal\\name.rs";
    let old = "fn old() {}\n";
    let temp = fixture(path, old);
    fs::write(temp.path().join(path), "fn new() {}\n").unwrap();
    let result = service(temp.path(), &[(path, old)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(result["changes"][0]["path"], path);
    assert_eq!(names(&result), ["old"]);
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.rs");
    fs::write(&secret, "fn secret() {}\n").unwrap();
    fs::remove_file(temp.path().join(path)).unwrap();
    std::os::unix::fs::symlink(&secret, temp.path().join(path)).unwrap();
    let result = service(temp.path(), &[(path, "fn secret() {}\n")])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(result["partial"], true);
    assert_eq!(names(&result), Vec::<&str>::new());
}
