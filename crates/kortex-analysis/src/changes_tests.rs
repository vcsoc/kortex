use super::*;
use kortex_core::{ParsedFile, Span};
use std::{fs, process::Command};
use tempfile::TempDir;

mod safety {
    include!("changes_safety_tests.rs");
}
mod submodules {
    include!("submodule_safety_test.rs");
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn fixture(path: &str, source: &str) -> TempDir {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(temp.path(), &["config", "user.name", "Kortex test"]);
    git(
        temp.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    fs::write(temp.path().join(".gitignore"), ".kortex/\n").unwrap();
    fs::write(temp.path().join(path), source).unwrap();
    git(temp.path(), &["add", "--all"]);
    git(temp.path(), &["commit", "-qm", "fixture"]);
    temp
}
// These fixture functions are single-line declarations, with exact fixture spans.
// No parser/graph dependencies or synthetic CALLS/routes/tests are introduced.
fn indexed_file(path: &str, source: &str) -> ParsedFile {
    let make = |name: &str, kind: &str, span: Span| Symbol {
        symbol_id: format!("{path}:{name}"),
        symbol_revision_id: hash(source),
        repo_id: "fixture".into(),
        kind: kind.into(),
        language: "rust".into(),
        name: name.into(),
        qualified_name: name.into(),
        file_path: path.into(),
        span,
        signature: name.into(),
        visibility: "private".into(),
        attributes: json!({}),
        content_hash: hash(source),
        doc_comment: None,
        owner_id: None,
    };
    let mut symbols = vec![make(
        "file",
        "file",
        Span {
            start_line: 1,
            end_line: source.lines().count().max(1),
            end_byte: source.len(),
            ..Span::default()
        },
    )];
    let mut offset = 0;
    for (line, text) in source.split_inclusive('\n').enumerate() {
        if let Some(name) = text
            .strip_prefix("fn ")
            .and_then(|s| s.split_once('(').map(|(name, _)| name))
        {
            symbols.push(make(
                name,
                "function",
                Span {
                    start_line: line + 1,
                    end_line: line + 1,
                    start_byte: offset,
                    end_byte: offset + text.trim_end().len(),
                    ..Span::default()
                },
            ));
        }
        offset += text.len();
    }
    ParsedFile {
        path: path.into(),
        language: "rust".into(),
        content_hash: hash(source),
        size_bytes: source.len(),
        symbols,
        references: vec![],
        imports: vec![],
        import_scopes: vec![],
        import_spans: vec![],
        diagnostics: vec![],
    }
}
fn service(root: &Path, files: &[(&str, &str)]) -> Service {
    let config = Config::default();
    Service {
        root: root.canonicalize().unwrap(),
        snapshot: Snapshot {
            root: kortex_core::public_path(&root.canonicalize().unwrap()),
            git_head: kortex_repo::head(root),
            config_hash: config.fingerprint().unwrap(),
            grammar_hash: kortex_core::PARSER_VERSION.into(),
            resolution_hash: kortex_repo::inputs::resolution_fingerprint(
                &root.canonicalize().unwrap(),
            ),
            files: files
                .iter()
                .map(|(path, source)| indexed_file(path, source))
                .collect(),
            ..Snapshot::default()
        },
        store: Store::open(root, true).unwrap(),
        config,
    }
}
fn names(data: &Value) -> Vec<&str> {
    data["changed_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect()
}

#[test]
fn insertion_before_symbols_never_uses_new_coordinates_on_old_index() {
    let old = "fn a() {}\n\nfn b() {}\n";
    let new = "fn a() {}\n\nfn inserted() {}\n\n\n\n\nfn b() {}\n";
    let temp = fixture("code.rs", old);
    fs::write(temp.path().join("code.rs"), new).unwrap();
    let before = service(temp.path(), &[("code.rs", old)])
        .changes(&json!({"unstaged":true}))
        .unwrap();
    assert_eq!(names(&before), ["file"]);
    assert_eq!(before["changes"][0]["indexed_sides"], json!(["old"]));
    assert_eq!(before["changes"][0]["hunks"][0]["old_count"], 0);
    assert_eq!(before["changes"][0]["old_line_ranges"], json!([]));
    let after = service(temp.path(), &[("code.rs", new)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&after), ["inserted"]);
    assert_eq!(after["changes"][0]["indexed_sides"], json!(["new"]));
}

#[test]
fn pure_deletion_maps_removed_symbol_only_on_old_side() {
    let old = "fn a() {}\nfn gone() {}\nfn b() {}\n";
    let new = "fn a() {}\nfn b() {}\n";
    let temp = fixture("code.rs", old);
    fs::write(temp.path().join("code.rs"), new).unwrap();
    let before = service(temp.path(), &[("code.rs", old)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&before), ["gone"]);
    assert_eq!(before["changes"][0]["hunks"][0]["new_count"], 0);
    let after = service(temp.path(), &[("code.rs", new)])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&after), ["file"]);
    assert_eq!(after["changes"][0]["line_ranges"], json!([]));
}

#[test]
fn staged_rename_preserves_paths_and_uses_file_fallback_without_hunks() {
    let text = "fn a() {}\nfn b() {}\n";
    let temp = fixture("old.rs", text);
    git(temp.path(), &["mv", "old.rs", "new.rs"]);
    for path in ["old.rs", "new.rs"] {
        let result = service(temp.path(), &[(path, text)])
            .changes(&json!({"staged":true}))
            .unwrap();
        assert_eq!(result["changes"][0]["status"], "R100");
        assert_eq!(result["changes"][0]["old_path"], "old.rs");
        assert_eq!(result["changes"][0]["path"], "new.rs");
        assert_eq!(names(&result), ["file"]);
        assert_eq!(result["partial"], false);
    }
}

#[test]
fn added_deleted_and_empty_files_have_side_specific_evidence() {
    let text = "fn removed() {}\n";
    let temp = fixture("old.rs", text);
    fs::remove_file(temp.path().join("old.rs")).unwrap();
    fs::write(temp.path().join("new.rs"), "fn added() { panic!(); }\n").unwrap();
    git(temp.path(), &["add", "--all"]);
    let result = service(
        temp.path(),
        &[("old.rs", text), ("new.rs", "fn added() { panic!(); }\n")],
    )
    .changes(&json!({"staged":true}))
    .unwrap();
    assert_eq!(names(&result), ["added", "removed"]);
    assert_eq!(result["changes"].as_array().unwrap().len(), 2);
    assert_eq!(result["partial"], false);
    let missing = service(temp.path(), &[("old.rs", text)])
        .changes(&json!({"staged":true}))
        .unwrap();
    assert_eq!(missing["partial"], true);
    let empty = fixture("empty.rs", "");
    fs::remove_file(empty.path().join("empty.rs")).unwrap();
    let result = service(empty.path(), &[("empty.rs", "")])
        .changes(&json!({}))
        .unwrap();
    assert_eq!(names(&result), ["file"]);
    assert_eq!(result["changes"][0]["hunks"], json!([]));
}

#[test]
fn same_head_does_not_prove_index_matches_either_side() {
    let old = "fn old() {}\n";
    let temp = fixture("code.rs", old);
    fs::write(temp.path().join("code.rs"), "fn current() {}\n").unwrap();
    let service = service(
        temp.path(),
        &[("code.rs", "fn unrelated_generation() {}\n")],
    );
    let result = service.execute("changes", &json!({})).unwrap();
    assert_eq!(result["partial"], true);
    assert_eq!(result["index_status"], "stale");
    assert!(!result["warnings"].as_array().unwrap().is_empty());
    assert_eq!(names(&result["data"]), Vec::<&str>::new());
    assert_eq!(
        result["data"]["changes"][0]["mapping_status"],
        "unavailable"
    );
    assert_eq!(result["data"]["affected_symbols"], json!([]));
    assert_eq!(result["data"]["likely_tests"], json!([]));
}

#[test]
fn staged_content_is_not_worktree_and_commit_pair_ignores_worktree() {
    let old = "fn old() {}\n";
    let staged = "fn staged() {}\n";
    let worktree = "fn worktree() {}\n";
    let temp = fixture("code.rs", old);
    let base = git(temp.path(), &["rev-parse", "HEAD"]).trim().to_owned();
    fs::write(temp.path().join("code.rs"), staged).unwrap();
    git(temp.path(), &["add", "code.rs"]);
    fs::write(temp.path().join("code.rs"), worktree).unwrap();
    let result = service(temp.path(), &[("code.rs", worktree)])
        .changes(&json!({"staged":true}))
        .unwrap();
    assert_eq!(result["partial"], true);
    let result = service(temp.path(), &[("code.rs", staged)])
        .changes(&json!({"staged":true}))
        .unwrap();
    assert_eq!(names(&result), ["staged"]);
    let result = service(temp.path(), &[("code.rs", staged)])
        .changes(&json!({"unstaged":true}))
        .unwrap();
    assert_eq!(names(&result), ["staged"]);
    git(temp.path(), &["commit", "-qm", "staged"]);
    for (text, expected) in [(old, "old"), (staged, "staged")] {
        let result = service(temp.path(), &[("code.rs", text)])
            .changes(&json!({"base":base,"head":"HEAD"}))
            .unwrap();
        assert_eq!(names(&result), [expected]);
        assert_eq!(result["partial"], false);
    }
    let result = service(temp.path(), &[("code.rs", worktree)])
        .changes(&json!({"base":base}))
        .unwrap();
    assert_eq!(names(&result), ["worktree"]);
}

#[test]
fn literal_hostile_paths_and_diff_drivers_are_read_only() {
    let old = "fn old() {}\n";
    for path in [
        "--output=owned.rs",
        ":(glob)*.rs",
        "[x].rs",
        "space name.rs",
        "tab\tline\nname.rs",
    ] {
        let temp = fixture(path, old);
        fs::write(temp.path().join("other.rs"), old).unwrap();
        git(temp.path(), &["add", "--all"]);
        git(temp.path(), &["commit", "-qm", "other"]);
        git(temp.path(), &["config", "diff.external", "false"]);
        git(temp.path(), &["config", "diff.hostile.textconv", "false"]);
        fs::write(temp.path().join(".gitattributes"), "*.rs diff=hostile\n").unwrap();
        git(temp.path(), &["config", "diff.relative", "true"]);
        git(temp.path(), &["config", "color.ui", "always"]);
        fs::write(temp.path().join(path), "fn current() {}\n").unwrap();
        let result = service(temp.path(), &[(path, old), ("other.rs", old)])
            .changes(&json!({}))
            .unwrap();
        assert_eq!(result["changes"].as_array().unwrap().len(), 1);
        assert_eq!(result["changes"][0]["path"], path);
        assert_eq!(names(&result), ["old"], "path={path:?}, result={result}");
    }
}

#[test]
fn invalid_modes_and_bounds_are_validated_even_for_empty_diff() {
    let temp = fixture("code.rs", "fn a() {}\n");
    let service = service(temp.path(), &[("code.rs", "fn a() {}\n")]);
    for args in [
        json!({"staged":true,"unstaged":true}),
        json!({"head":"HEAD"}),
        json!({"base":17}),
        json!({"base":"--output=bad"}),
        json!({"depth":11}),
    ] {
        assert!(service.changes(&args).is_err(), "{args}");
    }
}

#[test]
fn unborn_staged_addition_is_supported() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    let text = "fn added() {}\n";
    fs::write(temp.path().join("new.rs"), text).unwrap();
    git(temp.path(), &["add", "--all"]);
    let result = service(temp.path(), &[("new.rs", text)])
        .changes(&json!({"staged":true}))
        .unwrap();
    assert_eq!(names(&result), ["added"]);
    assert_eq!(result["partial"], false);
}
