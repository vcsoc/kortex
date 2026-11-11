//! Labeled real-grammar P1 fixtures. These never execute indexed sources.
use kortex_core::{Edge, Snapshot};
use kortex_parse::{LanguageProvider, Provider, parse};
fn snapshot(files: &[(&str, &str, &str)]) -> Snapshot {
    Snapshot {
        repo_id: "fixture".into(),
        files: files
            .iter()
            .map(|(path, lang, source)| {
                let file = parse("fixture", path, lang, source).unwrap();
                for symbol in file.symbols.iter().filter(|s| s.kind != "file") {
                    assert!(
                        source[symbol.span.start_byte..symbol.span.end_byte].contains(&symbol.name),
                        "{lang}: {symbol:#?}"
                    );
                }
                for r in &file.references {
                    assert!(!source[r.span.start_byte..r.span.end_byte].is_empty());
                }
                file
            })
            .collect(),
        ..Default::default()
    }
}
fn named<'a>(s: &'a Snapshot, name: &str) -> Vec<&'a Edge> {
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
fn verify(mut s: Snapshot, lang: &str) {
    for f in &s.files {
        assert!(
            !f.diagnostics.iter().any(|d| d.contains("syntax errors")),
            "{lang}: {:#?}",
            s.files
        );
    }
    let unrelated = match lang {
        "java" => "package unrelated; class Other {public static void missing(){}}",
        "csharp" => "namespace Unrelated {class Other {public static void missing(){}}}",
        "c" | "cpp" => "void missing(){}",
        "ruby" => "module Unrelated; def self.missing;end;end",
        "php" => "<?php namespace Unrelated; function missing(){}",
        _ => unreachable!(),
    };
    s.files
        .push(parse("fixture", "unrelated", lang, unrelated).unwrap());
    s.edges = kortex_languages::resolve(&s, None);
    for e in named(&s, "check")
        .into_iter()
        .filter(|e| e.resolution != "exact")
    {
        assert_eq!(e.resolution, "unresolved", "{lang}: {e:#?}");
        assert!(e.target_id.is_none() && e.candidates.is_empty());
    }
    assert_eq!(
        named(&s, "check")
            .iter()
            .filter(|e| e.resolution == "exact")
            .count(),
        1,
        "{lang}: {:#?}",
        s
    );
    let overloaded = named(&s, "overloaded");
    assert_eq!(overloaded.len(), 1, "{lang}: {:#?}", s.files);
    assert_eq!(overloaded[0].resolution, "ambiguous", "{lang}: {:#?}", s);
    assert!(overloaded[0].target_id.is_none());
    assert_eq!(overloaded[0].candidates.len(), 2);
    assert!(
        named(&s, "missing")
            .iter()
            .all(|e| e.resolution == "unresolved")
    );
    assert_eq!(named(&s, "missing").len(), 1, "{lang}: {:#?}", s.files);
    for f in &s.files {
        for symbol in &f.symbols {
            assert!(symbol.span.start_line >= 1 && symbol.span.start_col >= 1);
        }
    }
    assert!(
        s.edges
            .iter()
            .any(|e| e.kind == "IMPORTS" && e.resolution == "exact"),
        "{lang}: {:#?}",
        s.edges
    );
    // Cached immutable facts serialize identically and re-resolve deterministically.
    let roundtrip: Snapshot = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(kortex_languages::resolve(&roundtrip, None), s.edges);
    let d = tempfile::tempdir().unwrap();
    s.root = kortex_core::public_path(d.path());
    s.generation_id = "p1-fixture".into();
    let mut store = kortex_storage::Store::open(d.path(), true).unwrap();
    let lock = store.writer_lock().unwrap();
    store.commit(&s, &lock).unwrap();
    let persisted = store.load().unwrap();
    assert_eq!(persisted.files, s.files);
    assert_eq!(persisted.edges, s.edges);
    assert!(
        !store
            .keyword(&s.generation_id, "check", 20)
            .unwrap()
            .is_empty(),
        "{lang}"
    );
}
#[test]
fn java_packages_imports_overloads_and_unknown_receiver() {
    let s = snapshot(&[
        (
            "lib/Auth.java",
            "java",
            "package lib; public class Auth {public static void check(){} public static void overloaded(int x){} public static void overloaded(String x){} private static void hidden(){}}",
        ),
        (
            "Main.java",
            "java",
            "import lib.Auth; class Main {static void entry(){Auth.check();Auth.overloaded(1);missing();Object value=null;value.check();}}",
        ),
    ]);
    verify(s, "java");
}
#[test]
fn csharp_namespaces_using_partial_classes_and_overloads() {
    let s = snapshot(&[
        (
            "Auth.cs",
            "csharp",
            "namespace Lib {public partial class Auth {public static void check(){} public static void overloaded(int x){}}}",
        ),
        (
            "More.cs",
            "csharp",
            "namespace Lib {public partial class Auth {public static void overloaded(string x){}}}",
        ),
        (
            "Main.cs",
            "csharp",
            "using Lib; class Main {static void entry(){Auth.check();Auth.overloaded(1);missing();}}",
        ),
    ]);
    verify(s, "csharp");
}
#[test]
fn c_local_includes_declarations_and_conflicting_candidate_sets() {
    let s = snapshot(&[
        (
            "auth.h",
            "c",
            "int check(void){return 1;} int overloaded(int x); int overloaded(double x);",
        ),
        (
            "main.c",
            "c",
            "#include \"auth.h\"\nvoid entry(void){check();overloaded(1);missing();}",
        ),
    ]);
    verify(s, "c");
}
#[test]
fn cpp_include_namespace_and_overload_candidates() {
    let s = snapshot(&[
        (
            "auth.hpp",
            "cpp",
            "namespace Lib {int check(){return 1;} int overloaded(int x){return x;} int overloaded(double x){return 1;}}",
        ),
        (
            "main.cpp",
            "cpp",
            "#include \"auth.hpp\"\nvoid entry(){Lib::check();Lib::overloaded(1);missing();}",
        ),
    ]);
    verify(s, "cpp");
}
#[test]
fn ruby_require_relative_modules_and_redefinition_candidates() {
    let s = snapshot(&[
        (
            "auth.rb",
            "ruby",
            "module Lib\n class Auth\n def self.check; end\n def self.overloaded(x);end\n def self.overloaded(x,y);end\n end\nend\n",
        ),
        (
            "main.rb",
            "ruby",
            "require_relative 'auth'\ndef entry\n Lib::Auth.check\n Lib::Auth.overloaded(1)\n missing()\n value.check\nend\n",
        ),
    ]);
    verify(s, "ruby");
}
#[test]
fn php_namespaces_use_alias_static_calls_and_conflicting_declarations() {
    let s = snapshot(&[
        (
            "auth.php",
            "php",
            "<?php namespace Lib; class Auth {public static function check(){} public static function overloaded($x){} public static function overloaded($x,$y){}}",
        ),
        (
            "main.php",
            "php",
            "<?php namespace App; use Lib\\Auth as Gate; function entry(){Gate::check();Gate::overloaded(1);missing();$value->check();}",
        ),
    ]);
    verify(s, "php");
}
#[test]
fn explicit_inheritance_implementation_and_override_edges() {
    for (lang, path, source) in [
        (
            "java",
            "Types.java",
            "interface I {} class Base {public void run(){}} class Child extends Base implements I {@Override public void run(){}}",
        ),
        (
            "csharp",
            "Types.cs",
            "interface I {} class Base {public virtual void run(){}} class Child : Base,I {public override void run(){}}",
        ),
    ] {
        let mut s = snapshot(&[(path, lang, source)]);
        s.edges = kortex_languages::resolve(&s, None);
        for kind in ["INHERITS", "IMPLEMENTS", "OVERRIDES"] {
            assert!(
                s.edges
                    .iter()
                    .any(|e| e.kind == kind && e.resolution == "exact"),
                "{lang}/{kind}: {s:#?}"
            );
        }
    }
}
#[test]
fn p1_values_types_and_parameter_shadowing_are_real_facts() {
    for (lang, path, source) in [
        (
            "java",
            "Types.java",
            "class Model {} class Main {static void check(){} static Model entry(Model model){Model value=model;check();return value;}}",
        ),
        (
            "csharp",
            "Types.cs",
            "class Model {} class Main {static void check(){} static Model entry(Model model){Model value=model;check();return value;}}",
        ),
        (
            "c",
            "types.c",
            "struct Model {int value;}; int value; void entry(){value=2;}",
        ),
        (
            "cpp",
            "types.cpp",
            "struct Model {int value;}; int value; void entry(){value=2;}",
        ),
        ("ruby", "types.rb", "def entry(value);local=value;local;end"),
        (
            "php",
            "types.php",
            "<?php function entry($value){$local=$value;return $local;}",
        ),
    ] {
        let mut s = snapshot(&[(path, lang, source)]);
        s.edges = kortex_languages::resolve(&s, None);
        assert!(s.symbols().any(|s| s.kind == "variable"), "{lang}: {s:#?}");
        let kind = if matches!(lang, "c" | "cpp") {
            "WRITES"
        } else {
            "REFERENCES"
        };
        assert!(
            s.edges
                .iter()
                .any(|e| e.kind == kind && e.resolution == "exact"),
            "{lang}: {s:#?}"
        );
    }
}
#[test]
fn file_scoped_csharp_and_multiple_php_namespaces_keep_real_owners() {
    for (lang, path, source) in [
        (
            "csharp",
            "Names.cs",
            "namespace Lib; class Auth {public static void check(){}} class Main {static void entry(){Auth.check();}}",
        ),
        (
            "php",
            "names.php",
            "<?php namespace One; function check(){} function entry(){check();} namespace Two; function check(){} function entry(){check();}",
        ),
    ] {
        let mut s = snapshot(&[(path, lang, source)]);
        s.edges = kortex_languages::resolve(&s, None);
        assert!(s.symbols().any(|s| s.kind == "namespace"));
        for e in named(&s, "check") {
            assert_eq!(e.resolution, "exact", "{lang}: {s:#?}");
            let source = s.symbol(&e.source_id).unwrap();
            let target = s.symbol(e.target_id.as_deref().unwrap()).unwrap();
            assert_eq!(
                source.qualified_name.split("::").next(),
                target.qualified_name.split("::").next()
            );
        }
    }
}
#[test]
fn c_cpp_macros_and_conditional_declarations_remain_uncertain() {
    for lang in ["c", "cpp"] {
        let mut s = snapshot(&[
            ("macros.h", lang, "#define check() unknown()\n"),
            (
                "main",
                lang,
                "#include \"macros.h\"\nvoid check(){}\n#ifdef OPTIONAL\nvoid gated(){}\n#endif\nvoid entry(){check();gated();}",
            ),
        ]);
        s.edges = kortex_languages::resolve(&s, None);
        assert_eq!(
            named(&s, "check")[0].resolution,
            "unresolved",
            "{lang}: {s:#?}"
        );
        assert_eq!(
            named(&s, "gated")[0].resolution,
            "inferred",
            "{lang}: {s:#?}"
        );
    }
}
#[test]
fn every_extended_provider_handles_malformed_negative_and_ambiguity_sources() {
    for (lang, path, broken) in [
        ("java", "broken.java", "class Broken { void run( {"),
        ("csharp", "broken.cs", "class Broken { void Run( {"),
        ("c", "broken.c", "void broken( {"),
        ("cpp", "broken.cpp", "class Broken { void run( {"),
        ("ruby", "broken.rb", "def broken(\n"),
        ("php", "broken.php", "<?php function broken( {"),
    ] {
        let provider = Provider { language: lang };
        assert_eq!(provider.id(), lang);
        assert!(!provider.extensions().is_empty());
        let f = provider.parse("fixture", path, broken).unwrap();
        assert!(!f.diagnostics.is_empty(), "{lang}: {f:#?}");
        assert!(!f.symbols.is_empty());
    }
}
