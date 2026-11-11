//! Adversarial repeated-lookup cases: indexes may accelerate evidence, never broaden visibility.
use kortex_core::{Edge, Snapshot};
fn facts(files: &[(&str, &str, &str)]) -> Snapshot {
    Snapshot {
        repo_id: "lookup-fixture".into(),
        files: files
            .iter()
            .map(|(p, l, s)| kortex_parse::parse("lookup-fixture", p, l, s).unwrap())
            .collect(),
        ..Default::default()
    }
}
fn calls<'a>(s: &'a Snapshot, path: &str, name: &str) -> Vec<&'a Edge> {
    let file = s.files.iter().find(|f| f.path == path).unwrap();
    file.references
        .iter()
        .filter(|r| r.kind == "CALLS" && r.name == name)
        .map(|r| {
            s.edges
                .iter()
                .find(|e| {
                    e.source_id == r.source_id && e.source_span == r.span && e.kind == "CALLS"
                })
                .unwrap()
        })
        .collect()
}
#[test]
fn repeated_import_candidates_never_bypass_lexical_scope_or_mutation() {
    let source = format!(
        "from lib import check\ndef entry():\n{}def shadow(check):\n    check()\ndef unrelated():\n    missing()\n",
        "    check()\n".repeat(256)
    );
    let mut s = facts(&[
        ("main.py", "python", &source),
        ("lib.py", "python", "def check():pass"),
        ("other.py", "python", "def missing():pass"),
    ]);
    s.edges = kortex_languages::resolve(&s, None);
    let check = calls(&s, "main.py", "check");
    assert_eq!(check.len(), 257);
    assert_eq!(
        check.iter().filter(|e| e.resolution == "exact").count(),
        256
    );
    assert_eq!(
        check
            .iter()
            .filter(|e| e.resolution == "unresolved")
            .count(),
        1
    );
    assert_eq!(calls(&s, "main.py", "missing")[0].resolution, "unresolved");
    // A new pass cannot reuse a now-missing provider or any candidates from the previous snapshot.
    s.files.retain(|f| f.path != "lib.py");
    s.edges = kortex_languages::resolve(&s, None);
    assert!(
        calls(&s, "main.py", "check")
            .iter()
            .all(|e| e.resolution == "unresolved")
    );
}
#[test]
fn c_cpp_transitive_include_cycles_and_cross_grammar_headers_are_conservative() {
    let mut s = facts(&[
        (
            "main.cpp",
            "cpp",
            "#include \"one.h\"\nvoid entry(){check();missing();}",
        ),
        ("one.h", "c", "#include \"two.h\"\nint check(void);"),
        (
            "two.h",
            "c",
            "#include \"one.h\"\nint check(void){return 1;}",
        ),
        ("private.cpp", "cpp", "void missing(){}"),
    ]);
    s.edges = kortex_languages::resolve(&s, None);
    let check = calls(&s, "main.cpp", "check")[0];
    assert_eq!(check.resolution, "exact", "{s:#?}");
    assert_eq!(
        s.symbol(check.target_id.as_deref().unwrap())
            .unwrap()
            .file_path,
        "two.h"
    );
    assert_eq!(calls(&s, "main.cpp", "missing")[0].resolution, "unresolved");
}
#[test]
fn rust_reexport_cycles_keep_private_and_missing_paths_unresolved() {
    let mut s = facts(&[
        (
            "src/lib.rs",
            "rust",
            "mod a;mod b;use crate::a::{check,hidden,missing};fn entry(){check();hidden();missing();}",
        ),
        (
            "src/a.rs",
            "rust",
            "pub use crate::b::{check,hidden,missing};",
        ),
        (
            "src/b.rs",
            "rust",
            "pub use crate::a::missing;pub fn check(){} fn hidden(){}",
        ),
    ]);
    s.edges = kortex_languages::resolve(&s, None);
    assert_eq!(calls(&s, "src/lib.rs", "check")[0].resolution, "exact");
    for name in ["hidden", "missing"] {
        let e = calls(&s, "src/lib.rs", name)[0];
        assert_eq!(e.resolution, "unresolved", "{s:#?}");
        assert!(e.target_id.is_none() && e.candidates.is_empty());
    }
}
#[test]
fn context_fast_path_and_transformed_aliases_have_identical_relative_import_edges() {
    let s = facts(&[
        (
            "main.ts",
            "typescript",
            "import {check} from './lib';function entry(){check();}",
        ),
        ("lib.ts", "typescript", "export function check(){}"),
    ]);
    let plain = kortex_languages::resolve(&s, None);
    assert_eq!(
        kortex_languages::resolve_with_context(&s, &Default::default()),
        plain
    );
    let context = kortex_languages::ResolutionContext::default()
        .with_tsconfig("{\"compilerOptions\":{\"paths\":{\"unused/*\":[\"elsewhere/*\"]}}}")
        .unwrap();
    assert_eq!(kortex_languages::resolve_with_context(&s, &context), plain);
}
