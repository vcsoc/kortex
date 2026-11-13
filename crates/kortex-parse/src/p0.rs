//! Additional P0 facts derived from grammar nodes, never source-text pattern matching.
use super::{Sink, span, text};
use kortex_core::{Import, Reference, Result};
use serde_json::json;
use tree_sitter::Node;

pub(super) fn finish(root: Node<'_>, source: &str, file: &mut Sink) -> Result<()> {
    if file.language == "rust"
        && let Some(i) = file.symbols.iter().position(|s| s.kind == "file")
    {
        file.attribute(i, "rust_reexports", json!([]))?;
        for j in 0..file.import_spans.len() {
            let s = &file.import_spans[j];
            if source[s.start_byte..s.end_byte].starts_with("pub use ") {
                file.append_attribute(i, "rust_reexports", json!(j))?;
            }
        }
    }
    let mut stack = vec![(root, 0usize)];
    while let Some((n, depth)) = stack.pop() {
        file.checkpoint()?;
        if depth > 256 {
            continue;
        }
        if matches!(file.language.as_str(), "javascript" | "typescript") {
            commonjs(n, source, file)?;
        }
        if file.language == "go"
            && n.kind() == "method_declaration"
            && let (Some(receiver), Some(name)) = (
                n.child_by_field_name("receiver"),
                n.child_by_field_name("name"),
            )
        {
            let mut c = receiver.walk();
            if let Some(param) = receiver
                .named_children(&mut c)
                .find(|p| p.kind() == "parameter_declaration")
            {
                let ty = param
                    .child_by_field_name("type")
                    .map(|t| text(t, source).trim_start_matches('*'))
                    .unwrap_or("");
                if let Some(i) = file.symbols.iter().position(|s| {
                    s.attributes["name_start"].as_u64() == Some(name.start_byte() as u64)
                }) {
                    file.attribute(i, "receiver_type", json!(ty))?;
                }
            }
        }
        if file.language == "go"
            && matches!(n.kind(), "method_declaration" | "method_elem")
            && let Some(parameters) = n.child_by_field_name("parameters")
            && let Some(i) = file.symbols.iter().position(|s| {
                s.span.start_byte == n.start_byte() && s.span.end_byte == n.end_byte()
            })
        {
            let result = n
                .child_by_field_name("result")
                .map(|n| text(n, source))
                .unwrap_or("");
            file.attribute(i, "method_shape", json!([[], result]))?;
            let mut c = parameters.walk();
            for param in parameters.named_children(&mut c) {
                if let Some(ty) = param.child_by_field_name("type") {
                    let mut c = param.walk();
                    let count = param.children_by_field_name("name", &mut c).count().max(1);
                    for _ in 0..count {
                        file.append_array_slot(i, "method_shape", 0, json!(text(ty, source)))?;
                    }
                }
            }
        }
        if file.language == "rust"
            && matches!(n.kind(), "type_parameter" | "where_predicate")
            && let (Some(name), Some(bounds)) = (
                n.child_by_field_name("name")
                    .or_else(|| n.child_by_field_name("left")),
                n.child_by_field_name("bounds"),
            )
            && let Some(i) = file
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.span.start_byte <= n.start_byte() && n.end_byte() <= s.span.end_byte
                })
                .min_by_key(|(_, s)| s.span.end_byte - s.span.start_byte)
                .map(|(i, _)| i)
        {
            let key = text(name, source);
            file.nested_attribute(i, Some("trait_bounds"), key, json!([]))?;
            let mut c = bounds.walk();
            for name in bounds
                .named_children(&mut c)
                .filter(|n| matches!(n.kind(), "type_identifier" | "scoped_type_identifier"))
            {
                file.append_nested_attribute(i, "trait_bounds", key, json!(text(name, source)))?;
            }
        }
        if file.language == "rust" && n.kind().ends_with("_item") {
            let mut previous = n.prev_named_sibling();
            let mut conditional = false;
            while let Some(attr) = previous.filter(|n| n.kind() == "attribute_item") {
                conditional |= text(attr, source).contains("cfg");
                previous = attr.prev_named_sibling();
            }
            if conditional {
                for i in 0..file.symbols.len() {
                    file.checkpoint()?;
                    let s = &file.symbols[i];
                    if n.start_byte() <= s.span.start_byte && s.span.end_byte <= n.end_byte() {
                        file.attribute(i, "conditional", json!(true))?;
                    }
                }
            }
        }
        if file.language == "rust" && n.kind() == "impl_item" {
            if let Some(ty) = n.child_by_field_name("type") {
                for i in 0..file.symbols.len() {
                    let s = &file.symbols[i];
                    if n.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= n.end_byte()
                        && matches!(s.kind.as_str(), "method" | "function")
                    {
                        file.attribute(i, "receiver_type", json!(text(ty, source)))?;
                    }
                }
            }
            if let (Some(ty), Some(tr)) = (
                n.child_by_field_name("type"),
                n.child_by_field_name("trait"),
            ) {
                let type_name = text(ty, source);
                let trait_name = text(tr, source);
                if let Some(s) = file
                    .symbols
                    .iter()
                    .find(|s| s.name == type_name && s.kind == "struct")
                {
                    file.reference(Reference {
                        source_id: s.symbol_id.clone(),
                        name: trait_name.into(),
                        qualifier: None,
                        span: span(tr),
                        kind: "IMPLEMENTS".into(),
                    })?;
                }
                for i in 0..file.symbols.len() {
                    let s = &file.symbols[i];
                    if n.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= n.end_byte()
                        && matches!(s.kind.as_str(), "method" | "function")
                    {
                        file.attribute(i, "implements_trait", json!(trait_name))?;
                    }
                }
            }
        }
        if matches!(
            n.kind(),
            "assignment_expression" | "assignment" | "augmented_assignment_expression"
        ) && let Some(left) = n
            .child_by_field_name("left")
            .filter(|n| n.kind() == "identifier")
            && let Some(owner) = file
                .symbols
                .iter()
                .filter(|s| s.span.start_byte <= n.start_byte() && n.end_byte() <= s.span.end_byte)
                .min_by_key(|s| s.span.end_byte - s.span.start_byte)
        {
            file.reference(Reference {
                source_id: owner.symbol_id.clone(),
                name: text(left, source).into(),
                qualifier: None,
                span: span(left),
                kind: "WRITES".into(),
            })?;
        }
        if matches!(
            n.kind(),
            "class_heritage" | "extends_clause" | "implements_clause" | "extends_type_clause"
        ) || (file.language == "python"
            && n.kind() == "argument_list"
            && n.parent().is_some_and(|p| p.kind() == "class_definition"))
        {
            let kind = if n.kind() == "implements_clause" {
                "IMPLEMENTS"
            } else {
                "INHERITS"
            };
            let owner = file
                .symbols
                .iter()
                .filter(|s| s.kind == "class" || s.kind == "interface")
                .filter(|s| s.span.start_byte <= n.start_byte() && n.end_byte() <= s.span.end_byte)
                .min_by_key(|s| s.span.end_byte - s.span.start_byte)
                .map(|s| s.symbol_id.clone());
            if let Some(owner) = owner {
                let mut c = n.walk();
                for base in n.named_children(&mut c).filter(|n| {
                    matches!(
                        n.kind(),
                        "identifier" | "type_identifier" | "member_expression" | "attribute"
                    )
                }) {
                    let (q, name) = super::split_name(text(base, source));
                    file.reference(Reference {
                        source_id: owner.clone(),
                        name: name.into(),
                        qualifier: q.map(str::to_owned),
                        span: span(base),
                        kind: kind.into(),
                    })?;
                }
            }
        }
        // Only explicit annotations propagate; initializers and dynamic receivers are not guessed.
        if matches!(
            n.kind(),
            "parameter"
                | "parameter_declaration"
                | "let_declaration"
                | "variable_declarator"
                | "required_parameter"
        ) && let Some(ty) = n.child_by_field_name("type")
        {
            let ty = text(ty, source)
                .trim()
                .trim_start_matches(':')
                .trim()
                .trim_start_matches(['&', '*'])
                .trim()
                .trim_start_matches("dyn ")
                .trim_start_matches("mut ")
                .trim();
            if let Some(name) = n
                .child_by_field_name("name")
                .or_else(|| n.child_by_field_name("pattern"))
            {
                for i in 0..file.symbols.len() {
                    let s = &file.symbols[i];
                    if name.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= name.end_byte()
                        && s.attributes["binding"] == true
                    {
                        file.attribute(i, "declared_type", json!(ty))?;
                    }
                }
            }
        }
        let mut c = n.walk();
        for child in n.named_children(&mut c) {
            stack.push((child, depth + 1));
        }
    }
    Ok(())
}
fn commonjs(n: Node<'_>, source: &str, file: &mut Sink) -> Result<()> {
    if n.kind() == "variable_declarator" {
        let Some(value) = n.child_by_field_name("value") else {
            return Ok(());
        };
        if value.kind() != "call_expression"
            || value
                .child_by_field_name("function")
                .map(|n| text(n, source))
                != Some("require")
        {
            return Ok(());
        }
        // A locally declared require is not the CommonJS loader.
        if file.symbols.iter().any(|s| s.name == "require") {
            return Ok(());
        }
        let Some(args) = value.child_by_field_name("arguments") else {
            return Ok(());
        };
        if args.named_child_count() != 1 {
            return Ok(());
        }
        let Some(arg) = args.named_child(0).filter(|a| a.kind() == "string") else {
            return Ok(());
        };
        let module = super::unquote(text(arg, source));
        let Some(name) = n.child_by_field_name("name") else {
            return Ok(());
        };
        if name.kind() == "identifier" {
            commonjs_binding(file, n, name, source, &module, "*")?;
        } else if name.kind() == "object_pattern" {
            let mut c = name.walk();
            for child in name.named_children(&mut c) {
                if child.kind() == "shorthand_property_identifier_pattern" {
                    commonjs_binding(file, n, child, source, &module, text(child, source))?;
                } else if child.kind() == "pair_pattern"
                    && let (Some(key), Some(value)) = (
                        child.child_by_field_name("key"),
                        child.child_by_field_name("value"),
                    )
                    && value.kind() == "identifier"
                {
                    commonjs_binding(file, n, value, source, &module, text(key, source))?;
                }
            }
        }
    }
    if n.kind() != "assignment_expression" {
        return Ok(());
    }
    let (Some(left), Some(right)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return Ok(());
    };
    let lhs = text(left, source);
    if file.symbols.iter().any(|s| {
        matches!(s.name.as_str(), "exports" | "module") && s.span.start_byte != n.start_byte()
    }) {
        return Ok(());
    }
    if matches!(right.kind(), "function_expression" | "arrow_function") {
        if let Some(local) = file
            .symbols
            .iter()
            .find(|s| s.span.start_byte == n.start_byte() && s.kind == "function")
            .map(|s| s.name.clone())
        {
            if lhs == "module.exports" {
                export(file, &local, "default", true)?;
            } else if let Some(name) = lhs
                .strip_prefix("exports.")
                .or_else(|| lhs.strip_prefix("module.exports."))
            {
                export(file, &local, name, false)?;
            }
        }
    } else if lhs == "module.exports" && right.kind() == "object" {
        let mut c = right.walk();
        for item in right.named_children(&mut c) {
            if item.kind() == "shorthand_property_identifier" {
                export(file, text(item, source), text(item, source), false)?;
            } else if item.kind() == "pair"
                && let (Some(k), Some(v)) = (
                    item.child_by_field_name("key"),
                    item.child_by_field_name("value"),
                )
                && v.kind() == "identifier"
            {
                export(file, text(v, source), text(k, source), false)?;
            }
        }
    } else if right.kind() == "identifier" {
        if lhs == "module.exports" {
            export(file, text(right, source), "default", true)?;
        } else if let Some(name) = lhs
            .strip_prefix("exports.")
            .or_else(|| lhs.strip_prefix("module.exports."))
            && !name.contains('.')
        {
            export(file, text(right, source), name, false)?;
        }
    }
    Ok(())
}
fn commonjs_binding(
    file: &mut Sink,
    n: Node<'_>,
    binding: Node<'_>,
    source: &str,
    module: &str,
    imported: &str,
) -> Result<()> {
    if let Some(i) = file
        .symbols
        .iter()
        .position(|s| s.attributes["name_start"].as_u64() == Some(binding.start_byte() as u64))
    {
        file.attribute(i, "import_binding", json!(true))?;
        let scope = file.symbols[i].owner_id.clone().unwrap_or_default();
        file.import(
            Import {
                local: text(binding, source).into(),
                module: module.into(),
                imported: imported.into(),
            },
            scope,
            span(n),
        )?;
    }
    Ok(())
}
fn export(file: &mut Sink, local: &str, exported: &str, default: bool) -> Result<()> {
    let fid = file
        .symbols
        .iter()
        .find(|s| s.kind == "file")
        .map(|s| s.symbol_id.clone())
        .unwrap_or_default();
    for i in 0..file.symbols.len() {
        file.checkpoint()?;
        let s = &file.symbols[i];
        if s.name == local && s.owner_id.as_deref() == Some(&fid) {
            file.attribute(i, "exported", json!(true))?;
            file.attribute(i, "default_export", json!(default))?;
            file.append_attribute(i, "export_names", json!(exported))?;
        }
    }
    Ok(())
}
