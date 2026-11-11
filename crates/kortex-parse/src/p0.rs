//! Additional P0 facts derived from grammar nodes, never source-text pattern matching.
use super::{span, text};
use kortex_core::{Import, ParsedFile, Reference};
use serde_json::json;
use tree_sitter::Node;

pub(super) fn finish(root: Node<'_>, source: &str, file: &mut ParsedFile) {
    if file.language == "rust" {
        let indices: Vec<_> = file
            .import_spans
            .iter()
            .enumerate()
            .filter(|(_, s)| source[s.start_byte..s.end_byte].starts_with("pub use "))
            .map(|(i, _)| i)
            .collect();
        if let Some(s) = file.symbols.iter_mut().find(|s| s.kind == "file") {
            s.attributes["rust_reexports"] = json!(indices);
        }
    }
    let mut stack = vec![(root, 0usize)];
    while let Some((n, depth)) = stack.pop() {
        if depth > 256 {
            continue;
        }
        if matches!(file.language.as_str(), "javascript" | "typescript") {
            commonjs(n, source, file);
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
                if let Some(s) = file
                    .symbols
                    .iter_mut()
                    .find(|s| s.attributes["name_start"].as_u64() == Some(name.start_byte() as u64))
                {
                    s.attributes["receiver_type"] = json!(ty);
                }
            }
        }
        if file.language == "go"
            && matches!(n.kind(), "method_declaration" | "method_elem")
            && let Some(parameters) = n.child_by_field_name("parameters")
        {
            let mut types = vec![];
            let mut c = parameters.walk();
            for param in parameters.named_children(&mut c) {
                if let Some(ty) = param.child_by_field_name("type") {
                    let mut c = param.walk();
                    let count = param.children_by_field_name("name", &mut c).count().max(1);
                    types.extend(std::iter::repeat_n(text(ty, source).to_owned(), count));
                }
            }
            let result = n
                .child_by_field_name("result")
                .map(|n| text(n, source))
                .unwrap_or("");
            if let Some(s) = file
                .symbols
                .iter_mut()
                .find(|s| s.span.start_byte == n.start_byte() && s.span.end_byte == n.end_byte())
            {
                s.attributes["method_shape"] = json!([types, result]);
            }
        }
        if file.language == "rust"
            && matches!(n.kind(), "type_parameter" | "where_predicate")
            && let (Some(name), Some(bounds)) = (
                n.child_by_field_name("name")
                    .or_else(|| n.child_by_field_name("left")),
                n.child_by_field_name("bounds"),
            )
        {
            let mut c = bounds.walk();
            let names: Vec<_> = bounds
                .named_children(&mut c)
                .filter(|n| matches!(n.kind(), "type_identifier" | "scoped_type_identifier"))
                .map(|n| text(n, source))
                .collect();
            if let Some(s) = file
                .symbols
                .iter_mut()
                .filter(|s| s.span.start_byte <= n.start_byte() && n.end_byte() <= s.span.end_byte)
                .min_by_key(|s| s.span.end_byte - s.span.start_byte)
            {
                if !s.attributes["trait_bounds"].is_object() {
                    s.attributes["trait_bounds"] = json!({});
                }
                s.attributes["trait_bounds"][text(name, source)] = json!(names);
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
                for s in &mut file.symbols {
                    if n.start_byte() <= s.span.start_byte && s.span.end_byte <= n.end_byte() {
                        s.attributes["conditional"] = json!(true);
                    }
                }
            }
        }
        if file.language == "rust" && n.kind() == "impl_item" {
            if let Some(ty) = n.child_by_field_name("type") {
                for s in &mut file.symbols {
                    if n.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= n.end_byte()
                        && matches!(s.kind.as_str(), "method" | "function")
                    {
                        s.attributes["receiver_type"] = json!(text(ty, source));
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
                    file.references.push(Reference {
                        source_id: s.symbol_id.clone(),
                        name: trait_name.into(),
                        qualifier: None,
                        span: span(tr),
                        kind: "IMPLEMENTS".into(),
                    });
                }
                for s in &mut file.symbols {
                    if n.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= n.end_byte()
                        && matches!(s.kind.as_str(), "method" | "function")
                    {
                        s.attributes["implements_trait"] = json!(trait_name);
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
            file.references.push(Reference {
                source_id: owner.symbol_id.clone(),
                name: text(left, source).into(),
                qualifier: None,
                span: span(left),
                kind: "WRITES".into(),
            });
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
                    file.references.push(Reference {
                        source_id: owner.clone(),
                        name: name.into(),
                        qualifier: q.map(str::to_owned),
                        span: span(base),
                        kind: kind.into(),
                    });
                }
            }
        }
        // Only explicit annotation facts are propagated; initializers and dynamic receivers are not guessed.
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
            let name = n
                .child_by_field_name("name")
                .or_else(|| n.child_by_field_name("pattern"));
            if let Some(name) = name {
                for s in &mut file.symbols {
                    if name.start_byte() <= s.span.start_byte
                        && s.span.end_byte <= name.end_byte()
                        && s.attributes["binding"] == true
                    {
                        s.attributes["declared_type"] = json!(ty);
                    }
                }
            }
        }
        let mut c = n.walk();
        for child in n.named_children(&mut c) {
            stack.push((child, depth + 1));
        }
    }
}
fn commonjs(n: Node<'_>, source: &str, file: &mut ParsedFile) {
    if n.kind() == "variable_declarator" {
        let Some(value) = n.child_by_field_name("value") else {
            return;
        };
        if value.kind() != "call_expression"
            || value
                .child_by_field_name("function")
                .map(|n| text(n, source))
                != Some("require")
        {
            return;
        }
        // A locally declared require is not the CommonJS loader.
        if file.symbols.iter().any(|s| s.name == "require") {
            return;
        }
        let Some(args) = value.child_by_field_name("arguments") else {
            return;
        };
        if args.named_child_count() != 1 {
            return;
        }
        let Some(arg) = args.named_child(0).filter(|a| a.kind() == "string") else {
            return;
        };
        let module = super::unquote(text(arg, source));
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };
        let mut bindings = vec![];
        if name.kind() == "identifier" {
            bindings.push((name, "*".to_owned()));
        } else if name.kind() == "object_pattern" {
            let mut c = name.walk();
            for child in name.named_children(&mut c) {
                if child.kind() == "shorthand_property_identifier_pattern" {
                    bindings.push((child, text(child, source).into()));
                } else if child.kind() == "pair_pattern"
                    && let (Some(key), Some(value)) = (
                        child.child_by_field_name("key"),
                        child.child_by_field_name("value"),
                    )
                    && value.kind() == "identifier"
                {
                    bindings.push((value, text(key, source).into()));
                }
            }
        }
        for (binding, imported) in bindings {
            if let Some(s) = file
                .symbols
                .iter_mut()
                .find(|s| s.attributes["name_start"].as_u64() == Some(binding.start_byte() as u64))
            {
                s.attributes["import_binding"] = json!(true);
                file.imports.push(Import {
                    local: text(binding, source).into(),
                    module: module.clone(),
                    imported,
                });
                file.import_scopes
                    .push(s.owner_id.clone().unwrap_or_default());
                file.import_spans.push(span(n));
            }
        }
    }
    if n.kind() != "assignment_expression" {
        return;
    }
    let (Some(left), Some(right)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    let lhs = text(left, source);
    if file.symbols.iter().any(|s| {
        matches!(s.name.as_str(), "exports" | "module") && s.span.start_byte != n.start_byte()
    }) {
        return;
    }
    if matches!(right.kind(), "function_expression" | "arrow_function") {
        if let Some(local) = file
            .symbols
            .iter()
            .find(|s| s.span.start_byte == n.start_byte() && s.kind == "function")
            .map(|s| s.name.clone())
        {
            if lhs == "module.exports" {
                export(file, &local, "default", true);
            } else if let Some(name) = lhs
                .strip_prefix("exports.")
                .or_else(|| lhs.strip_prefix("module.exports."))
            {
                export(file, &local, name, false);
            }
        }
    } else if lhs == "module.exports" && right.kind() == "object" {
        let mut c = right.walk();
        for item in right.named_children(&mut c) {
            if item.kind() == "shorthand_property_identifier" {
                export(file, text(item, source), text(item, source), false);
            } else if item.kind() == "pair"
                && let (Some(k), Some(v)) = (
                    item.child_by_field_name("key"),
                    item.child_by_field_name("value"),
                )
                && v.kind() == "identifier"
            {
                export(file, text(v, source), text(k, source), false);
            }
        }
    } else if right.kind() == "identifier" {
        if lhs == "module.exports" {
            export(file, text(right, source), "default", true);
        } else if let Some(name) = lhs
            .strip_prefix("exports.")
            .or_else(|| lhs.strip_prefix("module.exports."))
            && !name.contains('.')
        {
            export(file, text(right, source), name, false);
        }
    }
}
fn export(file: &mut ParsedFile, local: &str, exported: &str, default: bool) {
    let fid = file
        .symbols
        .iter()
        .find(|s| s.kind == "file")
        .map(|s| s.symbol_id.clone())
        .unwrap_or_default();
    for s in &mut file.symbols {
        if s.name == local && s.owner_id.as_deref() == Some(&fid) {
            s.attributes["exported"] = json!(true);
            s.attributes["default_export"] = json!(default);
            if !s.attributes["export_names"].is_array() {
                s.attributes["export_names"] = json!([]);
            }
            s.attributes["export_names"]
                .as_array_mut()
                .unwrap()
                .push(json!(exported));
        }
    }
}
