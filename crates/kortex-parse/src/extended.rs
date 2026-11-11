//! Independent grammar-specific P1 extraction. No regex provider or execution of build inputs.
use super::{span, text};
use kortex_core::{Import, ParsedFile, Reference, Symbol, hash, id};
use serde_json::json;
use tree_sitter::Node;

pub(super) fn supports(lang: &str) -> bool {
    matches!(lang, "java" | "csharp" | "c" | "cpp" | "ruby" | "php")
}
pub(super) fn canonical(name: &str) -> String {
    name.trim_start_matches('\\').replace(['.', '\\'], "::")
}
pub(super) fn extract(
    root: Node<'_>,
    source: &str,
    repo: &str,
    file: &mut ParsedFile,
    owner: &str,
) {
    let mut namespace = String::new();
    // Java package and file-scoped C#/PHP namespaces affect sibling declarations.
    let mut c = root.walk();
    for n in root.named_children(&mut c) {
        if n.kind() == "package_declaration" {
            let mut c = n.walk();
            if let Some(name) = n
                .named_children(&mut c)
                .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
            {
                namespace = canonical(text(name, source));
            }
        } else if n.kind() == "file_scoped_namespace_declaration"
            && let Some(name) = n.child_by_field_name("name")
        {
            namespace = canonical(text(name, source));
        }
    }
    file.symbols[0].attributes["namespace"] = json!(namespace);
    file.symbols[0].attributes["provider"] = json!(format!("tree-sitter-{}-v1", file.language));
    if matches!(file.language.as_str(), "c" | "cpp") {
        file.diagnostics.push(format!("{}: preprocessing/compilation database not executed; conditional declarations and linkage remain conservative",file.path));
    }
    visit(root, source, repo, file, owner, &namespace, 0, false);
}
fn declaration<'a>(n: Node<'a>, lang: &str) -> Option<(&'static str, Node<'a>)> {
    let kind = match (lang, n.kind()) {
        ("java" | "csharp" | "php", "class_declaration") => "class",
        ("java" | "csharp", "record_declaration") => "class",
        ("java" | "csharp" | "php", "interface_declaration") => "interface",
        ("java" | "csharp" | "php", "enum_declaration") => "enum",
        ("csharp", "struct_declaration") => "struct",
        ("java" | "csharp" | "php", "method_declaration") => "method",
        ("java" | "csharp", "constructor_declaration") => "constructor",
        ("csharp", "property_declaration") => "property",
        ("csharp", "namespace_declaration" | "file_scoped_namespace_declaration")
        | ("cpp" | "php", "namespace_definition") => "namespace",
        ("php", "trait_declaration") => "trait",
        ("php", "function_definition") => "function",
        ("ruby", "class") => "class",
        ("ruby", "module") => "namespace",
        ("ruby", "method" | "singleton_method") => "method",
        ("c" | "cpp", "struct_specifier") => "struct",
        ("cpp", "class_specifier") => "class",
        ("c" | "cpp", "enum_specifier") => "enum",
        ("c" | "cpp", "function_definition") => {
            return declarator_name(n.child_by_field_name("declarator")?)
                .map(|name| ("function", name));
        }
        ("c" | "cpp", "declaration" | "field_declaration") => {
            let decl = n.child_by_field_name("declarator")?;
            if decl.kind() == "function_declarator" {
                return declarator_name(decl).map(|name| ("function", name));
            }
            return declarator_name(decl).map(|name| {
                (
                    if n.kind() == "field_declaration" {
                        "field"
                    } else {
                        "variable"
                    },
                    name,
                )
            });
        }
        ("java" | "csharp", "variable_declarator") => {
            if n.parent().is_some_and(|p| {
                p.kind() == "field_declaration"
                    || p.parent().is_some_and(|p| p.kind() == "field_declaration")
            }) {
                "field"
            } else {
                "variable"
            }
        }
        _ => return None,
    };
    n.child_by_field_name("name").map(|name| (kind, name))
}
fn declarator_name(mut n: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..32 {
        if matches!(
            n.kind(),
            "identifier" | "field_identifier" | "qualified_identifier"
        ) {
            return Some(n);
        }
        n = n.child_by_field_name("declarator")?;
    }
    None
}
#[allow(clippy::too_many_arguments)]
fn add(
    n: Node<'_>,
    name: Node<'_>,
    kind: &str,
    source: &str,
    repo: &str,
    file: &mut ParsedFile,
    owner: &str,
    prefix: &str,
    conditional: bool,
) -> String {
    let name_text = canonical(text(name, source));
    let qualified = if prefix.is_empty() {
        name_text.clone()
    } else {
        format!("{prefix}::{name_text}")
    };
    let ordinal = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == qualified && s.kind == kind)
        .count()
        .to_string();
    let symbol_id = id(&[repo, &file.language, &file.path, kind, &qualified, &ordinal]);
    let end = n
        .child_by_field_name("body")
        .map(|n| n.start_byte())
        .unwrap_or(n.end_byte());
    let signature = source[n.start_byte()..end]
        .chars()
        .take(2048)
        .collect::<String>();
    let mut c = n.walk();
    let modifiers: Vec<_> = n
        .named_children(&mut c)
        .filter(|n| n.kind().contains("modifier") || n.kind() == "storage_class_specifier")
        .map(|n| text(n, source))
        .collect();
    let static_method = n.kind() == "singleton_method"
        || modifiers
            .iter()
            .any(|s| s.split_whitespace().any(|w| w == "static"));
    let visibility = if modifiers
        .iter()
        .any(|s| s.split_whitespace().any(|w| w == "private"))
    {
        "private"
    } else if modifiers
        .iter()
        .any(|s| s.split_whitespace().any(|w| w == "protected"))
    {
        "protected"
    } else if modifiers
        .iter()
        .any(|s| s.split_whitespace().any(|w| w == "public"))
    {
        "public"
    } else {
        "unknown"
    };
    let override_marker = modifiers.iter().any(|m| {
        m.split_whitespace()
            .any(|w| w == "override" || w == "@Override")
    });
    let parameter = n.kind().contains("parameter");
    let scope = super::semantic::declaration_scope(n, &file.language);
    let declared_type = n
        .child_by_field_name("type")
        .map(|t| canonical(text(t, source)));
    let previous = n
        .prev_named_sibling()
        .filter(|n| n.kind().contains("comment"));
    file.symbols.push(Symbol {
        symbol_revision_id:id(&[&symbol_id,&hash(text(n,source))]),symbol_id:symbol_id.clone(),repo_id:repo.into(),kind:kind.into(),language:file.language.clone(),name:name_text.rsplit("::").next().unwrap_or("").into(),qualified_name:qualified,file_path:file.path.clone(),span:span(n),signature,
        visibility:visibility.into(),attributes:json!({"name_start":name.start_byte(),"name_end":name.end_byte(),"syntax_error":n.has_error(),"conditional":conditional,"static":static_method,"override":override_marker,"scope_start":scope.start_byte(),"scope_end":scope.end_byte(),"binding":kind=="variable","parameter":parameter,"declared_type":declared_type,"declaration_only": n.child_by_field_name("body").is_none() && matches!(kind,"method"|"function"|"constructor"),"source_snippet":text(n,source).chars().take(512).collect::<String>()}),content_hash:hash(text(n,source)),doc_comment:previous.map(|n|text(n,source).chars().take(4096).collect()),owner_id:Some(owner.into()),
    });
    symbol_id
}
fn import(
    file: &mut ParsedFile,
    n: Node<'_>,
    owner: &str,
    local: String,
    module: String,
    imported: &str,
) {
    file.imports.push(Import {
        local,
        module,
        imported: imported.into(),
    });
    file.import_scopes.push(owner.into());
    file.import_spans.push(span(n));
}
fn reference(file: &mut ParsedFile, n: Node<'_>, owner: &str, full: &str, kind: &str) {
    let full = canonical(full);
    if full.contains(['(', ')', '[', ']', ' ', '\n', '<', '>']) || full.is_empty() {
        return;
    }
    let (q, name) = full
        .rsplit_once("::")
        .map(|(q, name)| (Some(q.to_owned()), name))
        .unwrap_or((None, &full));
    file.references.push(Reference {
        source_id: owner.into(),
        name: name.into(),
        qualifier: q,
        span: span(n),
        kind: kind.into(),
    });
}
#[allow(clippy::too_many_arguments)]
fn visit(
    n: Node<'_>,
    source: &str,
    repo: &str,
    file: &mut ParsedFile,
    owner: &str,
    prefix: &str,
    depth: usize,
    conditional: bool,
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
    let conditional = conditional
        || matches!(
            n.kind(),
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_else"
        );
    let mut current = owner.to_owned();
    let mut qualified = prefix.to_owned();
    if let Some((kind, name)) = declaration(n, &file.language) {
        current = add(
            n,
            name,
            kind,
            source,
            repo,
            file,
            owner,
            prefix,
            conditional,
        );
        qualified = file.symbols.last().unwrap().qualified_name.clone();
    }
    if matches!(
        n.kind(),
        "formal_parameter" | "parameter" | "simple_parameter" | "parameter_declaration"
    ) && let Some(name) = n.child_by_field_name("name").or_else(|| {
        n.child_by_field_name("declarator")
            .and_then(declarator_name)
    }) && !file
        .symbols
        .iter()
        .any(|s| s.attributes["name_start"].as_u64() == Some(name.start_byte() as u64))
    {
        add(
            n,
            name,
            "variable",
            source,
            repo,
            file,
            &current,
            &qualified,
            conditional,
        );
    }
    match (file.language.as_str(), n.kind()) {
        ("c" | "cpp", "preproc_def" | "preproc_function_def" | "preproc_undef") => {
            if let Some(name) = n.child_by_field_name("name")
                && let Some(s) = file.symbols.iter_mut().find(|s| s.kind == "file")
            {
                if !s.attributes["macro_bindings"].is_array() {
                    s.attributes["macro_bindings"] = json!([]);
                }
                s.attributes["macro_bindings"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(text(name, source)));
            }
            return;
        }
        ("java", "import_declaration") => {
            let mut c = n.walk();
            if let Some(name) = n
                .named_children(&mut c)
                .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
            {
                let module = canonical(text(name, source));
                let local = module.rsplit("::").next().unwrap_or("").to_owned();
                let star = text(n, source)
                    .trim_end_matches(';')
                    .trim_end()
                    .ends_with('*');
                import(
                    file,
                    n,
                    owner,
                    if star { String::new() } else { local },
                    module,
                    "*",
                );
            }
            return;
        }
        ("csharp", "using_directive") => {
            let mut c = n.walk();
            let name = n.named_children(&mut c).last();
            if let Some(name) = name {
                let alias = n
                    .child_by_field_name("name")
                    .map(|n| text(n, source).to_owned())
                    .unwrap_or_default();
                import(file, n, owner, alias, canonical(text(name, source)), "*");
            }
            return;
        }
        ("c" | "cpp", "preproc_include") => {
            if let Some(path) = n
                .child_by_field_name("path")
                .filter(|n| n.kind() == "string_literal")
            {
                import(
                    file,
                    n,
                    owner,
                    String::new(),
                    super::unquote(text(path, source)),
                    "*",
                );
            } else {
                file.diagnostics.push(format!(
                    "{}:{}: non-local or dynamic include unresolved",
                    file.path,
                    n.start_position().row + 1
                ));
            }
            return;
        }
        ("php", "namespace_use_clause") => {
            let mut c = n.walk();
            if let Some(name) = n
                .named_children(&mut c)
                .find(|child| Some(*child) != n.child_by_field_name("alias"))
            {
                let module = canonical(text(name, source));
                let local = n
                    .child_by_field_name("alias")
                    .map(|n| text(n, source))
                    .unwrap_or_else(|| module.rsplit("::").next().unwrap_or(""));
                import(file, n, owner, local.into(), module.clone(), "*");
            }
            return;
        }
        ("java", "method_invocation") => {
            if let Some(name) = n.child_by_field_name("name") {
                let object = n.child_by_field_name("object").map(|n| text(n, source));
                let full = object
                    .map(|q| format!("{q}::{}", text(name, source)))
                    .unwrap_or_else(|| text(name, source).into());
                reference(file, n, &current, &full, "CALLS");
                if n.child_by_field_name("object").is_some_and(|q| {
                    q.kind() == "identifier"
                        && text(q, source)
                            .chars()
                            .next()
                            .is_some_and(char::is_lowercase)
                        && text(q, source) != "this"
                }) {
                    mark_dynamic(file, &current, n);
                }
            }
        }
        ("csharp", "invocation_expression")
        | ("c" | "cpp", "call_expression")
        | ("php", "function_call_expression") => {
            if let Some(callee) = n.child_by_field_name("function") {
                let full = text(callee, source);
                reference(file, n, &current, full, "CALLS");
                if matches!(callee.kind(), "field_expression" | "pointer_expression") {
                    mark_dynamic(file, &current, n);
                }
            }
        }
        (
            "php",
            "scoped_call_expression" | "member_call_expression" | "nullsafe_member_call_expression",
        ) => {
            if let (Some(name), Some(receiver)) = (
                n.child_by_field_name("name"),
                n.child_by_field_name("scope")
                    .or_else(|| n.child_by_field_name("object")),
            ) {
                let q = text(receiver, source);
                // Preserve the call site even when the dynamic PHP receiver is unknown.
                let full = format!("{}::{}", q.trim_start_matches('$'), text(name, source));
                reference(file, n, &current, &full, "CALLS");
                if n.kind() != "scoped_call_expression"
                    || !matches!(
                        receiver.kind(),
                        "name" | "qualified_name" | "relative_scope"
                    )
                {
                    mark_dynamic(file, &current, n);
                }
            }
        }
        ("ruby", "call") => {
            if let Some(method) = n.child_by_field_name("method") {
                let name = text(method, source);
                if matches!(name, "require_relative" | "require") {
                    if let Some(arg) = n
                        .child_by_field_name("arguments")
                        .and_then(|n| n.named_child(0))
                        .filter(|n| n.kind() == "string")
                    {
                        if name == "require_relative" {
                            import(
                                file,
                                n,
                                owner,
                                String::new(),
                                super::unquote(text(arg, source)),
                                "*",
                            );
                        } else {
                            file.diagnostics.push(format!(
                                "{}:{}: require load-path resolution unsupported",
                                file.path,
                                n.start_position().row + 1
                            ));
                        }
                    }
                } else {
                    let receiver = n.child_by_field_name("receiver");
                    let full = receiver
                        .map(|q| format!("{}::{name}", text(q, source)))
                        .unwrap_or_else(|| name.into());
                    reference(file, n, &current, &full, "CALLS");
                    if receiver
                        .is_some_and(|q| !matches!(q.kind(), "constant" | "scope_resolution"))
                    {
                        mark_dynamic(file, &current, n);
                    }
                }
            }
        }
        (
            _,
            "superclass"
            | "super_interfaces"
            | "extends_interfaces"
            | "base_list"
            | "base_clause"
            | "class_interface_clause"
            | "base_class_clause",
        ) => {
            let kind = if matches!(n.kind(), "super_interfaces" | "class_interface_clause") {
                "IMPLEMENTS"
            } else {
                "INHERITS"
            };
            let mut pending = vec![n];
            while let Some(base) = pending.pop() {
                if matches!(
                    base.kind(),
                    "type_identifier"
                        | "identifier"
                        | "name"
                        | "qualified_name"
                        | "qualified_identifier"
                        | "scope_resolution"
                ) {
                    reference(file, base, &current, text(base, source), kind);
                } else {
                    let mut c = base.walk();
                    for child in base.named_children(&mut c) {
                        pending.push(child);
                    }
                }
            }
        }
        _ => {}
    }
    if n.kind() == "type_parameter"
        && let Some(name) = n.child_by_field_name("name").or_else(|| n.named_child(0))
        && let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == current)
    {
        if !s.attributes["type_bindings"].is_array() {
            s.attributes["type_bindings"] = json!([]);
        }
        s.attributes["type_bindings"]
            .as_array_mut()
            .unwrap()
            .push(json!(text(name, source)));
    }
    if matches!(file.language.as_str(), "ruby" | "php")
        && n.kind() == "assignment"
        && let Some(name) = n
            .child_by_field_name("left")
            .filter(|n| matches!(n.kind(), "identifier" | "variable_name"))
        && !file
            .symbols
            .iter()
            .any(|s| s.name == text(name, source) && s.owner_id.as_deref() == Some(&current))
    {
        add(
            name,
            name,
            "variable",
            source,
            repo,
            file,
            &current,
            &qualified,
            conditional,
        );
    }
    if file.language == "ruby" && n.kind() == "method_parameters" {
        let mut c = n.walk();
        for name in n
            .named_children(&mut c)
            .filter(|n| n.kind() == "identifier")
        {
            add(
                name,
                name,
                "variable",
                source,
                repo,
                file,
                &current,
                &qualified,
                conditional,
            );
        }
    }
    value_reference(n, source, file, &current);
    // Explicit named types only; declarations, import names and aggregate fragments are excluded.
    if matches!(
        n.kind(),
        "type_identifier" | "qualified_type_identifier" | "named_type"
    ) && !file
        .symbols
        .iter()
        .any(|s| s.attributes["name_start"].as_u64() == Some(n.start_byte() as u64))
    {
        let mut ancestor = n.parent();
        let mut blocked = false;
        while let Some(p) = ancestor {
            if matches!(
                p.kind(),
                "import_declaration"
                    | "package_declaration"
                    | "namespace_use_clause"
                    | "superclass"
                    | "super_interfaces"
                    | "base_list"
                    | "base_clause"
                    | "base_class_clause"
            ) {
                blocked = true;
                break;
            }
            ancestor = p.parent();
        }
        if !blocked {
            reference(file, n, &current, text(n, source), "USES_TYPE");
        }
    }
    if matches!(
        n.kind(),
        "package_declaration" | "file_scoped_namespace_declaration" | "using_directive"
    ) {
        return;
    }
    let mut c = n.walk();
    for child in n.named_children(&mut c) {
        let file_namespace = n.parent().is_none()
            && (child.kind() == "file_scoped_namespace_declaration"
                || (file.language == "php"
                    && child.kind() == "namespace_definition"
                    && child.child_by_field_name("body").is_none()));
        if file_namespace {
            visit(child, source, repo, file, owner, "", depth + 1, conditional);
            if let Some(ns) = file
                .symbols
                .iter()
                .find(|s| s.kind == "namespace" && s.span.start_byte == child.start_byte())
            {
                current = ns.symbol_id.clone();
                qualified = ns.qualified_name.clone();
            }
        } else {
            visit(
                child,
                source,
                repo,
                file,
                &current,
                &qualified,
                depth + 1,
                conditional,
            );
        }
    }
}
fn value_reference(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    if !matches!(
        n.kind(),
        "identifier"
            | "name"
            | "variable_name"
            | "constant"
            | "qualified_name"
            | "qualified_identifier"
            | "member_access_expression"
            | "field_access"
            | "field_expression"
            | "scope_resolution"
    ) {
        return;
    }
    if file.symbols.iter().any(|s| {
        s.attributes["name_start"].as_u64() == Some(n.start_byte() as u64)
            && s.attributes["name_end"].as_u64() == Some(n.end_byte() as u64)
    }) {
        return;
    }
    let mut child = n;
    let mut ancestor = n.parent();
    let mut kind = "REFERENCES";
    while let Some(p) = ancestor {
        if matches!(
            p.kind(),
            "package_declaration"
                | "import_declaration"
                | "using_directive"
                | "namespace_use_declaration"
                | "namespace_use_clause"
                | "preproc_include"
                | "superclass"
                | "super_interfaces"
                | "extends_interfaces"
                | "base_list"
                | "base_clause"
                | "class_interface_clause"
                | "base_class_clause"
                | "type_parameter"
        ) {
            return;
        }
        if matches!(
            p.kind(),
            "qualified_name"
                | "qualified_identifier"
                | "member_access_expression"
                | "field_access"
                | "field_expression"
                | "scope_resolution"
                | "variable_name"
                | "named_type"
                | "type_identifier"
        ) {
            return;
        }
        if p.child_by_field_name("name") == Some(child)
            && matches!(
                p.kind(),
                "namespace_declaration"
                    | "namespace_definition"
                    | "class_declaration"
                    | "method_declaration"
                    | "function_definition"
                    | "variable_declarator"
                    | "formal_parameter"
                    | "simple_parameter"
                    | "parameter"
            )
        {
            return;
        }
        if matches!(
            p.kind(),
            "method_invocation"
                | "invocation_expression"
                | "call_expression"
                | "function_call_expression"
                | "member_call_expression"
                | "scoped_call_expression"
                | "call"
        ) && (p.child_by_field_name("function") == Some(child)
            || p.child_by_field_name("name") == Some(child)
            || p.child_by_field_name("object") == Some(child)
            || p.child_by_field_name("receiver") == Some(child)
            || p.child_by_field_name("method") == Some(child)
            || p.child_by_field_name("scope") == Some(child))
        {
            return;
        }
        if p.child_by_field_name("type") == Some(child)
            || p.child_by_field_name("returns") == Some(child)
            || p.child_by_field_name("return_type") == Some(child)
        {
            kind = "USES_TYPE";
        }
        if matches!(p.kind(), "assignment" | "assignment_expression")
            && p.child_by_field_name("left") == Some(child)
        {
            kind = "WRITES";
        }
        child = p;
        ancestor = p.parent();
    }
    reference(file, n, owner, text(n, source), kind);
}
fn mark_dynamic(file: &mut ParsedFile, owner: &str, n: Node<'_>) {
    if let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == owner) {
        if !s.attributes["dynamic_sites"].is_array() {
            s.attributes["dynamic_sites"] = json!([]);
        }
        s.attributes["dynamic_sites"]
            .as_array_mut()
            .unwrap()
            .push(json!(n.start_byte()));
    }
}
