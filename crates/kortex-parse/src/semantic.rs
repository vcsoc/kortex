//! AST-only value/type facts. Receiver types are deliberately not inferred.
use super::{join, span, split_name, text};
use kortex_core::{ParsedFile, Reference, Symbol, hash, id};
use serde_json::json;
use tree_sitter::Node;

pub(super) fn visibility(n: Node<'_>, source: &str, language: &str, name: &str) -> &'static str {
    let mut c = n.walk();
    for child in n.named_children(&mut c) {
        if matches!(
            child.kind(),
            "accessibility_modifier" | "visibility_modifier"
        ) {
            return match text(child, source) {
                "public" | "pub" => "public",
                "private" => "private",
                "protected" => "protected",
                _ => "restricted",
            };
        }
    }
    if language == "go" {
        return if name.chars().next().is_some_and(char::is_uppercase) {
            "public"
        } else {
            "private"
        };
    }
    if name.starts_with('#') {
        return "private";
    }
    "unknown"
}
pub(super) fn declaration_scope<'a>(n: Node<'a>, language: &str) -> Node<'a> {
    enclosing_scope(n, language, false)
}
fn enclosing_scope<'a>(n: Node<'a>, language: &str, parameter: bool) -> Node<'a> {
    let mut p = n;
    let mut function_scoped = parameter;
    let mut ancestor = Some(n);
    while let Some(a) = ancestor {
        if language != "python" && a.kind() == "variable_declaration" {
            function_scoped = true;
            break;
        }
        if matches!(a.kind(), "block" | "statement_block") {
            break;
        }
        ancestor = a.parent();
    }
    while let Some(parent) = p.parent() {
        p = parent;
        if matches!(
            p.kind(),
            "function_item"
                | "function_definition"
                | "function_declaration"
                | "method_definition"
                | "method_declaration"
                | "arrow_function"
                | "function_expression"
                | "closure_expression"
                | "lambda"
        ) || (!function_scoped
            && language != "python"
            && matches!(
                p.kind(),
                "block"
                    | "statement_block"
                    | "for_statement"
                    | "for_expression"
                    | "for_in_statement"
                    | "catch_clause"
            ))
            || p.parent().is_none()
        {
            break;
        }
    }
    p
}

fn parameter(n: Node<'_>) -> bool {
    let mut p = Some(n);
    while let Some(a) = p {
        if a.kind().contains("parameter") {
            return true;
        }
        if matches!(
            a.kind(),
            "block" | "statement_block" | "assignment" | "let_declaration" | "variable_declarator"
        ) {
            break;
        }
        p = a.parent();
    }
    false
}

fn add_symbol(
    n: Node<'_>,
    source: &str,
    file: &mut ParsedFile,
    owner: &str,
    kind: &str,
    binding: bool,
) {
    let name = text(n, source);
    let Some(parent) = file.symbols.iter().find(|s| s.symbol_id == owner) else {
        return;
    };
    let prefix = if parent.kind == "file" {
        ""
    } else {
        &parent.qualified_name
    };
    let qualified = join(prefix, name);
    let repo = parent.repo_id.clone();
    let is_parameter = parameter(n);
    let scope = enclosing_scope(n, &file.language, is_parameter);
    // Python assignment rebinds a function/module name rather than introducing a new lexical name.
    if file.language == "python"
        && file.symbols.iter().any(|s| {
            s.name == name
                && s.owner_id.as_deref() == Some(owner)
                && s.attributes["binding"] == true
                && s.attributes["scope_start"].as_u64() == Some(scope.start_byte() as u64)
                && s.attributes["scope_end"].as_u64() == Some(scope.end_byte() as u64)
        })
    {
        return;
    }
    if file.symbols.iter().any(|s| {
        s.attributes["name_start"].as_u64() == Some(n.start_byte() as u64)
            && s.attributes["name_end"].as_u64() == Some(n.end_byte() as u64)
    }) {
        return;
    }
    let ordinal = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == qualified && s.kind == kind)
        .count()
        .to_string();
    let symbol_id = id(&[
        &repo,
        &file.language,
        &file.path,
        kind,
        &qualified,
        &ordinal,
    ]);
    let mut declaration = n;
    if let Some(p) = n.parent() {
        declaration = p;
    }
    let exported = super::exported(declaration, source).0;
    let visibility = visibility(declaration, source, &file.language, name);
    file.symbols.push(Symbol {
        symbol_revision_id: id(&[&symbol_id, &hash(text(declaration, source))]),
        symbol_id, repo_id: repo, kind: kind.into(), language: file.language.clone(), name: name.into(),
        qualified_name: qualified, file_path: file.path.clone(), span: span(n),
        signature: text(declaration, source).chars().take(2048).collect(),
        visibility: visibility.into(),
        attributes: json!({"name_start": n.start_byte(), "name_end": n.end_byte(),
            "binding": binding, "parameter": is_parameter, "scope_start": scope.start_byte(),
            "scope_end": scope.end_byte(), "exported": exported,
            "syntax_error": declaration.has_error(), "source_snippet": text(declaration, source).chars().take(512).collect::<String>()}),
        content_hash: hash(text(declaration, source)), doc_comment: None, owner_id: Some(owner.into()),
    });
}

pub(super) fn binding(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    let mut stack = vec![n];
    let mut visited = 0;
    while let Some(n) = stack.pop() {
        visited += 1;
        if visited > 4096 {
            file.diagnostics
                .push("binding expansion limit reached".into());
            break;
        }
        if matches!(
            n.kind(),
            "identifier"
                | "pattern_identifier"
                | "shorthand_field_identifier"
                | "shorthand_property_identifier_pattern"
        ) {
            let kind = if file.language == "python"
                && file
                    .symbols
                    .iter()
                    .any(|s| s.symbol_id == owner && s.kind == "class")
            {
                "property"
            } else {
                "variable"
            };
            add_symbol(n, source, file, owner, kind, true);
        } else if n.kind().contains("pattern")
            || matches!(n.kind(), "expression_list" | "tuple" | "list")
        {
            let mut c = n.walk();
            for child in n.named_children(&mut c) {
                // Object-pattern keys are not bindings; the value/alias is.
                if (n.kind() == "pair_pattern" && n.child_by_field_name("key") == Some(child))
                    || (matches!(n.kind(), "assignment_pattern" | "object_assignment_pattern")
                        && n.child_by_field_name("left") != Some(child))
                {
                    continue;
                }
                stack.push(child);
            }
        }
    }
}

pub(super) fn members(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    if matches!(n.kind(), "global_statement" | "nonlocal_statement") {
        if let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == owner) {
            if !s.attributes["unsupported_bindings"].is_array() {
                s.attributes["unsupported_bindings"] = json!([]);
            }
            let mut c = n.walk();
            for name in n
                .named_children(&mut c)
                .filter(|child| child.kind() == "identifier")
            {
                s.attributes["unsupported_bindings"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(text(name, source)));
            }
        }
        return;
    }
    if matches!(
        n.kind(),
        "type_parameter"
            | "type_parameter_declaration"
            | "constrained_type_parameter"
            | "optional_type_parameter"
    ) {
        if let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == owner) {
            if !s.attributes["type_bindings"].is_array() {
                s.attributes["type_bindings"] = json!([]);
            }
            let mut c = n.walk();
            for name in n
                .named_children(&mut c)
                .filter(|child| n.child_by_field_name("name") == Some(*child))
            {
                s.attributes["type_bindings"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(text(name, source)));
            }
        }
        return;
    }
    let kind = match n.kind() {
        "field_declaration" => "field",
        "public_field_definition" | "field_definition" | "property_signature" => "property",
        "const_spec" => "constant",
        _ => return,
    };
    let mut c = n.walk();
    for child in n.named_children(&mut c) {
        if (n.child_by_field_name("name") == Some(child)
            || (n.kind() == "field_definition" && n.child_by_field_name("property") == Some(child))
            || (file.language == "go" && matches!(child.kind(), "identifier" | "field_identifier")))
            && matches!(
                child.kind(),
                "identifier"
                    | "field_identifier"
                    | "property_identifier"
                    | "private_property_identifier"
            )
        {
            add_symbol(child, source, file, owner, kind, kind == "constant");
        }
    }
}

fn import_or_metadata(n: Node<'_>) -> bool {
    matches!(
        n.kind(),
        "use_declaration"
            | "import_statement"
            | "import_from_statement"
            | "import_declaration"
            | "import_spec"
            | "package_clause"
            | "attribute_item"
            | "inner_attribute_item"
            | "macro_invocation"
            | "decorator"
            | "global_statement"
            | "nonlocal_statement"
    )
}
fn aggregate(n: Node<'_>) -> bool {
    matches!(
        n.kind(),
        "scoped_identifier"
            | "scoped_type_identifier"
            | "member_expression"
            | "attribute"
            | "selector_expression"
            | "field_expression"
            | "qualified_type"
            | "nested_type_identifier"
    )
}
fn type_context(n: Node<'_>) -> bool {
    if matches!(
        n.kind(),
        "type_identifier" | "scoped_type_identifier" | "qualified_type" | "nested_type_identifier"
    ) {
        return true;
    }
    let mut child = n;
    while let Some(parent) = child.parent() {
        if parent.kind() == "type_query" {
            return false;
        }
        if matches!(
            parent.kind(),
            "type_annotation"
                | "type_arguments"
                | "type"
                | "generic_type"
                | "trait_bounds"
                | "type_bound"
        ) || parent.child_by_field_name("type") == Some(child)
            || parent.child_by_field_name("return_type") == Some(child)
        {
            return true;
        }
        if matches!(
            parent.kind(),
            "block"
                | "statement_block"
                | "call"
                | "call_expression"
                | "assignment"
                | "expression_statement"
        ) {
            break;
        }
        child = parent;
    }
    false
}
fn ignored_name(n: Node<'_>, file: &ParsedFile) -> bool {
    if file.symbols.iter().any(|s| {
        s.attributes["name_start"].as_u64() == Some(n.start_byte() as u64)
            && s.attributes["name_end"].as_u64() == Some(n.end_byte() as u64)
    }) {
        return true;
    }
    let Some(p) = n.parent() else {
        return true;
    };
    if matches!(
        p.kind(),
        "type_parameter"
            | "type_parameter_declaration"
            | "constrained_type_parameter"
            | "optional_type_parameter"
    ) && p.child_by_field_name("name") == Some(n)
    {
        return true;
    }
    if matches!(
        p.kind(),
        "lifetime_parameter" | "label" | "break_statement" | "continue_statement"
    ) {
        return true;
    }
    if matches!(p.kind(), "pair" | "pair_pattern") && p.child_by_field_name("key") == Some(n) {
        return true;
    }
    // Reassignments in Python/JS are writes, not new read references on their LHS.
    if matches!(p.kind(), "assignment" | "assignment_expression")
        && p.child_by_field_name("left") == Some(n)
    {
        return true;
    }
    false
}

pub(super) fn mutation(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    if let Some(left) = n.child_by_field_name("left")
        && left.kind() == "identifier"
    {
        let scope = declaration_scope(n, &file.language);
        if let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == owner) {
            if !s.attributes["mutated_bindings"].is_array() {
                s.attributes["mutated_bindings"] = json!([]);
            }
            s.attributes["mutated_bindings"].as_array_mut().unwrap().push(json!({"name": text(left, source), "start": n.start_byte(), "end": scope.end_byte()}));
        }
    }
}
pub(super) fn unknown_receiver(file: &mut ParsedFile, owner: &str, start_byte: usize) {
    if let Some(s) = file.symbols.iter_mut().find(|s| s.symbol_id == owner) {
        if !s.attributes["runtime_receiver_sites"].is_array() {
            s.attributes["runtime_receiver_sites"] = json!([]);
        }
        s.attributes["runtime_receiver_sites"]
            .as_array_mut()
            .unwrap()
            .push(json!(start_byte));
    }
}
pub(super) fn reference(n: Node<'_>, source: &str, file: &mut ParsedFile, owner: &str) {
    if !aggregate(n)
        && !matches!(
            n.kind(),
            "identifier"
                | "type_identifier"
                | "field_identifier"
                | "property_identifier"
                | "shorthand_property_identifier"
                | "shorthand_field_identifier"
        )
    {
        return;
    }
    if ignored_name(n, file) {
        return;
    }
    let is_type = type_context(n);
    let mut child = n;
    let mut callee_part = true;
    while let Some(p) = child.parent() {
        if matches!(
            p.kind(),
            "lambda" | "closure_expression" | "arrow_function" | "function_expression"
        ) {
            callee_part = false;
        }
        if import_or_metadata(p) {
            return;
        }
        if aggregate(p) {
            return;
        } // the complete qualified name is emitted once
        if !is_type
            && callee_part
            && matches!(p.kind(), "call" | "call_expression")
            && p.child_by_field_name("function") == Some(child)
        {
            return;
        }
        if p.kind() == "lifetime" {
            return;
        }
        child = p;
    }
    let full = text(n, source);
    if full.contains(['(', ')', '[', ']', ' ', '\n', '?']) {
        return;
    }
    let (qualifier, name) = split_name(full);
    if name.is_empty() {
        return;
    }
    if file.language == "rust" && n.kind() == "field_expression" {
        unknown_receiver(file, owner, n.start_byte());
    }
    file.references.push(Reference {
        source_id: owner.into(),
        name: name.into(),
        qualifier: qualifier.map(str::to_owned),
        span: span(n),
        kind: if is_type { "USES_TYPE" } else { "REFERENCES" }.into(),
    });
}
