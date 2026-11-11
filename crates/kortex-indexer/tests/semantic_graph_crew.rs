//! Actual grammar -> persisted graph coverage for semantic value/type facts.
use kortex_config::Config;
use kortex_core::{Edge, Snapshot};
use kortex_indexer::{Options, index};
use std::fs;

fn fixture(files: &[(&str, &str)]) -> Snapshot {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    index(root.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot
}
fn sites<'a>(s: &'a Snapshot, kind: &str, name: &str) -> Vec<&'a Edge> {
    s.files
        .iter()
        .flat_map(|f| &f.references)
        .filter(|r| r.kind == kind && r.name == name)
        .map(|r| {
            s.edges
                .iter()
                .find(|e| e.kind == kind && e.source_id == r.source_id && e.source_span == r.span)
                .unwrap()
        })
        .collect()
}
fn exact(s: &Snapshot, kind: &str, name: &str, target_kind: &str, count: usize) {
    let edges = sites(s, kind, name);
    assert_eq!(
        edges.len(),
        count,
        "{kind} {name}: {edges:#?}; files: {:#?}",
        s.files
    );
    for edge in edges {
        assert_eq!(edge.resolution, "exact", "{edge:#?}");
        let target = s.symbol(edge.target_id.as_deref().unwrap()).unwrap();
        assert_eq!(target.kind, target_kind, "{target:#?}");
    }
}

#[test]
fn polyglot_values_types_members_and_no_declaration_or_callee_duplicates() {
    for (path, source, member_kind, limit_kind, typed) in [
        (
            "a.rs",
            "struct Model { value: i32 } const LIMIT: i32 = 1; fn entry(arg: Model) -> Model { let local = LIMIT; consume(local); arg }",
            "field",
            "constant",
            true,
        ),
        (
            "a.py",
            "class Model:\n    value: int = 1\nLIMIT = 1\ndef entry(arg: Model) -> Model:\n    local = LIMIT\n    consume(local)\n    return arg\n",
            "property",
            "variable",
            true,
        ),
        (
            "a.ts",
            "class Model { value: number = 1; } const LIMIT = 1; function entry(arg: Model): Model { const local = LIMIT; consume(local); return arg; }",
            "property",
            "variable",
            true,
        ),
        (
            "a.js",
            "class Model { value = 1; } const LIMIT = 1; function entry(arg) { const local = LIMIT; consume(local); return arg; }",
            "property",
            "variable",
            false,
        ),
        (
            "a.go",
            "package app\ntype Model struct { value int }\nconst LIMIT = 1\nfunc entry(arg Model) Model { local := LIMIT; consume(local); return arg }",
            "field",
            "constant",
            true,
        ),
    ] {
        let s = fixture(&[(path, source)]);
        assert!(
            s.files[0].diagnostics.is_empty(),
            "{path}: {:?}",
            s.files[0].diagnostics
        );
        exact(&s, "REFERENCES", "LIMIT", limit_kind, 1);
        exact(&s, "REFERENCES", "local", "variable", 1);
        exact(&s, "REFERENCES", "arg", "variable", 1);
        if typed {
            exact(
                &s,
                "USES_TYPE",
                "Model",
                if path.ends_with("rs") || path.ends_with("go") {
                    "struct"
                } else {
                    "class"
                },
                2,
            );
        }
        assert!(
            s.symbols()
                .any(|s| s.name == "value" && s.kind == member_kind),
            "{path}: {:#?}",
            s.files
        );
        assert!(sites(&s, "REFERENCES", "entry").is_empty());
        assert!(sites(&s, "REFERENCES", "consume").is_empty());
        assert!(sites(&s, "REFERENCES", "value").is_empty());
        assert_eq!(sites(&s, "CALLS", "consume").len(), 1);
    }
}

#[test]
fn polyglot_imported_values_and_types_have_source_evidence() {
    for (main, source, dep, definition, expected_type) in [
        (
            "lib.rs",
            "mod models; use crate::models::{Model, LIMIT}; fn entry(arg: Model) { let local = LIMIT; }",
            "models.rs",
            "pub struct Model; pub const LIMIT: i32 = 1;",
            "struct",
        ),
        (
            "main.py",
            "from models import Model, LIMIT\ndef entry(arg: Model):\n    return LIMIT\n",
            "models.py",
            "class Model: pass\nLIMIT = 1\n",
            "class",
        ),
        (
            "main.ts",
            "import {Model, LIMIT} from './models'; function entry(arg: Model) { return LIMIT; }",
            "models.ts",
            "export class Model {} export const LIMIT = 1;",
            "class",
        ),
        (
            "main.go",
            "package app\nimport \"app/models\"\nfunc entry(arg models.Model) { _ = models.LIMIT }",
            "models/model.go",
            "package models\ntype Model struct {}\nconst LIMIT = 1",
            "struct",
        ),
    ] {
        let s = fixture(&[
            (main, source),
            (dep, definition),
            ("go.mod", "module app\n"),
        ]);
        exact(&s, "USES_TYPE", "Model", expected_type, 1);
        exact(
            &s,
            "REFERENCES",
            "LIMIT",
            if main.ends_with("rs") || main.ends_with("go") {
                "constant"
            } else {
                "variable"
            },
            1,
        );
        for edge in
            sites(&s, "USES_TYPE", "Model")
                .into_iter()
                .chain(sites(&s, "REFERENCES", "LIMIT"))
        {
            assert!(edge.source_span.start_line >= 1 && edge.source_span.start_col >= 1);
            assert!(edge.evidence.contains(main));
            assert_eq!(
                s.symbol(edge.target_id.as_deref().unwrap())
                    .unwrap()
                    .file_path,
                dep
            );
        }
    }
}

#[test]
fn unsupported_receivers_missing_types_private_exports_and_wrong_kinds_stay_unresolved() {
    for (main, source, dep, definition, kind, name) in [
        (
            "a.rs",
            "fn Value() {} fn entry(arg: Value) {}",
            "other.rs",
            "struct Value;",
            "USES_TYPE",
            "Value",
        ),
        (
            "a.py",
            "Value = 1\ndef entry(arg: Value): pass\n",
            "other.py",
            "class Value: pass\n",
            "USES_TYPE",
            "Value",
        ),
        (
            "a.ts",
            "import {Hidden} from './other'; function entry(arg: Hidden) {}",
            "other.ts",
            "class Hidden {}",
            "USES_TYPE",
            "Hidden",
        ),
        (
            "a.ts",
            "class Model { value: number = 1; } function entry(obj: Model) { return obj.value; }",
            "other.ts",
            "export const value = 1;",
            "REFERENCES",
            "value",
        ),
        (
            "a.py",
            "class Model:\n    value = 1\ndef entry(obj): return obj.value\n",
            "other.py",
            "value = 1\n",
            "REFERENCES",
            "value",
        ),
        (
            "a.go",
            "package app\nfunc entry(obj int) { _ = obj.Value }",
            "other.go",
            "package app\nconst Value = 1",
            "REFERENCES",
            "Value",
        ),
        (
            "a.js",
            "import {value} from './other.js'; function entry(value) { return value.member; }",
            "other.js",
            "export const member = 1; export const value = {};",
            "REFERENCES",
            "member",
        ),
    ] {
        let s = fixture(&[(main, source), (dep, definition)]);
        let edges = sites(&s, kind, name);
        assert_eq!(edges.len(), 1, "{main}: {:#?}", s.files);
        assert_eq!(edges[0].resolution, "unresolved", "{main}: {:#?}", edges[0]);
        assert!(edges[0].target_id.is_none() && edges[0].candidates.is_empty());
    }
}

#[test]
fn ambiguous_types_and_values_retain_candidates_without_a_target() {
    for (source, kind, name) in [
        (
            "class Model {} class Model {} function entry(arg: Model) {}",
            "USES_TYPE",
            "Model",
        ),
        (
            "function value() {} function value() {} function entry() { return value; }",
            "REFERENCES",
            "value",
        ),
    ] {
        let s = fixture(&[("a.ts", source)]);
        let edges = sites(&s, kind, name);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].resolution, "ambiguous");
        assert!(edges[0].target_id.is_none());
        assert_eq!(edges[0].candidates.len(), 2);
        assert!(edges[0].candidates.iter().all(|c| s.symbol(c).is_some()));
    }
}

#[test]
fn block_shadowing_and_type_value_namespaces_are_not_conflated() {
    let s = fixture(&[(
        "a.ts",
        "class Model {} const value = 1; function entry(arg: Model) { { const value = 2; consume(value); } consume(value); const Model = 3; return Model; }",
    )]);
    exact(&s, "USES_TYPE", "Model", "class", 1);
    exact(&s, "REFERENCES", "Model", "variable", 1);
    let values = sites(&s, "REFERENCES", "value");
    assert_eq!(values.len(), 2);
    assert!(values.iter().all(|e| e.resolution == "exact"));
    assert_ne!(values[0].target_id, values[1].target_id);
    let s = fixture(&[(
        "a.py",
        "value = 1\ndef entry():\n    consume(value)\n    value = 2\n",
    )]);
    assert_eq!(
        sites(&s, "REFERENCES", "value")[0].resolution,
        "unresolved",
        "Python's later assignment blocks a global lookup"
    );
}

#[test]
fn namespace_properties_and_imported_function_values_are_not_calls() {
    for (main, source, dep, definition, name, target_kind) in [
        (
            "main.js",
            "import * as model from './model.js'; function entry() { return model.value; }",
            "model.js",
            "export const value = 1;",
            "value",
            "variable",
        ),
        (
            "main.py",
            "import model\ndef entry(): return model.value\n",
            "model.py",
            "value = 1\n",
            "value",
            "variable",
        ),
        (
            "main.ts",
            "import {handler as callback} from './model'; function entry() { return callback; }",
            "model.ts",
            "export function handler() {}",
            "callback",
            "function",
        ),
        (
            "main.py",
            "from model import handler as callback\ndef entry(): return callback\n",
            "model.py",
            "def handler(): pass\n",
            "callback",
            "function",
        ),
        (
            "lib.rs",
            "mod model; use crate::model::handler as callback; fn entry() { let cb = callback; }",
            "model.rs",
            "pub fn handler() {}",
            "callback",
            "function",
        ),
    ] {
        let s = fixture(&[(main, source), (dep, definition)]);
        exact(&s, "REFERENCES", name, target_kind, 1);
        assert!(s.edges.iter().all(|e| e.kind != "CALLS"));
    }
}
#[test]
fn semantic_edges_incremental_reuse_edit_rename_delete_match_full() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::default();
    fs::write(
        root.path().join("main.ts"),
        "import {Model, value} from './model'; function entry(arg: Model) { return value; }",
    )
    .unwrap();
    fs::write(
        root.path().join("model.ts"),
        "export class Model {} export const value = 1;",
    )
    .unwrap();
    index(root.path(), &config, &Options::default()).unwrap();
    let cached = index(root.path(), &config, &Options::default()).unwrap();
    assert_eq!(cached.reused, 2);
    assert!(
        cached
            .snapshot
            .edges
            .iter()
            .any(|e| e.kind == "USES_TYPE" && e.resolution == "exact")
    );
    for operation in ["edit", "rename", "delete"] {
        match operation {
            "edit" => fs::write(
                root.path().join("model.ts"),
                "export class Model { field: string = ''; } export const value = 2;",
            )
            .unwrap(),
            "rename" => {
                fs::rename(root.path().join("model.ts"), root.path().join("moved.ts")).unwrap()
            }
            _ => fs::remove_file(root.path().join("moved.ts")).unwrap(),
        }
        let incremental = index(root.path(), &config, &Options::default()).unwrap();
        let full = index(
            root.path(),
            &config,
            &Options {
                full: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(
            incremental.snapshot.files, full.snapshot.files,
            "{operation}"
        );
        assert_eq!(
            incremental.snapshot.edges, full.snapshot.edges,
            "{operation}"
        );
        if operation != "edit" {
            assert_eq!(
                sites(&full.snapshot, "USES_TYPE", "Model")[0].resolution,
                "unresolved"
            );
        }
    }
}
