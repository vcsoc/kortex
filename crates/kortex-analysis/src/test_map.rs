//! Test associations require explicit test-framework source or a persisted TESTS edge.
use crate::Service;
use kortex_core::{Result, hash};
use kortex_graph::Reach;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
use tree_sitter::{Node, ParseOptions, Parser};
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn shadowed(root: Node<'_>, alias: &str, source: &str, deadline: Instant) -> Result<bool> {
    let mut imports = 0;
    let mut stack = vec![(root, 0)];
    let mut visited = 0;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if depth > 256 || visited > 200_000 {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Err(kortex_core::Error::new(
                "QUERY_TIMEOUT",
                "test binding deadline exceeded",
                9,
            ));
        }
        if matches!(node.kind(), "import_statement" | "import_from_statement") {
            for item in children(node)
                .into_iter()
                .filter(|n| Some(*n) != node.child_by_field_name("module_name"))
            {
                let name = item
                    .child_by_field_name("alias")
                    .or_else(|| item.child_by_field_name("name"))
                    .unwrap_or(item);
                if text(name, source) == alias {
                    imports += 1;
                }
            }
        }
        if matches!(
            node.kind(),
            "assignment"
                | "augmented_assignment"
                | "named_expression"
                | "for_statement"
                | "as_pattern"
        ) && let Some(left) = node
            .child_by_field_name("left")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("alias"))
            && std::iter::once(left)
                .chain(children(left))
                .any(|n| n.kind() == "identifier" && text(n, source) == alias)
        {
            return Ok(true);
        }
        if matches!(node.kind(), "function_definition" | "class_definition")
            && node
                .child_by_field_name("name")
                .is_some_and(|n| text(n, source) == alias)
        {
            return Ok(true);
        }
        if node.kind() == "parameters"
            && children(node).iter().any(|n| {
                text(*n, source) == alias
                    || children(*n)
                        .iter()
                        .any(|n| n.kind() == "identifier" && text(*n, source) == alias)
            })
        {
            return Ok(true);
        }
        stack.extend(children(node).into_iter().map(|n| (n, depth + 1)));
    }
    Ok(imports != 1)
}
pub fn evidenced_tests(
    service: &Service,
    candidates: &BTreeSet<String>,
    deadline: Instant,
) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for edge in service.snapshot.edges.iter().filter(|e| {
        e.kind == "TESTS"
            && matches!(e.resolution.as_str(), "exact" | "inferred")
            && e.target_id
                .as_deref()
                .is_some_and(|id| service.snapshot.symbol(id).is_some())
            && service.snapshot.symbol(&e.source_id).is_some()
            && !e.evidence.is_empty()
            && !e.resolver.is_empty()
    }) {
        out.insert(edge.source_id.clone(),json!({"framework":"indexed-tests-edge","edge_id":edge.id,"evidence":edge.evidence,"source_span":edge.source_span}));
    }
    for file in &service.snapshot.files {
        if !matches!(file.language.as_str(), "rust" | "python")
            || !file
                .symbols
                .iter()
                .any(|s| candidates.contains(&s.symbol_id))
        {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(kortex_core::Error::new(
                "QUERY_TIMEOUT",
                "test evidence deadline exceeded",
                9,
            ));
        }
        let source = match kortex_repo::read_confined(
            &service.root,
            &file.path,
            service.config.repository.max_file_size_bytes,
        ) {
            Ok(s) if hash(&s) == file.content_hash => s,
            _ => continue,
        };
        let grammar = if file.language == "rust" {
            tree_sitter_rust::LANGUAGE.into()
        } else {
            tree_sitter_python::LANGUAGE.into()
        };
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|e| kortex_core::Error::new("GRAMMAR_ERROR", e.to_string(), 7))?;
        let bytes = source.as_bytes();
        let mut cancel = |_: &tree_sitter::ParseState| Instant::now() >= deadline;
        let tree = parser
            .parse_with_options(
                &mut |offset, _| &bytes[offset..],
                None,
                Some(ParseOptions::new().progress_callback(&mut cancel)),
            )
            .ok_or_else(|| {
                kortex_core::Error::new(
                    "QUERY_TIMEOUT",
                    "test evidence parser deadline exceeded",
                    9,
                )
            })?;
        if tree.root_node().has_error() {
            continue;
        }
        let mut unittest_aliases = vec![];
        let mut testcase_aliases = vec![];
        for n in children(tree.root_node()) {
            if n.kind() == "import_statement" {
                for imported in children(n) {
                    let name = imported.child_by_field_name("name").unwrap_or(imported);
                    let alias = imported.child_by_field_name("alias").unwrap_or(name);
                    if text(name, &source) == "unittest" {
                        unittest_aliases.push(text(alias, &source).to_owned());
                    }
                }
            }
            if n.kind() == "import_from_statement"
                && n.child_by_field_name("module_name")
                    .is_some_and(|m| text(m, &source) == "unittest")
            {
                for imported in children(n) {
                    let name = imported.child_by_field_name("name").unwrap_or(imported);
                    let alias = imported.child_by_field_name("alias").unwrap_or(name);
                    if text(name, &source) == "TestCase" {
                        testcase_aliases.push(text(alias, &source).to_owned());
                    }
                }
            }
        }
        let local_unittest = service.snapshot.files.iter().any(|f| {
            f.path == "unittest.py"
                || f.path == "unittest/__init__.py"
                || file.path.rsplit_once('/').is_some_and(|(dir, _)| {
                    f.path == format!("{dir}/unittest.py")
                        || f.path == format!("{dir}/unittest/__init__.py")
                })
        });
        let mut verified_modules = vec![];
        let mut verified_classes = vec![];
        if !local_unittest {
            for alias in unittest_aliases {
                if !shadowed(tree.root_node(), &alias, &source, deadline)? {
                    verified_modules.push(alias);
                }
            }
            for alias in testcase_aliases {
                if !shadowed(tree.root_node(), &alias, &source, deadline)? {
                    verified_classes.push(alias);
                }
            }
        }
        let mut stack = vec![(tree.root_node(), 0)];
        while let Some((n, depth)) = stack.pop() {
            if depth > 256 {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(kortex_core::Error::new(
                    "QUERY_TIMEOUT",
                    "test AST deadline exceeded",
                    9,
                ));
            }
            let framework = if n.kind() == "function_item"
                && n.prev_named_sibling().is_some_and(|p| {
                    p.kind() == "attribute_item"
                        && text(p, &source).split_whitespace().collect::<String>() == "#[test]"
                }) {
                Some("rust-test-attribute")
            } else if n.kind() == "function_definition" {
                let class = n
                    .parent()
                    .filter(|n| n.kind() == "block")
                    .and_then(|n| n.parent())
                    .filter(|n| n.kind() == "class_definition");
                let is_testcase = class
                    .and_then(|n| n.child_by_field_name("superclasses"))
                    .is_some_and(|bases| {
                        children(bases).iter().any(|b| {
                            let s = text(*b, &source);
                            verified_classes.iter().any(|a| a == s)
                                || verified_modules
                                    .iter()
                                    .any(|a| format!("{a}.TestCase") == s)
                        })
                    });
                if is_testcase
                    && n.child_by_field_name("name")
                        .is_some_and(|name| text(name, &source).starts_with("test"))
                {
                    Some("unittest-testcase-method")
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(framework) = framework
                && let Some(symbol) = file
                    .symbols
                    .iter()
                    .find(|s| s.span.start_byte == n.start_byte() && s.kind != "file")
            {
                out.insert(symbol.symbol_id.clone(),json!({"framework":framework,"source_hash":file.content_hash,"path":file.path,"line":n.start_position().row+1,"start_byte":n.start_byte(),"end_byte":n.end_byte(),"basis":if framework=="rust-test-attribute"{"explicit #[test] AST attribute"}else{"explicit imported unittest.TestCase base + framework discovery method"}}));
            }
            stack.extend(children(n).into_iter().rev().map(|n| (n, depth + 1)));
        }
    }
    Ok(out)
}
pub fn associations(service: &Service, items: &[Reach], deadline: Instant) -> Result<Vec<Value>> {
    let candidates = items.iter().map(|r| r.symbol_id.clone()).collect();
    let tests = evidenced_tests(service, &candidates, deadline)?;
    Ok(items.iter().filter_map(|reach|tests.get(&reach.symbol_id).map(|evidence|json!({"symbol_id":reach.symbol_id,"depth":reach.depth,"confidence":reach.confidence,"edge_path":reach.edge_path,"test_evidence":evidence}))).collect())
}
