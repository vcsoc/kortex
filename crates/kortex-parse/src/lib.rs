//! Real Tree-sitter providers. Facts are immutable; resolution is a separate pass.
use kortex_core::{Error, Import, ParsedFile, Reference, Result, Span, Symbol, hash, id};
use serde_json as kortex_json;
use std::time::{Duration, Instant};
use tree_sitter::{Language, Node, ParseOptions, Parser};
mod extended;
mod p0;
mod semantic;

pub trait LanguageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn parse(&self, repo: &str, path: &str, source: &str) -> Result<ParsedFile>;
}
pub struct Provider {
    pub language: &'static str,
}
impl LanguageProvider for Provider {
    fn id(&self) -> &'static str {
        self.language
    }
    fn extensions(&self) -> &'static [&'static str] {
        match self.language {
            "rust" => &["rs"],
            "python" => &["py", "pyi"],
            "typescript" => &["ts", "tsx"],
            "javascript" => &["js", "jsx", "mjs", "cjs"],
            "go" => &["go"],
            "java" => &["java"],
            "csharp" => &["cs"],
            "c" => &["c", "h"],
            "cpp" => &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            "ruby" => &["rb", "rake", "gemspec"],
            "php" => &["php", "phtml"],
            _ => &[],
        }
    }
    fn parse(&self, repo: &str, path: &str, source: &str) -> Result<ParsedFile> {
        parse(repo, path, self.language, source)
    }
}
pub fn span(n: Node<'_>) -> Span {
    let a = n.start_position();
    let b = n.end_position();
    Span {
        start_byte: n.start_byte(),
        end_byte: n.end_byte(),
        start_line: a.row + 1,
        start_col: a.column + 1,
        end_line: b.row + 1,
        end_col: b.column + 1,
    }
}
fn text<'a>(n: Node<'_>, s: &'a str) -> &'a str {
    &s[n.byte_range()]
}
fn grammar(lang: &str, path: &str) -> Result<Language> {
    Ok(match lang {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" if path.ends_with(".tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        _ => return Err(Error::new("UNSUPPORTED_LANGUAGE", lang, 7)),
    })
}
pub fn parse(repo: &str, path: &str, lang: &str, source: &str) -> Result<ParsedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&grammar(lang, path)?)
        .map_err(|e| Error::new("GRAMMAR_ERROR", e.to_string(), 7))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut cancel = |_: &tree_sitter::ParseState| Instant::now() > deadline;
    let bytes = source.as_bytes();
    let tree = parser
        .parse_with_options(
            &mut |offset, _| &bytes[offset..],
            None,
            Some(ParseOptions::new().progress_callback(&mut cancel)),
        )
        .ok_or_else(|| Error::new("PARSE_TIMEOUT", "parser exceeded five seconds", 9))?;
    let mut file = ParsedFile {
        path: path.into(),
        language: lang.into(),
        content_hash: hash(source),
        size_bytes: source.len(),
        symbols: vec![],
        references: vec![],
        imports: vec![],
        import_scopes: vec![],
        import_spans: vec![],
        diagnostics: vec![],
    };
    if tree.root_node().has_error() {
        file.diagnostics
            .push(format!("{path}: syntax errors; facts may be partial"));
    }
    let file_id = id(&[repo, lang, path, "file"]);
    file.symbols.push(Symbol {
        symbol_id: file_id.clone(),
        symbol_revision_id: id(&[&file_id, &file.content_hash]),
        repo_id: repo.into(),
        kind: "file".into(),
        language: lang.into(),
        name: path.into(),
        qualified_name: path.into(),
        file_path: path.into(),
        span: span(tree.root_node()),
        signature: String::new(),
        visibility: "unknown".into(),
        attributes: serde_value(),
        content_hash: file.content_hash.clone(),
        doc_comment: None,
        owner_id: None,
    });
    if extended::supports(lang) {
        extended::extract(tree.root_node(), source, repo, &mut file, &file_id);
    } else {
        walk(tree.root_node(), source, repo, &mut file, &file_id, "", 0);
        p0::finish(tree.root_node(), source, &mut file);
    }
    file.symbols.sort_by(|a, b| {
        a.span
            .start_byte
            .cmp(&b.span.start_byte)
            .then(a.symbol_id.cmp(&b.symbol_id))
    });
    Ok(file)
}
fn serde_value() -> kortex_json::Value {
    kortex_json::json!({})
}

fn declaration(n: Node<'_>) -> Option<&'static str> {
    Some(match n.kind() {
        "function_item"
        | "function_definition"
        | "function_declaration"
        | "generator_function_declaration" => "function",
        "method_definition"
        | "method_declaration"
        | "function_signature_item"
        | "method_signature"
        | "method_elem" => "method",
        "class_definition" | "class_declaration" => "class",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "interface_declaration" => "interface",
        "type_spec" => match n.child_by_field_name("type").map(|t| t.kind()) {
            Some("struct_type") => "struct",
            Some("interface_type") => "interface",
            _ => "type_alias",
        },
        "type_alias_declaration" | "type_item" => "type_alias",
        "mod_item" => "module",
        "const_item" | "static_item" => "constant",
        "assignment_expression"
            if n.child_by_field_name("right")
                .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression")) =>
        {
            "function"
        }
        "variable_declarator"
            if n.child_by_field_name("value")
                .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression")) =>
        {
            "function"
        }
        _ => return None,
    })
}
fn walk(
    n: Node<'_>,
    source: &str,
    repo: &str,
    file: &mut ParsedFile,
    owner: &str,
    prefix: &str,
    depth: usize,
) {
    if depth > 256 {
        if !file
            .diagnostics
            .iter()
            .any(|d| d == "AST depth limit reached")
        {
            file.diagnostics.push("AST depth limit reached".into());
        }
        return;
    }
    let mut current = owner.to_owned();
    let mut qualified = prefix.to_owned();
    // impl blocks are scopes, not declarations; retain the receiver in qualified names.
    if n.kind() == "impl_item"
        && let Some(t) = n.child_by_field_name("type")
    {
        qualified = join(prefix, text(t, source));
    }
    if file.language == "go"
        && n.kind() == "method_declaration"
        && let Some(receiver) = n.child_by_field_name("receiver")
    {
        let mut c = receiver.walk();
        if let Some(ty) = receiver
            .named_children(&mut c)
            .find_map(|p| p.child_by_field_name("type"))
        {
            qualified = join(prefix, text(ty, source).trim_start_matches('*'));
        }
    }
    if let Some(kind) = declaration(n)
        && let Some(name_node) = n.child_by_field_name("name").or_else(|| {
            if n.kind() != "assignment_expression" {
                return None;
            }
            let left = n.child_by_field_name("left")?;
            if !text(left, source).starts_with("exports.")
                && !text(left, source).starts_with("module.exports")
            {
                return None;
            }
            n.child_by_field_name("right")
                .and_then(|v| v.child_by_field_name("name"))
                .or_else(|| left.child_by_field_name("property"))
        })
    {
        let name = text(name_node, source);
        qualified = join(&qualified, name);
        let signature_end = n
            .child_by_field_name("body")
            .map(|b| b.start_byte())
            .unwrap_or(n.end_byte());
        let signature = source[n.start_byte()..signature_end]
            .chars()
            .take(2048)
            .collect::<String>();
        let ordinal = file
            .symbols
            .iter()
            .filter(|s| s.qualified_name == qualified && s.kind == kind)
            .count()
            .to_string();
        let symbol_id = id(&[repo, &file.language, &file.path, kind, &qualified, &ordinal]);
        let prev = n.prev_named_sibling();
        let doc = prev
            .filter(|p| p.kind().contains("comment"))
            .map(|p| text(p, source).chars().take(4096).collect());
        let test =
            name.starts_with("test_") || prev.is_some_and(|p| text(p, source).contains("#[test]"));
        let declaration_scope = semantic::declaration_scope(n, &file.language);
        file.symbols.push(Symbol {
            symbol_revision_id: id(&[&symbol_id, &hash(text(n, source))]),
            symbol_id: symbol_id.clone(),
            repo_id: repo.into(),
            kind: if test { "test" } else { kind }.into(),
            language: file.language.clone(),
            name: name.into(),
            qualified_name: qualified.clone(),
            file_path: file.path.clone(),
            span: span(n),
            signature,
            visibility: semantic::visibility(n, source, &file.language, name).into(),
            attributes: kortex_json::json!({"syntax_error":n.has_error(), "exported": exported(n, source).0, "default_export": exported(n, source).1, "source_snippet": text(n,source).chars().take(512).collect::<String>(), "name_start": name_node.start_byte(), "name_end": name_node.end_byte(), "scope_start": declaration_scope.start_byte(), "scope_end": declaration_scope.end_byte()}),
            content_hash: hash(text(n, source)),
            doc_comment: doc,
            owner_id: Some(owner.into()),
        });
        current = symbol_id;
    }
    semantic::members(n, source, file, &current);
    let imports_before = file.imports.len();
    match n.kind() {
        "package_clause" => {
            if let Some(package) = n.named_child(0)
                && let Some(s) = file.symbols.iter_mut().find(|s| s.kind == "file")
            {
                s.attributes["package"] = kortex_json::json!(text(package, source));
            }
        }
        "use_declaration" => {
            if let Some(arg) = n.child_by_field_name("argument") {
                rust_use(arg, source, "", &mut file.imports);
            }
        }
        "import_from_statement" => {
            let module = n
                .child_by_field_name("module_name")
                .map(|a| text(a, source))
                .unwrap_or("");
            let mut c = n.walk();
            for a in n.named_children(&mut c) {
                if n.child_by_field_name("module_name") == Some(a) {
                    continue;
                }
                let (original, local) = aliased(a, source);
                if !original.is_empty() {
                    file.imports.push(Import {
                        local,
                        module: module.into(),
                        imported: original,
                    });
                }
            }
        }
        "import_statement" if file.language == "python" => {
            let mut c = n.walk();
            for a in n.named_children(&mut c) {
                let (module, alias) = aliased(a, source);
                file.imports.push(Import {
                    local: alias,
                    module,
                    imported: "*".into(),
                });
            }
        }
        "import_statement" => {
            let module = n
                .child_by_field_name("source")
                .map(|a| unquote(text(a, source)))
                .unwrap_or_default();
            js_imports(n, source, &module, &mut file.imports);
        }
        "import_spec" => {
            if let Some(p) = n.child_by_field_name("path") {
                let module = unquote(text(p, source));
                let local = n
                    .child_by_field_name("name")
                    .map(|a| text(a, source).to_owned())
                    .unwrap_or_else(|| module.rsplit('/').next().unwrap_or("").into());
                file.imports.push(Import {
                    local,
                    module,
                    imported: "*".into(),
                });
            }
        }
        "call_expression" | "call" => {
            if let Some(callee) = n.child_by_field_name("function") {
                let full = text(callee, source);
                if !full.contains(['(', ')', '[', ']', ' ', '\n']) {
                    let (qualifier, name) = split_name(full);
                    if file.language == "rust" && callee.kind() == "field_expression" {
                        semantic::unknown_receiver(file, &current, n.start_byte());
                    }
                    file.references.push(Reference {
                        source_id: current.clone(),
                        name: name.into(),
                        qualifier: qualifier.map(str::to_owned),
                        span: span(n),
                        kind: "CALLS".into(),
                    });
                } else {
                    file.diagnostics.push(format!(
                        "{}:{}: dynamic callee unsupported",
                        file.path,
                        n.start_position().row + 1
                    ));
                }
            }
        }
        // Track local bindings so a same-name parameter/local never becomes a fabricated call edge.
        "parameter_declaration" if file.language == "go" => {
            let mut c = n.walk();
            for name in n.children_by_field_name("name", &mut c) {
                record_binding(name, source, file, &current);
            }
        }
        "catch_clause" => {
            if let Some(binding) = n.child_by_field_name("parameter") {
                record_binding(binding, source, file, &current);
            }
        }
        "for_expression" => {
            if let Some(binding) = n.child_by_field_name("pattern") {
                record_binding(binding, source, file, &current);
            }
        }
        "parameter"
        | "typed_parameter"
        | "default_parameter"
        | "required_parameter"
        | "optional_parameter"
        | "parameter_declaration"
        | "let_declaration"
        | "variable_declarator" => {
            if declaration(n).is_none() {
                let binding = n
                    .child_by_field_name("pattern")
                    .or_else(|| n.child_by_field_name("name"))
                    .or_else(|| n.named_child(0).filter(|n| n.kind() == "identifier"));
                if let Some(b) = binding {
                    record_binding(b, source, file, &current);
                }
            }
        }
        "parameters" | "formal_parameters" | "closure_parameters" | "lambda_parameters" => {
            let mut c = n.walk();
            for b in n.named_children(&mut c) {
                if b.kind() == "identifier" {
                    record_binding(b, source, file, &current);
                }
            }
        }
        "short_var_declaration" | "for_statement" => {
            if let Some(left) = n.child_by_field_name("left") {
                record_binding(left, source, file, &current);
            }
        }
        "var_spec" => {
            let mut c = n.walk();
            for child in n.named_children(&mut c) {
                if child.kind() == "identifier" {
                    record_binding(child, source, file, &current);
                }
            }
        }
        "assignment_expression" => semantic::mutation(n, source, file, &current),
        "assignment" if file.language == "python" => {
            if let Some(b) = n.child_by_field_name("left") {
                record_binding(b, source, file, &current);
            }
        }
        _ => {}
    }
    for _ in imports_before..file.imports.len() {
        file.import_scopes.push(current.clone());
        file.import_spans.push(span(n));
    }
    semantic::reference(n, source, file, &current);
    let mut c = n.walk();
    for child in n.named_children(&mut c) {
        walk(child, source, repo, file, &current, &qualified, depth + 1);
    }
}
fn exported(n: Node<'_>, source: &str) -> (bool, bool) {
    let mut p = n.parent();
    for _ in 0..3 {
        let Some(a) = p else {
            break;
        };
        if a.kind() == "export_statement" {
            return (true, text(a, source).starts_with("export default"));
        }
        if !matches!(a.kind(), "lexical_declaration" | "variable_declaration") {
            break;
        }
        p = a.parent();
    }
    (false, false)
}
fn record_binding(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    semantic::binding(n, source, file, owner);
}
fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.into()
    } else {
        format!("{a}::{b}")
    }
}
fn split_name(s: &str) -> (Option<&str>, &str) {
    if let Some((a, b)) = s.rsplit_once("::") {
        (Some(a), b)
    } else if let Some((a, b)) = s.rsplit_once('.') {
        (Some(a), b)
    } else {
        (None, s)
    }
}
fn unquote(s: &str) -> String {
    s.trim_matches(['\'', '"', '`']).into()
}
fn aliased(n: Node<'_>, s: &str) -> (String, String) {
    let original = n
        .child_by_field_name("name")
        .map(|a| text(a, s))
        .unwrap_or_else(|| text(n, s));
    let local = n
        .child_by_field_name("alias")
        .map(|a| text(a, s))
        .unwrap_or(original);
    (original.into(), local.into())
}
fn rust_use(n: Node<'_>, s: &str, prefix: &str, imports: &mut Vec<Import>) {
    match n.kind() {
        "scoped_use_list" => {
            let path = n
                .child_by_field_name("path")
                .map(|p| text(p, s))
                .unwrap_or("");
            let p = join(prefix, path);
            if let Some(list) = n.child_by_field_name("list") {
                rust_use(list, s, &p, imports);
            }
        }
        "use_list" => {
            let mut c = n.walk();
            for child in n.named_children(&mut c) {
                rust_use(child, s, prefix, imports);
            }
        }
        "use_as_clause" => {
            let path = n
                .child_by_field_name("path")
                .map(|p| text(p, s))
                .unwrap_or("");
            let alias = n
                .child_by_field_name("alias")
                .map(|p| text(p, s))
                .unwrap_or("");
            let full = join(prefix, path);
            let (module, name) = split_name(&full);
            imports.push(Import {
                local: alias.into(),
                module: module.unwrap_or("").into(),
                imported: name.into(),
            });
        }
        _ => {
            let full = join(prefix, text(n, s));
            let (module, name) = split_name(&full);
            imports.push(Import {
                local: name.into(),
                module: module.unwrap_or("").into(),
                imported: name.into(),
            });
        }
    }
}
fn js_imports(n: Node<'_>, s: &str, module: &str, imports: &mut Vec<Import>) {
    match n.kind() {
        "import_specifier" => {
            let (original, local) = aliased(n, s);
            imports.push(Import {
                local,
                module: module.into(),
                imported: original,
            });
            return;
        }
        "namespace_import" => {
            if let Some(a) = n.named_child(0) {
                imports.push(Import {
                    local: text(a, s).into(),
                    module: module.into(),
                    imported: "*".into(),
                });
            }
            return;
        }
        "import_clause" => {
            let mut c = n.walk();
            for a in n.named_children(&mut c) {
                if a.kind() == "identifier" {
                    imports.push(Import {
                        local: text(a, s).into(),
                        module: module.into(),
                        imported: "default".into(),
                    });
                } else {
                    js_imports(a, s, module, imports);
                }
            }
            return;
        }
        _ => {}
    }
    let mut c = n.walk();
    for a in n.named_children(&mut c) {
        js_imports(a, s, module, imports);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_grammars_extract_real_spans_and_calls() {
        for (path, lang, src) in [
            ("a.rs", "rust", "fn helper() {}\nfn entry() { helper(); }"),
            (
                "a.py",
                "python",
                "def helper():\n    pass\ndef entry():\n    helper()\n",
            ),
            (
                "a.ts",
                "typescript",
                "function helper(): void {}\nfunction entry() { helper(); }",
            ),
            (
                "a.js",
                "javascript",
                "function helper() {}\nfunction entry() { helper(); }",
            ),
            (
                "a.go",
                "go",
                "package app\nfunc helper() {}\nfunc entry() { helper() }",
            ),
        ] {
            let f = parse("r", path, lang, src).unwrap();
            assert!(f.diagnostics.is_empty(), "{lang}: {:?}", f.diagnostics);
            assert_eq!(
                f.symbols.iter().filter(|s| s.kind == "function").count(),
                2,
                "{lang}"
            );
            assert_eq!(f.references.len(), 1, "{lang}");
            assert_eq!(f.references[0].name, "helper");
            for s in f.symbols.iter().filter(|s| s.kind == "function") {
                assert!(src[s.span.start_byte..s.span.end_byte].contains(&s.name));
                assert!(s.span.start_line > 0);
            }
        }
    }
    #[test]
    fn imports_and_malformed_input() {
        for (lang, path, source) in [
            (
                "rust",
                "a.rs",
                "use crate::auth::check as verify; fn x() { verify(); }",
            ),
            (
                "python",
                "a.py",
                "from auth import check as verify\ndef x(): verify()",
            ),
            (
                "typescript",
                "a.ts",
                "import {check as verify} from './auth'; function x(){verify();}",
            ),
            (
                "go",
                "a.go",
                "package a\nimport auth \"app/auth\"\nfunc x(){auth.Check()}",
            ),
        ] {
            let f = parse("r", path, lang, source).unwrap();
            assert_eq!(f.imports.len(), 1, "{lang}: {:?}", f.imports);
            assert_eq!(f.import_spans.len(), f.imports.len());
            assert!(f.import_spans.iter().all(|s| s.start_line >= 1
                && s.start_col >= 1
                && !source[s.start_byte..s.end_byte].is_empty()));
        }
        let f = parse("r", "broken.rs", "rust", "fn broken( { ").unwrap();
        assert!(!f.diagnostics.is_empty());
        assert!(parse("r", "x", "unknown", "").is_err());
    }
}
