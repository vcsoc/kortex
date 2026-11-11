//! Regression assertions for false-positive dependencies.
use kortex_config::Config;
use kortex_indexer::{Options, index};
use std::fs;
#[test]
fn unrelated_names_local_imports_shadowing_and_exports() {
    for (main, source, other, dependency, exact, unresolved) in [
        (
            "main.py",
            "class A:\n    def check(): pass\n    def entry(): check()\n",
            "other.py",
            "def unused(): pass\n",
            0,
            1,
        ),
        (
            "main.go",
            "package app\nfunc check(){}\nfunc entry(){check := func(){}; check()}",
            "other.go",
            "package app\nfunc unused(){}",
            0,
            1,
        ),
        (
            "main.go",
            "package app\nfunc entry(){check()}",
            "other.go",
            "package different\nfunc check(){}",
            0,
            1,
        ),
        (
            "main.py",
            "def f():\n    from auth import check\n    check()\ndef g(): check()\n",
            "auth.py",
            "def check(): pass\n",
            1,
            1,
        ),
        (
            "main.js",
            "import {check} from './auth.js'; function entry(check){check();}",
            "auth.js",
            "export function check(){}",
            0,
            1,
        ),
        (
            "main.ts",
            "import {check} from './auth'; function entry(check: () => void){check();}",
            "auth.ts",
            "export function check(){}",
            0,
            1,
        ),
        (
            "main.ts",
            "import {check} from './auth'; function entry(){check();}",
            "auth.ts",
            "function check(){}",
            0,
            1,
        ),
        (
            "main.py",
            "def entry(): check()\n",
            "auth.py",
            "def check(): pass\n",
            0,
            1,
        ),
        (
            "main.py",
            "import auth\ndef entry(auth): auth.check()\n",
            "auth.py",
            "def check(): pass\n",
            0,
            1,
        ),
        (
            "main.rs",
            "struct Foo; impl Foo { fn check(){} } fn entry(){check();}",
            "other.rs",
            "fn check(){}",
            0,
            1,
        ),
        (
            "lib.rs",
            "mod auth; fn entry(){auth::check();}",
            "auth.rs",
            "pub fn check(){}",
            1,
            0,
        ),
        (
            "lib.rs",
            "use crate::auth; fn entry(){auth::check();}",
            "auth.rs",
            "pub fn check(){}",
            1,
            0,
        ),
        (
            "main.go",
            "package app\nfunc entry(){check()}",
            "auth.go",
            "package app\nfunc check(){}",
            1,
            0,
        ),
    ] {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(main), source).unwrap();
        fs::write(d.path().join(other), dependency).unwrap();
        let r = index(d.path(), &Config::default(), &Options::default()).unwrap();
        let calls: Vec<_> = r
            .snapshot
            .edges
            .iter()
            .filter(|e| e.kind == "CALLS")
            .collect();
        assert_eq!(
            calls.iter().filter(|e| e.resolution == "exact").count(),
            exact,
            "{main}: {source}: {calls:?}"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|e| e.resolution == "unresolved")
                .count(),
            unresolved,
            "{main}: {source}: {calls:?}"
        );
    }
}
#[test]
fn reader_pins_graph_and_fts_generation() {
    let d = tempfile::tempdir().unwrap();
    let c = Config::default();
    fs::write(d.path().join("a.py"), "def original(): pass\n").unwrap();
    let first = index(d.path(), &c, &Options::default()).unwrap();
    let reader = kortex_storage::Store::open(d.path(), false).unwrap();
    let snap = reader.load().unwrap();
    fs::write(d.path().join("a.py"), "def replacement(): pass\n").unwrap();
    let second = index(d.path(), &c, &Options::default()).unwrap();
    assert_ne!(first.snapshot.generation_id, second.snapshot.generation_id);
    assert_eq!(reader.load().unwrap().generation_id, snap.generation_id);
    assert!(
        !reader
            .keyword(&snap.generation_id, "original", 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        reader
            .keyword(&snap.generation_id, "replacement", 10)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn rename_delete_full_equivalence() {
    let d = tempfile::tempdir().unwrap();
    let c = Config::default();
    fs::write(d.path().join("a.py"), "def entry(): pass\n").unwrap();
    fs::write(d.path().join("b.py"), "def gone(): pass\n").unwrap();
    index(d.path(), &c, &Options::default()).unwrap();
    fs::rename(d.path().join("a.py"), d.path().join("renamed.py")).unwrap();
    fs::remove_file(d.path().join("b.py")).unwrap();
    let a = index(d.path(), &c, &Options::default()).unwrap();
    let b = index(
        d.path(),
        &c,
        &Options {
            full: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(a.snapshot.files, b.snapshot.files);
    assert_eq!(a.snapshot.edges, b.snapshot.edges);
    assert_eq!(b.snapshot.files.len(), 1);
}
