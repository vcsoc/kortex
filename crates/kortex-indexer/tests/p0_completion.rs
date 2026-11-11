//! Real persisted P0 module/receiver/edge fixtures; no repository code is executed.
use kortex_config::Config;
use kortex_core::{Edge, Snapshot};
use kortex_indexer::{Options, index};
use std::fs;
fn setup(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let p = d.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, source).unwrap();
    }
    d
}
fn calls<'a>(s: &'a Snapshot, name: &str) -> Vec<&'a Edge> {
    s.files
        .iter()
        .flat_map(|f| &f.references)
        .filter(|r| r.kind == "CALLS" && r.name == name)
        .map(|r| {
            s.edges
                .iter()
                .find(|e| e.kind == r.kind && e.source_id == r.source_id && e.source_span == r.span)
                .unwrap()
        })
        .collect()
}
#[test]
fn commonjs_exports_aliases_namespaces_and_shadowed_loader() {
    let d = setup(&[
        (
            "auth.cjs",
            "function check(){} function hidden(){} module.exports={verify:check};",
        ),
        (
            "main.cjs",
            "const {verify: run} = require('./auth'); const auth=require('./auth'); function entry(){run();auth.verify();auth.hidden();}",
        ),
        (
            "shadow.cjs",
            "function entry(require){const auth=require('./auth');auth.verify();}",
        ),
    ]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(calls(&s, "run")[0].resolution, "exact", "{:#?}", s.edges);
    let verify = calls(&s, "verify");
    assert_eq!(
        verify.iter().filter(|e| e.resolution == "exact").count(),
        1,
        "{verify:#?}"
    );
    assert_eq!(
        verify
            .iter()
            .filter(|e| e.resolution == "unresolved")
            .count(),
        1
    );
    assert_eq!(calls(&s, "hidden")[0].resolution, "unresolved");
    assert!(s.edges.iter().any(|e| e.kind == "EXPORTS"));
}
#[test]
fn tsconfig_jsonc_alias_edit_reuses_facts_but_reresolves_globally() {
    let d = setup(&[
        (
            "src/main.ts",
            "import {check} from '@lib/auth'; function entry(){check();}",
        ),
        ("src/one/auth.ts", "export function check(){}"),
        ("src/two/auth.ts", "export function check(){}"),
        (
            "tsconfig.json",
            "{/*comment*/\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@lib/*\":[\"src/one/*\"],},},}",
        ),
    ]);
    let first = index(d.path(), &Config::default(), &Options::default()).unwrap();
    let target = calls(&first.snapshot, "check")[0]
        .target_id
        .as_deref()
        .unwrap();
    assert_eq!(
        first.snapshot.symbol(target).unwrap().file_path,
        "src/one/auth.ts"
    );
    fs::write(
        d.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"paths\":{\"@lib/*\":[\"src/two/*\"]}}}",
    )
    .unwrap();
    let incremental = index(d.path(), &Config::default(), &Options::default()).unwrap();
    assert_eq!(incremental.reused, 3);
    let target = calls(&incremental.snapshot, "check")[0]
        .target_id
        .as_deref()
        .unwrap();
    assert_eq!(
        incremental.snapshot.symbol(target).unwrap().file_path,
        "src/two/auth.ts"
    );
    let full = index(
        d.path(),
        &Config::default(),
        &Options {
            full: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(incremental.snapshot.edges, full.snapshot.edges);
    assert_eq!(incremental.snapshot.files, full.snapshot.files);
}
#[test]
fn explicit_rust_go_receivers_are_not_same_name_guesses() {
    for (path, source) in [
        (
            "a.rs",
            "struct Worker; impl Worker { fn run(&self) {} } fn entry(w: &Worker) { w.run(); } fn unknown(w: Unknown) { w.run(); }",
        ),
        (
            "a.go",
            "package app\ntype Worker struct {}\nfunc (w Worker) Run() {}\nfunc entry(w Worker){w.Run()}\nfunc unknown(w Unknown){w.Run()}",
        ),
    ] {
        let d = setup(&[(path, source)]);
        let s = index(d.path(), &Config::default(), &Options::default())
            .unwrap()
            .snapshot;
        let edges: Vec<_> = s.edges.iter().filter(|e| e.kind == "CALLS").collect();
        assert_eq!(
            edges.iter().filter(|e| e.resolution == "exact").count(),
            1,
            "{path}: {:#?}",
            s.files
        );
        assert_eq!(
            edges
                .iter()
                .filter(|e| e.resolution == "unresolved")
                .count(),
            1,
            "{path}: {edges:#?}"
        );
    }
}
#[test]
fn rust_trait_and_go_interface_dispatch_are_candidates_not_exact() {
    for (path, source) in [
        (
            "a.rs",
            "trait Job { fn run(&self); } struct A; struct B; impl Job for A {fn run(&self){}} impl Job for B {fn run(&self){}} fn entry(w: &dyn Job){w.run();}",
        ),
        (
            "a.go",
            "package app\ntype Job interface { Run() }\ntype A struct {}\ntype B struct {}\nfunc (a A) Run(){}\nfunc (b B) Run(){}\nfunc entry(w Job){w.Run()}",
        ),
    ] {
        let d = setup(&[(path, source)]);
        let s = index(d.path(), &Config::default(), &Options::default())
            .unwrap()
            .snapshot;
        let e = s.edges.iter().find(|e| e.kind == "CALLS").unwrap();
        assert_eq!(e.resolution, "ambiguous", "{path}: {:#?}", s.files);
        assert!(e.target_id.is_none());
        assert_eq!(e.candidates.len(), 2, "{path}: {e:#?}");
    }
}
#[test]
fn commonjs_direct_function_and_default_exports() {
    let d = setup(&[
        ("named.cjs", "exports.check = function(){ };"),
        ("default.cjs", "module.exports = function verify(){};"),
        (
            "main.cjs",
            "const named=require('./named');const verify=require('./default');function entry(){named.check();verify();}",
        ),
    ]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    for name in ["check", "verify"] {
        assert_eq!(calls(&s, name)[0].resolution, "exact", "{s:#?}");
    }
}
#[test]
fn rust_inline_glob_reexports_generic_traits_and_module_boundaries() {
    let d = setup(&[
        (
            "src/lib.rs",
            "mod inner {pub fn check() {}} mod api; mod service; use crate::api::*; use crate::service::Job; fn entry(){inner::check(); exported();} fn generic<T: Job>(w: &T){w.run();}",
        ),
        ("src/api.rs", "pub use crate::inner::check as exported;"),
        (
            "src/service.rs",
            "pub trait Job{fn run(&self);} struct A; impl Job for A{fn run(&self){}}",
        ),
    ]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(calls(&s, "check")[0].resolution, "exact", "{s:#?}");
    assert_eq!(calls(&s, "exported")[0].resolution, "exact", "{s:#?}");
    assert_eq!(calls(&s, "run")[0].resolution, "ambiguous", "{s:#?}");
    assert_eq!(calls(&s, "run")[0].candidates.len(), 1);
}
#[test]
fn go_cross_file_receiver_and_interface_method_shape_filter() {
    let d = setup(&[
        ("go.mod", "module app"),
        (
            "model.go",
            "package app\ntype Worker struct {}\ntype Job interface{Run(int)}\ntype Wrong struct {}",
        ),
        (
            "methods.go",
            "package app\nfunc(w Worker)Run(x int){}\nfunc(w Wrong)Run(x string){}\nfunc(w Worker)Extra(){}",
        ),
        (
            "main.go",
            "package app\nfunc entry(w Worker){w.Extra()}\nfunc invoke(w Job){w.Run(1)}",
        ),
    ]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(calls(&s, "Extra")[0].resolution, "exact", "{s:#?}");
    let dispatch = calls(&s, "Run")[0];
    assert_eq!(dispatch.resolution, "ambiguous");
    assert_eq!(dispatch.candidates.len(), 1, "{dispatch:#?}");
    assert_eq!(
        s.symbol(&dispatch.candidates[0]).unwrap().qualified_name,
        "Worker::Run"
    );
}
#[test]
fn rust_cfg_and_macro_calls_never_become_unconditional_exact_calls() {
    let d = setup(&[
        (
            "src/lib.rs",
            "#[cfg(feature=\"optional\")] mod conditional; #[cfg(feature=\"optional\")] fn gated(){} fn same(){} fn entry(){gated();conditional::check();same!();}",
        ),
        ("src/conditional.rs", "pub fn check(){}"),
    ]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    assert_eq!(calls(&s, "gated")[0].resolution, "inferred");
    assert_eq!(calls(&s, "check")[0].resolution, "inferred");
    assert!(
        !s.files
            .iter()
            .flat_map(|f| &f.references)
            .any(|r| r.name == "same")
    );
}
#[test]
fn configured_root_is_selected_once_without_git_reexpansion() {
    let d = setup(&[
        ("outer.rs", "fn outside(){}"),
        ("selected/inner.rs", "fn inside(){}"),
    ]);
    let mut config = Config::default();
    config.repository.root = "selected".into();
    let first = index(d.path(), &config, &Options::default()).unwrap();
    assert_eq!(first.snapshot.files.len(), 1);
    assert_eq!(first.snapshot.files[0].path, "inner.rs");
    let again =
        kortex_indexer::index_at_root(&d.path().join("selected"), &config, &Options::default())
            .unwrap();
    assert_eq!(again.reused, 1);
    assert_eq!(first.snapshot.files, again.snapshot.files);
    assert!(!d.path().join(".kortex").exists());
}
#[test]
fn configured_shared_cache_child() {
    let Some(root) = std::env::var_os("KORTEX_INDEXER_CACHE_CHILD") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mut config = Config::default();
    config.index.storage = "user-cache".into();
    let first = index(&root, &config, &Options::default()).unwrap();
    let store = kortex_storage::Store::open_with_storage(&root, "user-cache", false).unwrap();
    assert_eq!(
        store.load().unwrap().generation_id,
        first.snapshot.generation_id
    );
    assert!(!root.join(".kortex").exists());
    let dry = index(
        &root,
        &config,
        &Options {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(dry.reused, 1);
    assert!(!dry.committed);
}
#[test]
fn configured_shared_cache_persists_without_repository_writes() {
    let root = setup(&[("a.rs", "fn entry(){}")]);
    let cache = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "configured_shared_cache_child", "--nocapture"])
        .env("KORTEX_INDEXER_CACHE_CHILD", root.path())
        .env("XDG_CACHE_HOME", cache.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!root.path().join(".kortex").exists());
}
#[test]
fn reads_writes_inheritance_and_implementation_are_explicit_edges() {
    let d = setup(&[(
        "a.ts",
        "interface I {} class Base {} class Child extends Base implements I {} let value=1; function entry(){value=2;return value;}",
    )]);
    let s = index(d.path(), &Config::default(), &Options::default())
        .unwrap()
        .snapshot;
    for kind in ["READS", "WRITES", "INHERITS", "IMPLEMENTS", "CONTAINS"] {
        assert!(
            s.edges
                .iter()
                .any(|e| e.kind == kind && e.resolution == "exact"),
            "{kind}: {:#?}",
            s.edges
        );
    }
}
