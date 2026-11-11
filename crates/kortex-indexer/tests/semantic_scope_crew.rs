//! Namespace, declaration-name and lexical scope false-positive guards.
use kortex_config::Config;
use kortex_core::{Edge, Snapshot};
use kortex_indexer::{Options, index};
use std::fs;
fn graph(path: &str, source: &str) -> Snapshot {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(path), source).unwrap();
    index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot
}
fn named<'a>(s: &'a Snapshot, kind: &str, name: &str) -> Vec<&'a Edge> {
    s.files
        .iter()
        .flat_map(|f| &f.references)
        .filter(|r| r.name == name && r.kind == kind)
        .map(|r| {
            s.edges
                .iter()
                .find(|e| e.kind == kind && e.source_id == r.source_id && e.source_span == r.span)
                .unwrap()
        })
        .collect()
}
#[test]
fn generic_parameters_do_not_resolve_to_unrelated_concrete_types() {
    for (path, source) in [
        (
            "a.rs",
            "struct T; trait Bound {} fn entry<T: Bound>(arg: T) {}",
        ),
        (
            "a.ts",
            "class T {} interface Bound {} function entry<T extends Bound>(arg: T) {}",
        ),
        (
            "a.go",
            "package app\ntype T struct {}\nfunc entry[T any](arg T) {}",
        ),
    ] {
        let s = graph(path, source);
        let refs = named(&s, "USES_TYPE", "T");
        assert_eq!(refs.len(), 1, "{path}: {:#?}", s.files);
        assert_eq!(refs[0].resolution, "unresolved", "{path}: {:#?}", refs[0]);
        if !path.ends_with("go") {
            let bounds = named(&s, "USES_TYPE", "Bound");
            assert_eq!(bounds.len(), 1, "{path}: {:#?}", s.files);
            assert_eq!(bounds[0].resolution, "exact");
        }
    }
}
#[test]
fn block_declarations_and_future_python_bindings_do_not_leak() {
    for (path, source, kind, name) in [
        (
            "a.rs",
            "fn entry() { { fn hidden() {} } hidden(); }",
            "CALLS",
            "hidden",
        ),
        (
            "a.ts",
            "function entry() { { function hidden() {} } hidden(); }",
            "CALLS",
            "hidden",
        ),
        (
            "a.ts",
            "const value = 1; function entry() { consume(value); const value = 2; }",
            "REFERENCES",
            "value",
        ),
    ] {
        let s = graph(path, source);
        let refs = named(&s, kind, name);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].resolution, "unresolved", "{path}: {:#?}", refs[0]);
    }
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("a.py"),
        "from other import value\ndef entry():\n    consume(value)\n    value = 2\n",
    )
    .unwrap();
    fs::write(d.path().join("other.py"), "value = 1\n").unwrap();
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(named(&s, "REFERENCES", "value")[0].resolution, "unresolved");
}
#[test]
fn var_closures_and_multi_name_parameters_have_lexical_value_bindings() {
    for (path, source, name) in [
        (
            "a.js",
            "function entry() { { var value = 1; } return value; }",
            "value",
        ),
        (
            "a.go",
            "package app\nfunc entry(x, y int) int { return y }",
            "y",
        ),
        (
            "a.rs",
            "fn entry() { let value = 1; let closure = |value| value; }",
            "value",
        ),
        (
            "a.py",
            "value = 1\ndef entry(): return (lambda value: value)(2)\n",
            "value",
        ),
        (
            "a.js",
            "function entry() { try {} catch (value) { consume(value); } }",
            "value",
        ),
    ] {
        let s = graph(path, source);
        let refs = named(&s, "REFERENCES", name);
        assert_eq!(refs.len(), 1, "{path}: {:#?}", s.files);
        assert_eq!(refs[0].resolution, "exact", "{path}: {:#?}", refs[0]);
        let target = s.symbol(refs[0].target_id.as_deref().unwrap()).unwrap();
        assert_eq!(target.kind, "variable");
        if path.ends_with("rs") || path.ends_with("py") {
            assert_eq!(target.attributes["parameter"], true);
        }
    }
}
#[test]
fn destructuring_default_values_and_typeof_are_value_uses_not_declarations() {
    let s = graph(
        "a.ts",
        "const fallback = 1; function factory() {} function entry() { const {key = fallback} = {}; return key; } type Factory = typeof factory;",
    );
    let fallbacks: Vec<_> = s.symbols().filter(|s| s.name == "fallback").collect();
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(named(&s, "REFERENCES", "fallback")[0].resolution, "exact");
    assert_eq!(named(&s, "REFERENCES", "factory")[0].resolution, "exact");
    assert!(named(&s, "USES_TYPE", "factory").is_empty());
    assert_eq!(named(&s, "REFERENCES", "key").len(), 1);
}
#[test]
fn type_alias_value_namespace_visibility_and_explicit_uncertainty() {
    let s = graph(
        "a.ts",
        "type Model = {}; const Model = 1; class Holder { private secret: Model; protected other: Model; } function entry(arg: Model) { return Model; }",
    );
    assert_eq!(named(&s, "REFERENCES", "Model").len(), 1);
    let value = named(&s, "REFERENCES", "Model")[0];
    assert_eq!(value.resolution, "exact");
    assert_eq!(
        s.symbol(value.target_id.as_deref().unwrap()).unwrap().kind,
        "variable"
    );
    assert!(
        named(&s, "USES_TYPE", "Model")
            .iter()
            .all(|e| e.resolution == "exact"
                && s.symbol(e.target_id.as_deref().unwrap()).unwrap().kind == "type_alias")
    );
    assert!(
        s.symbols()
            .any(|s| s.name == "secret" && s.visibility == "private")
    );
    assert!(
        s.symbols()
            .any(|s| s.name == "other" && s.visibility == "protected")
    );
    let s = graph(
        "a.py",
        "value = 1\ndef entry():\n    global value\n    value = 2\n    return value\n",
    );
    let refs = named(&s, "REFERENCES", "value");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].resolution, "unresolved");
    assert!(refs[0].evidence.contains("global/nonlocal"));
}
#[test]
fn reassignments_are_not_declarations_or_known_callable_types() {
    for (path, source) in [
        (
            "a.rs",
            "fn entry() { let mut value = 1; value = 2; consume(value); }",
        ),
        (
            "a.js",
            "function entry() { let value = 1; value = 2; consume(value); }",
        ),
    ] {
        let s = graph(path, source);
        assert_eq!(s.symbols().filter(|s| s.name == "value").count(), 1);
        assert_eq!(named(&s, "REFERENCES", "value")[0].resolution, "exact");
    }
    let s = graph(
        "a.js",
        "function check() {} function entry() { check = other; check(); }",
    );
    assert_eq!(named(&s, "CALLS", "check")[0].resolution, "unresolved");
}
#[test]
fn rust_runtime_receiver_syntax_is_not_confused_with_associated_paths() {
    let s = graph(
        "a.rs",
        "struct Model; impl Model { fn method() {} const VALUE: i32 = 1; } fn entry() { Model::method(); Model.method(); let a = Model::VALUE; let b = Model.VALUE; }",
    );
    for (kind, name) in [("CALLS", "method"), ("REFERENCES", "VALUE")] {
        let refs = named(&s, kind, name);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].resolution, "exact");
        assert_eq!(refs[1].resolution, "unresolved");
        assert!(refs[1].target_id.is_none());
        assert!(refs[1].evidence.contains("runtime receiver"));
    }
}
#[test]
fn private_rust_types_and_instance_fields_do_not_become_public_or_associated() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("lib.rs"), "mod other; use crate::other::Hidden; struct Model { field: i32 } fn entry(arg: Hidden) { let v = Model::field; }").unwrap();
    fs::write(d.path().join("other.rs"), "struct Hidden;").unwrap();
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(named(&s, "USES_TYPE", "Hidden")[0].resolution, "unresolved");
    assert_eq!(named(&s, "REFERENCES", "field")[0].resolution, "unresolved");
}
