//! Discovery -> real parser -> resolver -> SQLite -> keyword query, with labeled expectations.
//! Fixture source is data only; no compiler, interpreter or repository build is executed.
use kortex_config::Config;
use kortex_core::{Edge, Snapshot};
use kortex_indexer::{Options, index_at_root};
use kortex_storage::Store;
use std::{collections::BTreeMap, fs, path::Path};

struct Fixture {
    language: &'static str,
    library: &'static str,
    main: &'static str,
    source: &'static str,
    caller: &'static str,
    unrelated: &'static str,
    unrelated_source: &'static str,
}
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            language: "java",
            library: "lib/Auth.java",
            main: "Main.java",
            source: "package lib; public class Auth {public static void check(){} public static void overloaded(int x){} public static void overloaded(String x){}}",
            caller: "import lib.Auth; class Main {static void entry(){Auth.check();Auth.overloaded(1);missing();}}",
            unrelated: "Other.java",
            unrelated_source: "package unrelated; class Other {public static void missing(){}}",
        },
        Fixture {
            language: "csharp",
            library: "Auth.cs",
            main: "Main.cs",
            source: "namespace Lib {public class Auth {public static void check(){} public static void overloaded(int x){} public static void overloaded(string x){}}}",
            caller: "using Lib; class Main {static void entry(){Auth.check();Auth.overloaded(1);missing();}}",
            unrelated: "Other.cs",
            unrelated_source: "namespace Unrelated {class Other {public static void missing(){}}}",
        },
        Fixture {
            language: "c",
            library: "auth.h",
            main: "main.c",
            source: "int check(void){return 1;} int overloaded(int x); int overloaded(double x);",
            caller: "#include \"auth.h\"\nvoid entry(void){check();overloaded(1);missing();}",
            unrelated: "other.c",
            unrelated_source: "void missing(){}",
        },
        // .hpp is discovered as C++, .h as C. This deliberately exercises the real mapping.
        Fixture {
            language: "cpp",
            library: "auth.hpp",
            main: "main.cpp",
            source: "namespace Lib {int check(){return 1;} int overloaded(int x){return x;} int overloaded(double x){return 1;}}",
            caller: "#include \"auth.hpp\"\nvoid entry(){Lib::check();Lib::overloaded(1);missing();}",
            unrelated: "other.cpp",
            unrelated_source: "void missing(){}",
        },
        Fixture {
            language: "ruby",
            library: "auth.rb",
            main: "main.rb",
            source: "module Lib\n class Auth\n def self.check;end\n def self.overloaded(x);end\n def self.overloaded(x,y);end\n end\nend\n",
            caller: "require_relative 'auth'\ndef entry\n Lib::Auth.check\n Lib::Auth.overloaded(1)\n missing()\nend\n",
            unrelated: "other.rb",
            unrelated_source: "module Unrelated;def self.missing;end;end",
        },
        Fixture {
            language: "php",
            library: "auth.php",
            main: "main.php",
            source: "<?php namespace Lib; class Auth {public static function check(){} public static function overloaded($x){} public static function overloaded($x,$y){}}",
            caller: "<?php namespace App; use Lib\\Auth as Gate; function entry(){Gate::check();Gate::overloaded(1);missing();}",
            unrelated: "other.php",
            unrelated_source: "<?php namespace Unrelated; function missing(){}",
        },
    ]
}
fn put(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}
fn calls<'a>(s: &'a Snapshot, path: &str, name: &str) -> Vec<&'a Edge> {
    let f = s.files.iter().find(|f| f.path == path).unwrap();
    f.references
        .iter()
        .filter(|r| r.kind == "CALLS" && r.name == name)
        .map(|r| {
            s.edges
                .iter()
                .find(|e| e.kind == r.kind && e.source_id == r.source_id && e.source_span == r.span)
                .unwrap()
        })
        .collect()
}
fn readback(root: &Path, s: &Snapshot) {
    let store = Store::open(root, false).unwrap();
    let persisted = store.load().unwrap();
    assert_eq!(persisted.files, s.files);
    assert_eq!(persisted.edges, s.edges);
    assert_eq!(persisted.generation_id, s.generation_id);
    let hits = store.keyword(&s.generation_id, "check", 100).unwrap();
    let check_ids: Vec<_> = s
        .symbols()
        .filter(|s| s.name == "check")
        .map(|s| &s.symbol_id)
        .collect();
    for id in check_ids {
        assert!(
            hits.iter().any(|(hit, _)| hit == id),
            "persisted FTS omitted check declaration"
        );
    }
}
fn equivalent(root: &Path, incremental: &Snapshot) {
    readback(root, incremental);
    let full = index_at_root(
        root,
        &Config::default(),
        &Options {
            full: true,
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(incremental.files, full.snapshot.files);
    assert_eq!(incremental.edges, full.snapshot.edges);
    readback(root, &full.snapshot);
}
fn assert_targets(s: &Snapshot, f: &Fixture, library: Option<&str>) {
    for (name, expected, count) in [
        ("check", "exact", 1),
        ("overloaded", "ambiguous", 2),
        ("missing", "unresolved", 0),
    ] {
        let calls = calls(s, f.main, name);
        assert_eq!(calls.len(), 1, "{}/{name}", f.language);
        let e = calls[0];
        if library.is_none() || name == "missing" {
            assert_eq!(e.resolution, "unresolved", "{}/{name}: {e:#?}", f.language);
            assert!(e.target_id.is_none() && e.candidates.is_empty());
        } else {
            assert_eq!(e.resolution, expected, "{}/{name}: {e:#?}", f.language);
            let ids: Vec<_> = e.target_id.iter().chain(&e.candidates).collect();
            assert_eq!(ids.len(), count);
            for id in ids {
                let target = s.symbol(id).unwrap();
                assert_eq!(target.name, name);
                assert_eq!(Some(target.file_path.as_str()), library);
            }
        }
    }
}
#[test]
fn all_six_p1_discovery_persistence_query_and_incremental_full_equivalence() {
    for f in fixtures() {
        let d = tempfile::tempdir().unwrap();
        for (p, source) in [
            (f.library, f.source),
            (f.main, f.caller),
            (f.unrelated, f.unrelated_source),
        ] {
            put(d.path(), p, source);
        }
        let config = Config::default();
        let first = index_at_root(d.path(), &config, &Options::default()).unwrap();
        assert!(first.committed);
        assert_eq!(first.parsed, 3, "{} discovery", f.language);
        assert!(
            first
                .snapshot
                .files
                .iter()
                .all(|p| p.language == f.language)
        );
        assert_targets(&first.snapshot, &f, Some(f.library));
        equivalent(d.path(), &first.snapshot);
        let unchanged = index_at_root(d.path(), &config, &Options::default()).unwrap();
        assert_eq!((unchanged.parsed, unchanged.reused), (0, 3));
        assert_eq!(unchanged.snapshot.edges, first.snapshot.edges);
        // Semantic edit: rename check and its call, not just a comment-only cache exercise.
        put(d.path(), f.library, &f.source.replace("check", "verify"));
        put(d.path(), f.main, &f.caller.replace("check", "verify"));
        let edited = index_at_root(d.path(), &config, &Options::default()).unwrap();
        assert_eq!((edited.parsed, edited.reused), (2, 1));
        assert_eq!(
            calls(&edited.snapshot, f.main, "verify")[0].resolution,
            "exact"
        );
        assert!(edited.snapshot.symbols().all(|s| s.name != "check"));
        equivalent(d.path(), &edited.snapshot);
        // Restore names while relocating the actual provider file; path-based imports change too.
        let renamed = format!(
            "moved/{}",
            Path::new(f.library).file_name().unwrap().to_str().unwrap()
        );
        fs::create_dir_all(d.path().join("moved")).unwrap();
        fs::rename(d.path().join(f.library), d.path().join(&renamed)).unwrap();
        put(d.path(), &renamed, f.source);
        let caller = if matches!(f.language, "c" | "cpp") {
            f.caller.replace(f.library, &renamed)
        } else if f.language == "ruby" {
            f.caller.replace("'auth'", "'moved/auth'")
        } else {
            f.caller.into()
        };
        put(d.path(), f.main, &caller);
        let moved = index_at_root(d.path(), &config, &Options::default()).unwrap();
        assert_eq!((moved.parsed, moved.reused), (2, 1));
        assert!(!moved.snapshot.files.iter().any(|p| p.path == f.library));
        assert_targets(&moved.snapshot, &f, Some(&renamed));
        equivalent(d.path(), &moved.snapshot);
        fs::remove_file(d.path().join(&renamed)).unwrap();
        let deleted = index_at_root(d.path(), &config, &Options::default()).unwrap();
        assert_eq!((deleted.parsed, deleted.reused), (0, 2));
        assert_targets(&deleted.snapshot, &f, None);
        equivalent(d.path(), &deleted.snapshot);
        let store = Store::open(d.path(), false).unwrap();
        let hits = store
            .keyword(&deleted.snapshot.generation_id, "check", 100)
            .unwrap();
        assert!(hits.iter().all(|(id, _)| {
            deleted
                .snapshot
                .symbol(id)
                .is_some_and(|s| s.name != "check")
        }));
    }
}

/// Explicitly opt-in: the caller supplies the trusted, freshly built CLI, never fixture code.
#[test]
#[ignore = "requires KORTEX_TEST_CLI pointing to the trusted current executable"]
fn actual_cli_discovery_query_and_context_for_all_six_p1_languages() {
    let binary = std::env::var_os("KORTEX_TEST_CLI").expect("set KORTEX_TEST_CLI");
    let binary = Path::new(&binary).canonicalize().unwrap();
    let base = tempfile::tempdir().unwrap();
    let executable = base.path().join(if cfg!(windows) {
        "kortex.exe"
    } else {
        "kortex"
    });
    fs::copy(binary, &executable).unwrap();
    let home = base.path().join("home");
    fs::create_dir_all(&home).unwrap();
    for f in fixtures() {
        let root = base.path().join(f.language);
        fs::create_dir(&root).unwrap();
        for (path, source) in [
            (f.library, f.source),
            (f.main, f.caller),
            (f.unrelated, f.unrelated_source),
        ] {
            put(&root, path, source);
        }
        put(
            &root,
            ".kortex.toml",
            "schema_version = 1\n[index]\nstorage = \"repository\"\n[search]\nembeddings = \"disabled\"\n[http]\nenabled = false\n",
        );
        let invoke = |words: &[&str]| -> serde_json::Value {
            let mut command = std::process::Command::new(&executable);
            command
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("XDG_CONFIG_HOME", home.join("config"))
                .env("XDG_CACHE_HOME", home.join("cache"))
                .env(
                    "KORTEX_REGISTRY",
                    base.path().join("private-registry/registry.db"),
                )
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env(
                    "GIT_CONFIG_SYSTEM",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env("GIT_TERMINAL_PROMPT", "0")
                .current_dir(&root)
                .arg("--repo")
                .arg(&root)
                .arg("--config")
                .arg(root.join(".kortex.toml"))
                .args(words)
                .arg("--json");
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{} {:?}: {} {}",
                f.language,
                words,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            serde_json::from_slice(&output.stdout).unwrap()
        };
        invoke(&["index", ".", "--verify"]);
        let s = Store::open(&root, false).unwrap().load().unwrap();
        assert_eq!(s.files.len(), 3);
        assert_targets(&s, &f, Some(f.library));
        let query = invoke(&["query", "check"]);
        let items = query["data"]["items"].as_array().unwrap();
        assert!(
            items.iter().any(|item| item["symbol"]["name"] == "check"
                && item["symbol"]["file_path"] == f.library
                && item["symbol"]["language"] == f.language),
            "{}: {query}",
            f.language
        );
        let context = invoke(&["context", &format!("{}:check", f.library)]);
        let callers = context["data"]["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["resolution"], "exact");
        let entry = s.symbol(callers[0]["source_id"].as_str().unwrap()).unwrap();
        assert_eq!(
            (entry.file_path.as_str(), entry.name.as_str()),
            (f.main, "entry")
        );
        println!(
            "P1_CLI language={} discovered=3 persisted=true query_check=true context_exact_callers=1",
            f.language
        );
    }
}

#[test]
fn labeled_p1_callable_extraction_and_resolution_report() {
    for f in fixtures() {
        let d = tempfile::tempdir().unwrap();
        for (p, source) in [
            (f.library, f.source),
            (f.main, f.caller),
            (f.unrelated, f.unrelated_source),
        ] {
            put(d.path(), p, source);
        }
        let s = index_at_root(d.path(), &Config::default(), &Options::default())
            .unwrap()
            .snapshot;
        // Entire callable-declaration and CALLS population is labeled for these three files.
        // Other symbol/reference kinds are deliberately outside this metric's denominator.
        let expected = BTreeMap::from([
            ((f.library, "check"), 1),
            ((f.library, "overloaded"), 2),
            ((f.main, "entry"), 1),
            ((f.unrelated, "missing"), 1),
        ]);
        let mut actual = BTreeMap::new();
        for s in s
            .symbols()
            .filter(|s| matches!(s.kind.as_str(), "function" | "method" | "constructor"))
        {
            *actual
                .entry((s.file_path.as_str(), s.name.as_str()))
                .or_insert(0) += 1;
        }
        let tp: usize = expected
            .iter()
            .map(|(key, n)| (*n).min(*actual.get(key).unwrap_or(&0)))
            .sum();
        let predicted: usize = actual.values().sum();
        let labeled: usize = expected.values().sum();
        let fp = predicted - tp;
        let missed = labeled - tp;
        assert_eq!(actual, expected, "{} extraction", f.language);
        let expected_calls = BTreeMap::from([("check", 1), ("overloaded", 1), ("missing", 1)]);
        let mut extracted_calls = BTreeMap::new();
        for r in s
            .files
            .iter()
            .flat_map(|p| &p.references)
            .filter(|r| r.kind == "CALLS")
        {
            *extracted_calls.entry(r.name.as_str()).or_insert(0) += 1;
        }
        assert_eq!(
            extracted_calls, expected_calls,
            "{} call extraction",
            f.language
        );
        assert_targets(&s, &f, Some(f.library));
        println!(
            "P1_LABELED language={} files=3 callable_tp={tp} callable_fp={fp} callable_fn={missed} extraction_precision={:.3} extraction_recall={:.3} call_sites=3 exact_target_tp=1 exact_target_fp=0 exact_target_fn=0 candidate_sets_correct=1 negative_unresolved=1 unknown_runtime_targets=0 scope=handwritten_callable_fixture_only",
            f.language,
            tp as f64 / predicted as f64,
            tp as f64 / labeled as f64
        );
    }
}
