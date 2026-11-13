//! Admission tests use actual provider trees and source, never fabricated fact vectors.
use super::*;

#[test]
fn unchanged_deep_prefixes_keep_complete_pre_borrow_provider_outputs() {
    let name = format!("long_{}", "x".repeat(64 * 1024));
    let expr = format!("{}1{}", "(".repeat(96), ")".repeat(96));
    // Pinned from actual pre-borrow parses, including conservative diagnostics.
    // This is an exact-output regression, not an allocator/RSS measurement.
    for (language, path, source, expected) in [
        (
            "python",
            "deep.py",
            format!("def {name}():\n    return {expr}\n"),
            "92c0c06d75a09260a6174696e3091d2f276c9b86f8a81d0855e5928c83964d42",
        ),
        (
            "php",
            "deep.php",
            format!("<?php function {name}() {{ return {expr}; }}\n"),
            "93206ddf71d72f8734189d51e0aee98c9122d9f66a1f2672f5740c9593b2d236",
        ),
        (
            "cpp",
            "deep.cpp",
            format!("int {name}() {{ return {expr}; }}\n"),
            "90c957013c927ebe5e14e8214b822e210706800666097ce19d8644dd8a1caf93",
        ),
    ] {
        let file = parse_bounded_with_output_limits(
            "prefix-fixture",
            path,
            language,
            &source,
            &ast_limits(),
            &OutputLimits::default(),
        )
        .unwrap();
        assert!(file.symbols.iter().any(|symbol| symbol.name == name));
        assert_eq!(
            hash(kortex_json::to_vec(&file).unwrap()),
            expected,
            "{language}"
        );
    }
}

#[test]
fn broad_binding_pending_work_refuses_before_enqueue_without_changing_trusted_facts() {
    let source = format!(
        "[{}] = values\n",
        (0..4100)
            .map(|i| format!("v{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let trusted = parse("pending-fixture", "bindings.py", "python", &source).unwrap();
    let bytes = kortex_json::to_vec(&trusted).unwrap();
    assert_eq!(trusted.symbols.len(), 4096);
    assert_eq!(bytes.len(), 13_750_793);
    // Exact pre-admission-change compatibility output, including its diagnostic.
    assert_eq!(
        hash(&bytes),
        "41ed5ff6ec8466afe6e3f5b2f642232e6312c774e132cedf2f6f5a1857730b98"
    );
    let error = parse_bounded_with_output_limits(
        "pending-fixture",
        "bindings.py",
        "python",
        &source,
        &ast_limits(),
        &OutputLimits::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "RESOURCE_LIMIT");
    assert_eq!(error.message, "binding expansion limit reached");
    let retry = parse_bounded_with_output_limits(
        "pending-fixture",
        "bindings.py",
        "python",
        "[a,b] = values\n",
        &ast_limits(),
        &OutputLimits::default(),
    )
    .unwrap();
    assert!(retry.symbols.iter().any(|s| s.name == "a"));
    assert!(retry.symbols.iter().any(|s| s.name == "b"));
    assert!(retry.diagnostics.is_empty());
}

fn ast_limits() -> ParseLimits {
    ParseLimits {
        max_ast_nodes: 250_000,
        max_ast_depth: 192,
    }
}
fn fixtures() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "a.rs",
            "rust",
            "pub use crate::{one, two as alias};\ntrait Bound {}\nstruct Service;\n#[cfg(unix)]\nimpl Bound for Service {\n/// caf\u{e9} \"quoted\"\nfn run<T: Bound>(&self, a: T) { let x: Service = Service; x = Service; self.call(); }\n}",
        ),
        (
            "a.py",
            "python",
            "from auth import check as verify\nvalue = 0\n# caf\u{e9}\nclass Service(Base):\n    def run(self, arg: Payload):\n        global value\n        value = arg\n        verify(arg)\n",
        ),
        (
            "a.ts",
            "typescript",
            "import { check as verify } from './auth';\n/** caf\u{e9} */\nexport class Service<T> extends Base implements Contract {\n  item: Payload;\n  run(arg: Payload) { let x: Payload = arg; x = arg; verify(x); }\n}",
        ),
        (
            "a.js",
            "javascript",
            "const {check: verify, other} = require('./auth');\n/** caf\u{e9} */\nfunction run(arg) { let x = arg; x = other; verify(x); }\nexports.alias = run; module.exports = {run, again: run};",
        ),
        (
            "a.go",
            "go",
            "package app\nimport auth \"app/auth\"\ntype Payload struct { Value int }\ntype Contract interface { Run(a,b Payload) Payload }\ntype Service struct {}\n// caf\u{e9}\nfunc (s *Service) Run(a,b Payload) Payload { auth.Check(a); return b }",
        ),
        (
            "a.java",
            "java",
            "package app; import auth.Check;\n/** caf\u{e9} */\npublic class Service<T> extends Base implements Contract { Payload item; public void run(Payload arg) { arg.check(); Check.call(); } }",
        ),
        (
            "a.cs",
            "csharp",
            "using Auth; namespace App;\n/// caf\u{e9}\npublic class Service<T> : Base, IContract { public Payload Item; public void Run(Payload arg) { arg.Check(); } }",
        ),
        (
            "a.c",
            "c",
            "#include \"auth.h\"\n#define NAME one\n#ifdef NAME\n/* caf\u{e9} */\nint run(int arg) { return check(arg); }\n#endif\n#include <system.h>",
        ),
        (
            "a.cpp",
            "cpp",
            "#include \"auth.h\"\n#define NAME one\nnamespace App {\n/* caf\u{e9} */\nclass Service : public Base { public: int run(Payload arg) { return arg.check(); } }; }",
        ),
        (
            "a.rb",
            "ruby",
            "require_relative 'auth'\nrequire 'external'\n# caf\u{e9}\nclass Service < Base\n  def run(arg)\n    value = arg\n    arg.check(value)\n  end\nend",
        ),
        (
            "a.php",
            "php",
            "<?php namespace App; use Auth\\Check;\n/** caf\u{e9} */\nclass Service extends Base implements Contract { public function run(Payload $arg) { $value = $arg; $arg->check(); Check::call(); } }",
        ),
    ]
}
fn bytes(file: &ParsedFile) -> Vec<u8> {
    serde_json::to_vec(file).unwrap()
}
fn resource(error: Error) -> Error {
    assert_eq!((error.code, error.exit), ("RESOURCE_LIMIT", 9), "{error:?}");
    error
}
fn bounded(path: &str, lang: &str, source: &str, output: &OutputLimits) -> Result<ParsedFile> {
    parse_bounded_with_output_limits("r", path, lang, source, &ast_limits(), output)
}

#[test]
fn all_providers_exact_bytes_counts_and_trusted_api_equivalence() {
    for (path, lang, source) in fixtures() {
        let oracle = parse("r", path, lang, source).unwrap();
        let encoded = bytes(&oracle);
        assert!(oracle.symbols.len() > 1, "{lang}");
        assert!(!oracle.references.is_empty(), "{lang}");
        assert!(!oracle.imports.is_empty(), "{lang}");
        let exact = OutputLimits {
            max_symbols: oracle.symbols.len(),
            max_references: oracle.references.len(),
            max_imports: oracle.imports.len(),
            max_diagnostics: oracle.diagnostics.len(),
            max_bytes: encoded.len(),
        };
        assert_eq!(
            check_output_limits(&oracle, &exact).unwrap(),
            encoded.len(),
            "{lang}"
        );
        let admitted =
            bounded(path, lang, source, &exact).unwrap_or_else(|e| panic!("{lang}: {e:?}"));
        assert_eq!(bytes(&admitted), encoded, "exact boundary: {lang}");
        assert_eq!(
            bytes(&parse_bounded("r", path, lang, source, &ast_limits()).unwrap()),
            encoded,
            "AST oracle: {lang}"
        );
        assert_eq!(
            bytes(&bounded(path, lang, source, &OutputLimits::default()).unwrap()),
            encoded,
            "generous: {lang}"
        );
        assert_eq!(admitted.import_scopes.len(), admitted.imports.len());
        assert_eq!(admitted.import_spans.len(), admitted.imports.len());
        let mut lower = exact.clone();
        lower.max_symbols -= 1;
        resource(bounded(path, lang, source, &lower).unwrap_err());
        resource(check_output_limits(&oracle, &lower).unwrap_err());
        let mut lower = exact.clone();
        lower.max_references -= 1;
        resource(bounded(path, lang, source, &lower).unwrap_err());
        resource(check_output_limits(&oracle, &lower).unwrap_err());
        let mut lower = exact.clone();
        lower.max_imports -= 1;
        resource(bounded(path, lang, source, &lower).unwrap_err());
        resource(check_output_limits(&oracle, &lower).unwrap_err());
        if exact.max_diagnostics > 0 {
            let mut lower = exact.clone();
            lower.max_diagnostics -= 1;
            resource(bounded(path, lang, source, &lower).unwrap_err());
            resource(check_output_limits(&oracle, &lower).unwrap_err());
        }
        let mut lower = exact.clone();
        lower.max_bytes -= 1;
        resource(bounded(path, lang, source, &lower).unwrap_err());
        resource(check_output_limits(&oracle, &lower).unwrap_err());
        let mut higher = exact;
        higher.max_bytes += 1;
        assert_eq!(
            bytes(&bounded(path, lang, source, &higher).unwrap()),
            encoded,
            "cap +1: {lang}"
        );
    }
}

#[test]
fn malformed_diagnostics_are_admitted_or_fail_the_whole_parse() {
    for (path, lang, source) in fixtures() {
        let source = &source[..source.floor_char_boundary(source.len() / 2)];
        let trusted = parse("r", path, lang, source).unwrap();
        let good = bounded(path, lang, source, &OutputLimits::default()).unwrap();
        assert_eq!(bytes(&good), bytes(&trusted), "{lang}");
        if !trusted.diagnostics.is_empty() {
            let limits = OutputLimits {
                max_diagnostics: trusted.diagnostics.len() - 1,
                ..OutputLimits::default()
            };
            resource(bounded(path, lang, source, &limits).unwrap_err());
        }
    }
    let source = "fn broken( {";
    let parsed = parse("r", "a.rs", "rust", source).unwrap();
    assert!(!parsed.diagnostics.is_empty());
    resource(
        bounded(
            "a.rs",
            "rust",
            source,
            &OutputLimits {
                max_diagnostics: 0,
                ..OutputLimits::default()
            },
        )
        .unwrap_err(),
    );
}

#[test]
fn broad_and_nested_real_sources_reject_then_retry_without_state_leakage() {
    for (path, lang, source) in fixtures() {
        let broad = source.repeat(12);
        let cap = OutputLimits {
            max_symbols: 6,
            ..OutputLimits::default()
        };
        for _ in 0..3 {
            resource(bounded(path, lang, &broad, &cap).unwrap_err());
            let good = bounded(path, lang, source, &OutputLimits::default()).unwrap();
            assert_eq!(
                bytes(&good),
                bytes(&parse("r", path, lang, source).unwrap()),
                "retry {lang}"
            );
        }
    }
    for (path, lang, prefix, open, body, close, suffix) in [
        ("nested.rs", "rust", "fn run(){", "{", "leaf();", "}", "}"),
        (
            "nested.py",
            "python",
            "def run():\n",
            "",
            "    leaf()\n",
            "",
            "",
        ),
        (
            "nested.ts",
            "typescript",
            "function run(){",
            "if(true){",
            "leaf();",
            "}",
            "}",
        ),
        (
            "nested.js",
            "javascript",
            "function run(){",
            "if(true){",
            "leaf();",
            "}",
            "}",
        ),
        (
            "nested.go",
            "go",
            "package app\nfunc run(){",
            "if true {",
            "leaf()",
            "}",
            "}",
        ),
        (
            "Nested.java",
            "java",
            "class Nested {void run(){",
            "if(true){",
            "leaf();",
            "}",
            "}}",
        ),
        (
            "Nested.cs",
            "csharp",
            "class Nested {void Run(){",
            "if(true){",
            "Leaf();",
            "}",
            "}}",
        ),
        (
            "nested.c",
            "c",
            "void run(){",
            "if(1){",
            "leaf();",
            "}",
            "}",
        ),
        (
            "nested.cpp",
            "cpp",
            "void run(){",
            "if(true){",
            "leaf();",
            "}",
            "}",
        ),
        (
            "nested.rb",
            "ruby",
            "def run\n",
            "if true\n",
            "leaf()\n",
            "end\n",
            "end\n",
        ),
        (
            "nested.php",
            "php",
            "<?php function run(){",
            "if(true){",
            "leaf();",
            "}",
            "}",
        ),
    ] {
        let source = format!(
            "{prefix}{}{body}{}{suffix}",
            open.repeat(10),
            close.repeat(10)
        );
        let good = bounded(path, lang, &source, &OutputLimits::default()).unwrap();
        assert_eq!(
            bytes(&good),
            bytes(&parse("r", path, lang, &source).unwrap()),
            "nested {lang}"
        );
        resource(
            bounded(
                path,
                lang,
                &source,
                &OutputLimits {
                    max_references: 0,
                    ..OutputLimits::default()
                },
            )
            .unwrap_err(),
        );
    }
}

#[test]
fn ast_admission_precedes_emitters_and_keeps_its_existing_limits() {
    let source = "fn run(){leaf();}";
    let mut parser = Parser::new();
    parser
        .set_language(&grammar("rust", "a.rs").unwrap())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let nodes = tree.root_node().descendant_count();
    let exact = ParseLimits {
        max_ast_nodes: nodes,
        max_ast_depth: 192,
    };
    parse_bounded_with_output_limits(
        "r",
        "a.rs",
        "rust",
        source,
        &exact,
        &OutputLimits::default(),
    )
    .unwrap();
    let error = resource(
        parse_bounded_with_output_limits(
            "r",
            "a.rs",
            "rust",
            source,
            &ParseLimits {
                max_ast_nodes: nodes - 1,
                max_ast_depth: 192,
            },
            &OutputLimits {
                max_symbols: 0,
                ..OutputLimits::default()
            },
        )
        .unwrap_err(),
    );
    assert!(error.message.contains("AST nodes"), "{error:?}");
    let deep = format!("fn run(){{{}leaf();{}}}", "{".repeat(200), "}".repeat(200));
    let error = resource(bounded("a.rs", "rust", &deep, &OutputLimits::default()).unwrap_err());
    assert!(error.message.contains("AST depth"));
    // The compatibility API predates AST admission; production workers declare an 8 MiB stack.
    let trusted_deep = deep.clone();
    assert!(
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || parse("r", "a.rs", "rust", &trusted_deep))
            .unwrap()
            .join()
            .unwrap()
            .is_ok()
    );
    let admitted_deep = format!("fn run(){{{}leaf();{}}}", "{".repeat(80), "}".repeat(80));
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            assert_eq!(
                bytes(&bounded("a.rs", "rust", &admitted_deep, &OutputLimits::default()).unwrap()),
                bytes(&parse("r", "a.rs", "rust", &admitted_deep).unwrap())
            );
        })
        .unwrap()
        .join()
        .unwrap();
    resource(
        parse_bounded_with_output_limits(
            "r",
            "a.rs",
            "rust",
            source,
            &ParseLimits {
                max_ast_nodes: nodes,
                max_ast_depth: 1,
            },
            &OutputLimits::default(),
        )
        .unwrap_err(),
    );
}

fn real_tree(path: &str, lang: &str, source: &str) -> tree_sitter::Tree {
    let mut parser = Parser::new();
    parser.set_language(&grammar(lang, path).unwrap()).unwrap();
    parser.parse(source, None).unwrap()
}
#[test]
fn live_sink_refuses_before_retaining_more_real_symbols_or_references() {
    let source = (0..40)
        .map(|i| format!("fn f{i}(){{leaf();}}\n"))
        .collect::<String>();
    let tree = real_tree("a.rs", "rust", &source);
    let limits = OutputLimits {
        max_symbols: 4,
        ..OutputLimits::default()
    };
    let mut sink = file_node(
        tree.root_node(),
        "r",
        "a.rs",
        "rust",
        &source,
        Some(&limits),
        None,
    )
    .unwrap();
    let owner = sink.symbols[0].symbol_id.clone();
    resource(walk(tree.root_node(), &source, "r", &mut sink, &owner, "", 0).unwrap_err());
    sink.assert_accounting();
    assert_eq!(sink.symbols.len(), 4);
    assert_eq!(sink.references.len(), 3);
    assert!(!sink.symbols.iter().any(|s| s.name == "f3"));

    let limits = OutputLimits {
        max_references: 2,
        ..OutputLimits::default()
    };
    let mut sink = file_node(
        tree.root_node(),
        "r",
        "a.rs",
        "rust",
        &source,
        Some(&limits),
        None,
    )
    .unwrap();
    resource(walk(tree.root_node(), &source, "r", &mut sink, &owner, "", 0).unwrap_err());
    sink.assert_accounting();
    assert_eq!(sink.references.len(), 2);
    assert_eq!(sink.symbols.len(), 4);
}

#[test]
fn p0_post_emit_metadata_and_commonjs_imports_are_not_an_unmetered_tail() {
    for (path, lang, source, key) in [
        (
            "a.js",
            "javascript",
            "function run(){}\nexports.a = run; exports.b = run; exports.c = run;",
            "export_names",
        ),
        (
            "a.go",
            "go",
            "package app\ntype Service struct{}\nfunc (s *Service) Run(a,b,c,d Payload) Payload {return a}",
            "method_shape",
        ),
        (
            "a.rs",
            "rust",
            "pub use crate::{a,b,c};\ntrait A {}\nfn run<T: A>(x:T){}",
            "trait_bounds",
        ),
    ] {
        let tree = real_tree(path, lang, source);
        let mut preliminary =
            file_node(tree.root_node(), "r", path, lang, source, None, None).unwrap();
        let owner = preliminary.symbols[0].symbol_id.clone();
        walk(
            tree.root_node(),
            source,
            "r",
            &mut preliminary,
            &owner,
            "",
            0,
        )
        .unwrap();
        let before = parsed_file_bytes(&preliminary, usize::MAX).unwrap();
        let final_file = parse("r", path, lang, source).unwrap();
        assert!(
            final_file
                .symbols
                .iter()
                .any(|s| !s.attributes[key].is_null()),
            "{lang}"
        );
        assert!(
            parsed_file_bytes(&final_file, usize::MAX).unwrap() > before,
            "{lang}"
        );
        let limits = OutputLimits {
            max_bytes: before,
            ..OutputLimits::default()
        };
        let mut sink = file_node(
            tree.root_node(),
            "r",
            path,
            lang,
            source,
            Some(&limits),
            None,
        )
        .unwrap();
        walk(tree.root_node(), source, "r", &mut sink, &owner, "", 0).unwrap();
        let admissions = sink.admissions;
        resource(p0::finish(tree.root_node(), source, &mut sink).unwrap_err());
        sink.assert_accounting();
        assert!(parsed_file_bytes(&sink, usize::MAX).unwrap() <= before);
        assert!(sink.admissions >= admissions);
        resource(bounded(path, lang, source, &limits).unwrap_err());
    }
    let source = "const {a,b,c,d} = require('./auth');";
    let tree = real_tree("a.js", "javascript", source);
    let limits = OutputLimits {
        max_imports: 2,
        ..OutputLimits::default()
    };
    let mut sink = file_node(
        tree.root_node(),
        "r",
        "a.js",
        "javascript",
        source,
        Some(&limits),
        None,
    )
    .unwrap();
    let owner = sink.symbols[0].symbol_id.clone();
    walk(tree.root_node(), source, "r", &mut sink, &owner, "", 0).unwrap();
    resource(p0::finish(tree.root_node(), source, &mut sink).unwrap_err());
    sink.assert_accounting();
    assert_eq!(sink.imports.len(), 2);
    assert_eq!(sink.import_scopes.len(), 2);
    assert_eq!(sink.import_spans.len(), 2);
}

#[test]
fn cancellation_and_deadlines_abort_without_poisoning_the_next_request() {
    let source = "fn run(){leaf();}";
    let control = RequestControl::new(Duration::from_secs(60)).unwrap();
    control.cancel();
    let error = parse_bounded_with_output_limits_and_control(
        "r",
        "a.rs",
        "rust",
        source,
        &ast_limits(),
        &OutputLimits::default(),
        &control,
    )
    .unwrap_err();
    assert_eq!((error.code, error.exit), ("CANCELLED", 9));
    let expired = RequestControl::from_deadline(Instant::now());
    let error = parse_bounded_with_output_limits_and_control(
        "r",
        "a.rs",
        "rust",
        source,
        &ast_limits(),
        &OutputLimits::default(),
        &expired,
    )
    .unwrap_err();
    assert_eq!((error.code, error.exit), ("TIMEOUT", 9));
    let tree = real_tree("a.rs", "rust", source);
    let control = RequestControl::new(Duration::from_secs(60)).unwrap();
    let mut sink = file_node(
        tree.root_node(),
        "r",
        "a.rs",
        "rust",
        source,
        Some(&OutputLimits::default()),
        Some(&control),
    )
    .unwrap();
    let owner = sink.symbols[0].symbol_id.clone();
    control.cancel();
    assert_eq!(
        walk(tree.root_node(), source, "r", &mut sink, &owner, "", 0)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(sink.symbols.len(), 1);
    assert!(sink.references.is_empty());
    assert_eq!(
        bounds::admit_with_control(tree.root_node(), &ast_limits(), Some(&control))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let fresh = RequestControl::new(Duration::from_secs(60)).unwrap();
    let good = parse_bounded_with_output_limits_and_control(
        "r",
        "a.rs",
        "rust",
        source,
        &ast_limits(),
        &OutputLimits::default(),
        &fresh,
    )
    .unwrap();
    assert_eq!(
        bytes(&good),
        bytes(&parse("r", "a.rs", "rust", source).unwrap())
    );
}

#[test]
fn escaped_unicode_owned_fields_and_empty_file_boundaries() {
    let (repo, path, source) = (
        "r\"\\\u{e9}",
        "src/caf\u{e9}\t\".rs",
        "/// caf\u{e9} \"doc\"\nfn caf\u{e9}() { other::leaf(); }\n",
    );
    let oracle = parse(repo, path, "rust", source).unwrap();
    assert!(oracle.symbols.iter().any(|s| {
        s.doc_comment
            .as_ref()
            .is_some_and(|s| s.contains("caf\u{e9}"))
    }));
    let encoded = bytes(&oracle);
    let exact = OutputLimits {
        max_bytes: encoded.len(),
        ..OutputLimits::default()
    };
    let result =
        parse_bounded_with_output_limits(repo, path, "rust", source, &ast_limits(), &exact)
            .unwrap();
    assert_eq!(bytes(&result), encoded);
    assert_eq!(check_output_limits(&oracle, &exact).unwrap(), encoded.len());
    resource(
        parse_bounded_with_output_limits(
            repo,
            path,
            "rust",
            source,
            &ast_limits(),
            &OutputLimits {
                max_bytes: encoded.len() - 1,
                ..exact
            },
        )
        .unwrap_err(),
    );
    let empty = parse("r", "empty.py", "python", "").unwrap();
    assert_eq!(empty.symbols.len(), 1);
    let limits = OutputLimits {
        max_symbols: 1,
        max_references: 0,
        max_imports: 0,
        max_diagnostics: 0,
        max_bytes: bytes(&empty).len(),
    };
    assert_eq!(
        bytes(&bounded("empty.py", "python", "", &limits).unwrap()),
        bytes(&empty)
    );
    resource(
        bounded(
            "empty.py",
            "python",
            "",
            &OutputLimits {
                max_symbols: 0,
                ..limits
            },
        )
        .unwrap_err(),
    );
}

#[test]
fn real_go_grouped_type_expansion_obeys_default_policy_and_cache_check() {
    let policy = OutputLimits::default();
    assert_eq!(policy.max_bytes, 32 * 1024 * 1024);
    // The locked Go grammar resolves large ambiguous lists into unnamed types.
    // Nine names retain actual grouped fields; escaped raw tags cross 32 MiB
    // in emitted JSON without increasing the source above the default 2 MiB.
    let names = (0..9)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let ty = format!("struct{{ Value string `{}` }}", "\\".repeat(1920 * 1024));
    let source =
        format!("package app\ntype Service struct{{}}\nfunc (s *Service) Run({names} {ty}) {{}}\n");
    assert!(source.len() < 2 * 1024 * 1024);
    let tree = real_tree("a.go", "go", &source);
    bounds::admit(tree.root_node(), &ast_limits()).unwrap();
    let mut sink = file_node(
        tree.root_node(),
        "r",
        "a.go",
        "go",
        &source,
        Some(&policy),
        None,
    )
    .unwrap();
    let owner = sink.symbols[0].symbol_id.clone();
    walk(tree.root_node(), &source, "r", &mut sink, &owner, "", 0).unwrap();
    let result = p0::finish(tree.root_node(), &source, &mut sink);
    assert!(
        result.is_err(),
        "tree_error={}, byte_size={}, types={:?}",
        tree.root_node().has_error(),
        parsed_file_bytes(&sink, usize::MAX).unwrap(),
        sink.symbols
            .iter()
            .filter_map(|s| s.attributes["method_shape"][0]
                .as_array()
                .map(|a| (a.len(), a.first().and_then(|v| v.as_str()).map(str::len))))
            .collect::<Vec<_>>()
    );
    let error = resource(result.unwrap_err());
    assert!(error.message.contains("JSON bytes"));
    sink.assert_accounting();
    assert!(parsed_file_bytes(&sink, usize::MAX).unwrap() <= policy.max_bytes);
    let retained_types = sink
        .symbols
        .iter()
        .find(|s| s.name == "Run")
        .unwrap()
        .attributes["method_shape"][0]
        .as_array()
        .unwrap()
        .len();
    assert!(retained_types > 0 && retained_types < 9);
    drop(sink);
    // A legacy cached real parse must fail the SAME default policy before a caller clones it.
    let cached = parse("r", "a.go", "go", &source).unwrap();
    resource(check_output_limits(&cached, &policy).unwrap_err());
    let expanded = OutputLimits {
        max_bytes: 96 * 1024 * 1024,
        ..policy
    };
    let size = check_output_limits(&cached, &expanded).unwrap();
    assert!(size > OutputLimits::default().max_bytes);
    assert_eq!(size, parsed_file_bytes(&cached, usize::MAX).unwrap());
}
