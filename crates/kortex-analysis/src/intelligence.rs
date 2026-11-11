//! Conservative framework adapters. Syntax trees + verified indexed source, never names alone.
use crate::{Service, boolean, number};
use kortex_core::{Error, ParsedFile, Result, Symbol, hash, id};
use kortex_graph::{Direction, Limits};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
use tree_sitter::{Node, ParseOptions, Parser};

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn literal(node: Node<'_>, source: &str) -> Option<String> {
    if !matches!(node.kind(), "string" | "string_literal") {
        return None;
    }
    let s = text(node, source);
    let quote = s.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') || s.len() < 2 || s.as_bytes().last() != Some(&quote) {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    if inner.contains(['\\', '\n', '\r', '\0'])
        || inner.contains("${")
        || inner.contains(quote as char)
    {
        return None;
    }
    Some(inner.into())
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}
fn nodes(root: Node<'_>, deadline: Instant) -> Result<Vec<Node<'_>>> {
    let mut stack = vec![(root, 0)];
    let mut out = vec![];
    while let Some((n, depth)) = stack.pop() {
        if Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "route AST deadline exceeded",
                9,
            ));
        }
        if depth > 256 || out.len() >= 200_000 {
            return Err(Error::new("QUERY_LIMIT", "route AST bounds exceeded", 7));
        }
        out.push(n);
        stack.extend(children(n).into_iter().rev().map(|n| (n, depth + 1)));
    }
    Ok(out)
}
fn method(value: &str) -> Option<&'static str> {
    Some(match value {
        "get" => "GET",
        "post" => "POST",
        "put" => "PUT",
        "patch" => "PATCH",
        "delete" => "DELETE",
        "options" => "OPTIONS",
        "head" => "HEAD",
        _ => return None,
    })
}
struct Adapter<'a> {
    file: &'a ParsedFile,
    source: &'a str,
    factories: BTreeMap<String, String>,
    factory_positions: BTreeMap<String, usize>,
    apps: BTreeMap<String, String>,
    app_positions: BTreeMap<String, usize>,
    bindings: BTreeMap<String, usize>,
}
impl Adapter<'_> {
    fn bind(&mut self, name: &str) {
        *self.bindings.entry(name.into()).or_default() += 1;
    }
    fn imports(&mut self, root: Node<'_>) {
        for n in children(root) {
            if n.kind() == "import_from_statement" {
                let Some(module) = n.child_by_field_name("module_name") else {
                    continue;
                };
                let module = text(module, self.source);
                for item in children(n)
                    .into_iter()
                    .filter(|x| *x != n.child_by_field_name("module_name").unwrap())
                {
                    let original = item.child_by_field_name("name").unwrap_or(item);
                    let local = item.child_by_field_name("alias").unwrap_or(original);
                    let factory = match (module, text(original, self.source)) {
                        ("fastapi", "FastAPI") => Some("fastapi"),
                        ("flask", "Flask") => Some("flask"),
                        _ => None,
                    };
                    self.bind(text(local, self.source));
                    if let Some(f) = factory {
                        self.factories
                            .insert(text(local, self.source).into(), f.into());
                        self.factory_positions
                            .insert(text(local, self.source).into(), n.start_byte());
                    }
                }
            } else if n.kind() == "import_statement" && self.file.language == "python" {
                for item in children(n) {
                    let original = item.child_by_field_name("name").unwrap_or(item);
                    let local = item.child_by_field_name("alias").unwrap_or(original);
                    let module = text(original, self.source);
                    let alias = text(local, self.source);
                    let constructor = match module {
                        "fastapi" => Some("FastAPI"),
                        "flask" => Some("Flask"),
                        _ => None,
                    };
                    self.bind(alias);
                    if let Some(c) = constructor {
                        self.factories.insert(format!("{alias}.{c}"), module.into());
                        self.factory_positions
                            .insert(format!("{alias}.{c}"), n.start_byte());
                    }
                }
            } else if n.kind() == "import_statement" {
                let module = n
                    .child_by_field_name("source")
                    .and_then(|s| literal(s, self.source));
                if let Some(clause) = children(n)
                    .into_iter()
                    .find(|n| n.kind() == "import_clause")
                {
                    for part in children(clause) {
                        if part.kind() == "identifier" {
                            let name = text(part, self.source);
                            self.bind(name);
                            if module.as_deref() == Some("express") {
                                self.factories.insert(name.into(), "express".into());
                                self.factory_positions.insert(name.into(), 0);
                            }
                        } else {
                            for named in children(part) {
                                let name = named
                                    .child_by_field_name("alias")
                                    .or_else(|| named.child_by_field_name("name"))
                                    .unwrap_or(named);
                                if name.kind() == "identifier" {
                                    self.bind(text(name, self.source));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn collect_bindings(&mut self, all: &[Node<'_>]) {
        for n in all {
            if n.kind() == "delete_statement" {
                for name in children(*n)
                    .into_iter()
                    .filter(|n| n.kind() == "identifier")
                {
                    self.bind(text(name, self.source));
                }
            }
            if matches!(n.kind(), "import_statement" | "import_from_statement")
                && n.parent()
                    .is_some_and(|p| !matches!(p.kind(), "module" | "program"))
            {
                // Any nested import binding is conservatively treated as shadowing.
                for item in children(*n) {
                    let name = item
                        .child_by_field_name("alias")
                        .or_else(|| item.child_by_field_name("name"))
                        .unwrap_or(item);
                    if name.kind() == "identifier" {
                        self.bind(text(name, self.source));
                    }
                }
            }
            if matches!(
                n.kind(),
                "assignment"
                    | "augmented_assignment"
                    | "named_expression"
                    | "assignment_expression"
                    | "augmented_assignment_expression"
                    | "variable_declarator"
                    | "for_statement"
                    | "for_in_statement"
                    | "as_pattern"
            ) && let Some(left) = n
                .child_by_field_name("left")
                .or_else(|| n.child_by_field_name("name"))
                .or_else(|| n.child_by_field_name("alias"))
            {
                for part in std::iter::once(left).chain(children(left)) {
                    if part.kind() == "identifier" {
                        self.bind(text(part, self.source));
                    }
                }
            }
            if matches!(
                n.kind(),
                "parameters" | "formal_parameters" | "lambda_parameters"
            ) {
                for p in children(*n) {
                    let name = if p.kind() == "identifier" {
                        Some(p)
                    } else {
                        p.child_by_field_name("name")
                            .or_else(|| p.child_by_field_name("pattern"))
                    };
                    if let Some(name) = name {
                        self.bind(text(name, self.source));
                    } else {
                        for name in children(p).into_iter().filter(|n| n.kind() == "identifier") {
                            self.bind(text(name, self.source));
                        }
                    }
                }
            }
            if matches!(
                n.kind(),
                "function_definition"
                    | "function_declaration"
                    | "class_definition"
                    | "class_declaration"
            ) && let Some(name) = n.child_by_field_name("name")
            {
                self.bind(text(name, self.source));
            }
        }
    }
    fn applications(&mut self, root: Node<'_>) {
        // Only module-level assignments prove the receiver binding; nested construction is unsupported.
        let statements = children(root)
            .into_iter()
            .flat_map(|n| {
                if matches!(
                    n.kind(),
                    "expression_statement" | "lexical_declaration" | "variable_declaration"
                ) {
                    children(n)
                } else {
                    vec![n]
                }
            })
            .collect::<Vec<_>>();
        // Explicit CommonJS factory; require must itself be unshadowed.
        if self.bindings.get("require").copied().unwrap_or(0) == 0 {
            for n in &statements {
                if n.kind() != "variable_declarator" {
                    continue;
                }
                let Some(name) = n.child_by_field_name("name") else {
                    continue;
                };
                let Some(value) = n.child_by_field_name("value") else {
                    continue;
                };
                if value.kind() != "call_expression" {
                    continue;
                }
                if value
                    .child_by_field_name("function")
                    .is_none_or(|f| text(f, self.source) != "require")
                {
                    continue;
                }
                let args = value
                    .child_by_field_name("arguments")
                    .map(children)
                    .unwrap_or_default();
                if args.len() == 1
                    && literal(args[0], self.source).as_deref() == Some("express")
                    && self.bindings.get(text(name, self.source)) == Some(&1)
                {
                    self.factories
                        .insert(text(name, self.source).into(), "express".into());
                    self.factory_positions
                        .insert(text(name, self.source).into(), n.start_byte());
                }
            }
        }
        for n in statements {
            if !matches!(n.kind(), "assignment" | "variable_declarator") {
                continue;
            }
            let Some(name) = n
                .child_by_field_name("left")
                .or_else(|| n.child_by_field_name("name"))
            else {
                continue;
            };
            if name.kind() != "identifier" || self.bindings.get(text(name, self.source)) != Some(&1)
            {
                continue;
            }
            let Some(value) = n
                .child_by_field_name("right")
                .or_else(|| n.child_by_field_name("value"))
            else {
                continue;
            };
            if !matches!(value.kind(), "call" | "call_expression") {
                continue;
            }
            let Some(function) = value.child_by_field_name("function") else {
                continue;
            };
            let full = text(function, self.source);
            let base = full.split('.').next().unwrap_or(full);
            if self.bindings.get(base) != Some(&1) {
                continue;
            }
            if let Some(framework) = self.factories.get(full)
                && self
                    .factory_positions
                    .get(full)
                    .is_some_and(|position| *position < n.start_byte())
            {
                self.apps
                    .insert(text(name, self.source).into(), framework.clone());
                self.app_positions
                    .insert(text(name, self.source).into(), n.start_byte());
            }
        }
    }
}
fn location(file: &ParsedFile, node: Node<'_>) -> Value {
    let p = node.start_position();
    json!({"path":file.path,"line":p.row+1,"column":p.column+1,"start_byte":node.start_byte(),"end_byte":node.end_byte()})
}
struct Registration<'a> {
    framework: &'a str,
    http_method: &'a str,
    path: &'a str,
    handler: &'a Symbol,
}
fn route(
    service: &Service,
    file: &ParsedFile,
    node: Node<'_>,
    registration: Registration<'_>,
    processes: bool,
    limits: &Limits,
) -> Result<Value> {
    let Registration {
        framework,
        http_method,
        path,
        handler,
    } = registration;
    let route_id = id(&[
        &service.snapshot.repo_id,
        &file.path,
        &node.start_byte().to_string(),
        http_method,
        path,
    ]);
    let consumers=service.snapshot.edges.iter().filter(|e| e.kind=="CALLS" && e.target_id.as_deref()==Some(&handler.symbol_id) && matches!(e.resolution.as_str(),"exact"|"inferred")).take(501).map(|e|json!({"symbol_id":e.source_id,"edge_id":e.id,"kind":"direct_handler_call","resolution":e.resolution,"confidence":e.confidence,"evidence":e.evidence,"source_span":e.source_span})).collect::<Vec<_>>();
    let upstream = kortex_graph::traverse_kinds(
        &service.snapshot,
        &handler.symbol_id,
        Direction::Upstream,
        limits,
        &["CALLS", "TESTS"],
    )?;
    // Test identities require explicit framework AST or persisted TESTS evidence.
    let tests = crate::test_map::associations(
        service,
        &upstream.items,
        Instant::now() + std::time::Duration::from_millis(limits.timeout_ms),
    )?;
    let provenance = json!({"adapter_version":"framework-ast-1","source_hash":file.content_hash,"generation_id":service.snapshot.generation_id,"location":location(file,node),"basis":"verified framework import + constructor binding + literal registration + indexed declaration"});
    let mut bounded = upstream.truncated || consumers.len() > 500;
    let mut partial = upstream.ambiguous_relationships > 0 || upstream.unresolved_relationships > 0;
    let process = if processes {
        let downstream = kortex_graph::traverse_kinds(
            &service.snapshot,
            &handler.symbol_id,
            Direction::Downstream,
            limits,
            &["CALLS"],
        )?;
        bounded |= downstream.truncated;
        partial |=
            downstream.ambiguous_relationships > 0 || downstream.unresolved_relationships > 0;
        json!({"process_id":id(&[&route_id,"bounded-call-v1"]),"algorithm":"bounded-call-v1","heuristic":true,"entry_id":handler.symbol_id,"items":downstream.items,"edges":downstream.edges,"ambiguous_relationships":downstream.ambiguous_relationships,"unresolved_relationships":downstream.unresolved_relationships,"partial":partial,"truncated":downstream.truncated,"provenance":provenance})
    } else {
        Value::Null
    };
    Ok(
        json!({"route_id":route_id,"framework":framework,"http_method":http_method,"path":path,"handler_id":handler.symbol_id,"location":location(file,node),"resolution":"exact","confidence":1.0,"provenance":provenance,"consumers":consumers.into_iter().take(500).collect::<Vec<_>>(),"tests":tests,"process":process,"partial":partial,"truncated":bounded}),
    )
}
pub fn discover(service: &Service, args: &Value, deadline: Instant) -> Result<Value> {
    let processes = boolean(args, "include_processes", true)?;
    let limits = service.limits(args)?;
    let framework = args
        .get("framework")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .ok_or_else(|| Error::new("INVALID_INPUT", "framework must be a string", 2))
        })
        .transpose()?;
    if framework.is_some_and(|s| !matches!(s, "fastapi" | "flask" | "express")) {
        return Err(Error::new("INVALID_INPUT", "unsupported framework", 2));
    }
    let prefix = args
        .get("path_prefix")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut items = vec![];
    let mut diagnostics = vec![];
    for file in &service.snapshot.files {
        if !matches!(
            file.language.as_str(),
            "python" | "javascript" | "typescript"
        ) {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "route discovery deadline exceeded",
                9,
            ));
        }
        let source = match kortex_repo::read_confined(
            &service.root,
            &file.path,
            service.config.repository.max_file_size_bytes,
        ) {
            Ok(s) if hash(&s) == file.content_hash => s,
            _ => {
                diagnostics.push(json!({"path":file.path,"message":"indexed source unavailable or changed; route evidence refused"}));
                continue;
            }
        };
        let grammar = match file.language.as_str() {
            "python" => tree_sitter_python::LANGUAGE.into(),
            "typescript" if file.path.ends_with(".tsx") => {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            }
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            _ => tree_sitter_javascript::LANGUAGE.into(),
        };
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|e| Error::new("GRAMMAR_ERROR", e.to_string(), 7))?;
        let bytes = source.as_bytes();
        let mut cancel = |_: &tree_sitter::ParseState| Instant::now() >= deadline;
        let tree = parser
            .parse_with_options(
                &mut |offset, _| &bytes[offset..],
                None,
                Some(ParseOptions::new().progress_callback(&mut cancel)),
            )
            .ok_or_else(|| Error::new("QUERY_TIMEOUT", "route parse deadline exceeded", 9))?;
        if tree.root_node().has_error() {
            diagnostics
                .push(json!({"path":file.path,"message":"syntax errors; framework facts refused"}));
            continue;
        }
        let all = nodes(tree.root_node(), deadline)?;
        if file.imports.iter().any(|i| {
            matches!(
                (i.module.as_str(), i.imported.as_str()),
                ("fastapi", "APIRouter") | ("flask", "Blueprint")
            )
        }) {
            diagnostics.push(json!({"path":file.path,"message":"APIRouter/Blueprint mounting is unsupported; no expanded paths inferred"}));
        }
        let mut adapter = Adapter {
            file,
            source: &source,
            factories: BTreeMap::new(),
            factory_positions: BTreeMap::new(),
            apps: BTreeMap::new(),
            app_positions: BTreeMap::new(),
            bindings: BTreeMap::new(),
        };
        adapter.imports(tree.root_node());
        adapter.factories.retain(|_, framework| {
            let directory = file
                .path
                .rsplit_once('/')
                .map(|p| format!("{}/", p.0))
                .unwrap_or_default();
            let local = [
                format!("{framework}.py"),
                format!("{framework}/__init__.py"),
                format!("{directory}{framework}.py"),
                format!("{directory}{framework}/__init__.py"),
            ];
            !service
                .snapshot
                .files
                .iter()
                .any(|f| local.contains(&f.path))
        });
        adapter.collect_bindings(&all);
        adapter.applications(tree.root_node());
        let mut seen = BTreeSet::new();
        for n in &all {
            if !matches!(n.kind(), "call" | "call_expression") {
                continue;
            }
            let Some(function) = n.child_by_field_name("function") else {
                continue;
            };
            let Some((receiver, verb)) = text(function, &source).rsplit_once('.') else {
                continue;
            };
            let Some(adapter_framework) = adapter.apps.get(receiver) else {
                continue;
            };
            if adapter
                .app_positions
                .get(receiver)
                .is_none_or(|position| *position >= n.start_byte())
            {
                if method(verb).is_some() || verb == "route" {
                    diagnostics.push(json!({"path":file.path,"message":"registration precedes application binding; route refused","location":location(file,*n)}));
                }
                continue;
            }
            if matches!(
                verb,
                "api_route" | "include_router" | "mount" | "register_blueprint" | "use"
            ) {
                diagnostics.push(json!({"path":file.path,"message":"router mounting, middleware and dynamic method adapters are unsupported; no expanded route inferred","location":location(file,*n)}));
                continue;
            }
            let module_registration = if adapter_framework == "express" {
                n.parent().is_some_and(|p| {
                    p.kind() == "expression_statement"
                        && p.parent().is_some_and(|g| g.kind() == "program")
                })
            } else {
                n.parent()
                    .filter(|p| p.kind() == "decorator")
                    .and_then(|p| p.parent())
                    .filter(|p| p.kind() == "decorated_definition")
                    .and_then(|p| p.parent())
                    .is_some_and(|p| p.kind() == "module")
            };
            if !module_registration {
                if method(verb).is_some() || verb == "route" {
                    diagnostics.push(json!({"path":file.path,"message":"nested framework registration is not statically established","location":location(file,*n)}));
                }
                continue;
            }
            if framework.is_some_and(|f| f != adapter_framework) {
                continue;
            }
            let args = n
                .child_by_field_name("arguments")
                .map(children)
                .unwrap_or_default();
            let diagnostic = || json!({"path":file.path,"message":"dynamic or unsupported framework registration; no route inferred","location":location(file,*n)});
            let Some(first) = args.first() else {
                continue;
            };
            let Some(path) =
                literal(*first, &source).filter(|s| s.starts_with('/') && s.len() <= 4096)
            else {
                if method(verb).is_some() || verb == "route" {
                    diagnostics.push(diagnostic());
                }
                continue;
            };
            if !path.starts_with(prefix) {
                continue;
            }
            let mut methods = vec![];
            if let Some(m) = method(verb) {
                methods.push(m.to_owned());
            } else if verb == "route" && adapter_framework == "flask" {
                if let Some(kw) = args.iter().find(|a| {
                    a.kind() == "keyword_argument"
                        && a.child_by_field_name("name")
                            .is_some_and(|name| text(name, &source) == "methods")
                }) {
                    if let Some(value) = kw
                        .child_by_field_name("value")
                        .filter(|v| v.kind() == "list")
                    {
                        let parsed: Option<Vec<_>> = children(value)
                            .into_iter()
                            .map(|val| {
                                literal(val, &source)
                                    .filter(|s| method(&s.to_lowercase()).is_some())
                                    .map(|s| s.to_uppercase())
                            })
                            .collect();
                        if let Some(parsed) = parsed {
                            methods = parsed;
                        } else {
                            diagnostics.push(diagnostic());
                            continue;
                        }
                    }
                    if methods.is_empty() {
                        diagnostics.push(diagnostic());
                        continue;
                    }
                } else {
                    methods.push("GET".into());
                }
            } else {
                if matches!(
                    verb,
                    "route"
                        | "api_route"
                        | "include_router"
                        | "mount"
                        | "register_blueprint"
                        | "use"
                ) {
                    diagnostics.push(diagnostic());
                }
                continue;
            }
            let handler = if adapter_framework == "express" {
                if args.len() != 2 || args[1].kind() != "identifier" {
                    diagnostics.push(diagnostic());
                    continue;
                }
                let name = text(args[1], &source);
                let matches = file
                    .symbols
                    .iter()
                    .filter(|s| {
                        s.name == name
                            && matches!(s.kind.as_str(), "function" | "method" | "test")
                            && s.owner_id.as_deref().is_some_and(|id| {
                                file.symbols
                                    .iter()
                                    .any(|f| f.symbol_id == id && f.kind == "file")
                            })
                    })
                    .collect::<Vec<_>>();
                if matches.len() != 1 || adapter.bindings.get(name) != Some(&1) {
                    diagnostics.push(diagnostic());
                    continue;
                }
                if matches[0].span.start_byte > n.start_byte()
                    && all
                        .iter()
                        .find(|node| node.start_byte() == matches[0].span.start_byte)
                        .is_none_or(|node| {
                            !matches!(
                                node.kind(),
                                "function_declaration" | "generator_function_declaration"
                            )
                        })
                {
                    diagnostics.push(diagnostic());
                    continue;
                }
                Some(matches[0])
            } else {
                let decorated = n
                    .parent()
                    .filter(|n| n.kind() == "decorator")
                    .and_then(|n| n.parent())
                    .filter(|n| n.kind() == "decorated_definition");
                decorated
                    .and_then(|d| {
                        children(d).into_iter().find(|n| {
                            matches!(
                                n.kind(),
                                "function_definition" | "async_function_definition"
                            )
                        })
                    })
                    .and_then(|definition| {
                        file.symbols.iter().find(|s| {
                            s.span.start_byte == definition.start_byte()
                                && matches!(s.kind.as_str(), "function" | "test")
                        })
                    })
            };
            let Some(handler) = handler else {
                diagnostics.push(diagnostic());
                continue;
            };
            for m in methods {
                if seen.insert((n.start_byte(), m.clone())) {
                    items.push(route(
                        service,
                        file,
                        *n,
                        Registration {
                            framework: adapter_framework,
                            http_method: &m,
                            path: &path,
                            handler,
                        },
                        processes,
                        &limits,
                    )?);
                }
            }
        }
    }
    items.sort_by(|a, b| {
        a["path"]
            .as_str()
            .cmp(&b["path"].as_str())
            .then(a["http_method"].as_str().cmp(&b["http_method"].as_str()))
            .then(a["route_id"].as_str().cmp(&b["route_id"].as_str()))
    });
    let truncated = items.iter().any(|v| v["truncated"] == true);
    Ok(
        json!({"schema_version":"1.0","operation":"routes","items":items,"partial":!diagnostics.is_empty() || items.iter().any(|v|v["partial"]==true),"diagnostics":diagnostics,"truncated":truncated}),
    )
}

pub fn paginate(
    service: &Service,
    args: &Value,
    mut data: Value,
) -> Result<(Value, Option<String>)> {
    let limit = number(args, "limit", 20)?;
    if !(1..=500).contains(&limit) {
        return Err(Error::new("QUERY_LIMIT", "limit must be 1..500", 2));
    }
    let offset = crate::contracts::cursor_offset(service, "routes", args)?;
    let items = data["items"].as_array_mut().unwrap();
    let more = items.len() > offset.saturating_add(limit);
    *items = items.iter().skip(offset).take(limit).cloned().collect();
    let cursor = more.then(|| crate::contracts::cursor(service, "routes", args, offset + limit));
    Ok((data, cursor))
}
