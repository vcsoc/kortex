//! Conservative lexical/import resolution. Never link unrelated same-name declarations.
use kortex_core::{Edge, Import, ParsedFile, Reference, Snapshot, Symbol, id};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

mod bounds;
mod context;
pub use bounds::ResolutionLimits;
mod extended;
mod lookup;
use lookup::Lookup;
mod receivers;
mod relationships;
mod rust_modules;
pub use context::{ResolutionContext, resolve_bounded_with_context, resolve_with_context};
fn split_type(name: &str) -> (Option<&str>, &str) {
    name.rsplit_once("::")
        .or_else(|| name.rsplit_once('.'))
        .map(|(a, b)| (Some(a), b))
        .unwrap_or((None, name))
}
/// Trusted compatibility API without aggregate limits. Use the bounded API for repositories.
pub fn resolve(snapshot: &Snapshot, go_module: Option<&str>) -> Vec<Edge> {
    resolve_impl(snapshot, go_module, &mut bounds::Budget::new(None))
        .expect("unbounded resolution has no admission failures")
}
fn resolve_impl(
    snapshot: &Snapshot,
    go_module: Option<&str>,
    budget: &mut bounds::Budget<'_>,
) -> kortex_core::Result<Vec<Edge>> {
    let lookup = Lookup::new(snapshot);
    let snapshot = &lookup;
    let mut edges = Vec::new();
    for file in &snapshot.files {
        for s in &file.symbols {
            if let Some(owner) = &s.owner_id {
                edges.push(budget.edge(
                    snapshot,
                    owner,
                    Some(&s.symbol_id),
                    "DECLARES",
                    s.span.clone(),
                    "exact",
                    &[],
                    "lexical declaration",
                )?);
            }
        }
        for (index, import) in file.imports.iter().enumerate() {
            for target in module_files(snapshot, file, &import.module, go_module) {
                if let Some(s) = target.symbols.iter().find(|s| s.kind == "file") {
                    edges.push(
                        budget.edge(
                            snapshot,
                            file.symbols
                                .iter()
                                .find(|s| s.kind == "file")
                                .map(|s| s.symbol_id.as_str())
                                .unwrap_or(""),
                            Some(&s.symbol_id),
                            "IMPORTS",
                            file.import_spans.get(index).cloned().unwrap_or_else(|| {
                                file.symbols
                                    .iter()
                                    .find(|s| s.kind == "file")
                                    .map(|s| s.span.clone())
                                    .unwrap_or_default()
                            }),
                            "exact",
                            &[],
                            &format!("import {}", import.module),
                        )?,
                    );
                }
            }
        }
        for reference in &file.references {
            let mut candidates: Vec<&Symbol> = vec![];
            let source = snapshot.symbol(&reference.source_id);
            let scopes: Vec<_> = owners(snapshot, source)
                .into_iter()
                .filter(|s| file.language != "python" || s.kind != "class")
                .collect();
            let visible_imports: Vec<_> = file
                .imports
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    file.import_scopes
                        .get(*i)
                        .is_some_and(|owner| scopes.iter().any(|s| s.symbol_id == *owner))
                })
                .map(|(_, i)| i)
                .collect();
            let binding_name = reference
                .qualifier
                .as_deref()
                .unwrap_or(&reference.name)
                .split(['.', ':'])
                .next()
                .unwrap_or(&reference.name);
            let generic_type = reference.kind == "USES_TYPE"
                && reference.qualifier.is_none()
                && scopes.iter().any(|scope| {
                    scope.attributes["type_bindings"]
                        .as_array()
                        .is_some_and(|names| {
                            names.iter().any(|name| name.as_str() == Some(binding_name))
                        })
                });
            let unsupported_binding = scopes.iter().any(|scope| {
                scope.attributes["unsupported_bindings"]
                    .as_array()
                    .is_some_and(|names| {
                        names.iter().any(|name| name.as_str() == Some(binding_name))
                    })
            });
            let runtime_receiver = source.is_some_and(|s| {
                ["runtime_receiver_sites", "dynamic_sites"]
                    .iter()
                    .any(|key| {
                        s.attributes[key].as_array().is_some_and(|sites| {
                            sites
                                .iter()
                                .any(|site| site.as_u64() == Some(reference.span.start_byte as u64))
                        })
                    })
            });
            let mutated = (reference.kind == "CALLS" || reference.qualifier.is_some())
                && scopes.iter().any(|scope| {
                    scope.attributes["mutated_bindings"]
                        .as_array()
                        .is_some_and(|bindings| {
                            bindings.iter().any(|binding| {
                                binding["name"].as_str() == Some(binding_name)
                                    && binding["start"].as_u64().is_some_and(|start| {
                                        start as usize <= reference.span.start_byte
                                    })
                                    && binding["end"]
                                        .as_u64()
                                        .is_some_and(|end| reference.span.start_byte < end as usize)
                            })
                        })
                });
            let shadowed = mutated
                || runtime_receiver
                || generic_type
                || unsupported_binding
                || snapshot.local(file, binding_name).any(|s| {
                    s.attributes["binding"] == true
                        && s.attributes["import_binding"] != true
                        && scopes
                            .iter()
                            .any(|scope| s.owner_id.as_deref() == Some(&scope.symbol_id))
                        && in_scope(s, reference)
                        && (reference.qualifier.is_some() || reference.kind == "CALLS")
                        && (file.language == "python" || available(s, reference))
                });
            let receiver_candidates = receivers::candidates(snapshot, file, reference, go_module);
            let dynamic_dispatch = receiver_candidates
                .as_ref()
                .is_some_and(|(_, dynamic)| *dynamic);
            if extended::supports(&file.language) {
                if !shadowed {
                    candidates = extended::candidates(snapshot, file, reference);
                }
            } else if let Some((receiver_targets, _)) = receiver_candidates {
                candidates = receiver_targets;
            } else if !shadowed {
                if let Some(q) = &reference.qualifier {
                    let imported: Vec<_> = visible_imports
                        .iter()
                        .filter(|i| i.local == *q && i.imported == "*")
                        .collect();
                    if !imported.is_empty() {
                        for import in imported {
                            candidates.extend(module_candidates(
                                snapshot,
                                file,
                                import,
                                &reference.name,
                                go_module,
                            ));
                        }
                    } else if file.language == "rust" {
                        let (module, receiver) = if q.starts_with("crate::")
                            || q.starts_with("self::")
                            || q.starts_with("super::")
                            || q == "crate"
                            || file
                                .symbols
                                .iter()
                                .any(|s| s.kind == "module" && s.name == *q)
                        {
                            (q.as_str(), None)
                        } else {
                            ("", Some(q.as_str()))
                        };
                        if let Some(receiver) = receiver {
                            for import in visible_imports.iter().filter(|i| i.local == receiver) {
                                let namespace = Import {
                                    local: import.local.clone(),
                                    module: format!("{}::{}", import.module, import.imported),
                                    imported: "*".into(),
                                };
                                candidates.extend(module_candidates(
                                    snapshot,
                                    file,
                                    &namespace,
                                    &reference.name,
                                    go_module,
                                ));
                            }
                            candidates.extend(snapshot.local(file, &reference.name).filter(|s| {
                                !matches!(s.kind.as_str(), "field" | "property")
                                    && s.qualified_name == format!("{receiver}::{}", reference.name)
                            }));
                        } else {
                            for f in module_files(snapshot, file, module, go_module) {
                                candidates.extend(snapshot.local(f, &reference.name).filter(|s| {
                                    top_level(f, s)
                                        && (f.path == file.path || s.visibility == "public")
                                }));
                            }
                        }
                    }
                } else {
                    let mut local_found = false;
                    for scope in owners(snapshot, source) {
                        if scope.kind == "class"
                            && matches!(
                                file.language.as_str(),
                                "python" | "javascript" | "typescript"
                            )
                        {
                            continue;
                        }
                        candidates = snapshot
                            .local(file, &reference.name)
                            .filter(|s| {
                                s.owner_id.as_deref() == Some(&scope.symbol_id)
                                    && s.kind != "method"
                                    && s.attributes["import_binding"] != true
                                    && in_scope(s, reference)
                                    && accepts(&reference.kind, s)
                                    && (!matches!(file.language.as_str(), "rust" | "go")
                                        || available(s, reference))
                                    && (scope.kind != "file" || !s.qualified_name.contains("::"))
                            })
                            .collect();
                        if !candidates.is_empty() {
                            local_found = true;
                            // The narrowest containing block wins before declaration-order shadowing.
                            let nearest = candidates
                                .iter()
                                .filter_map(|s| s.attributes["scope_start"].as_u64())
                                .max();
                            candidates.retain(|s| {
                                nearest.is_none_or(|at| {
                                    s.attributes["scope_start"].as_u64().unwrap_or(0) == at
                                })
                            });
                            // Value bindings shadow earlier declarations, never unrelated same-name symbols.
                            let latest = candidates
                                .iter()
                                .filter(|s| {
                                    s.attributes["binding"] == true && available(s, reference)
                                })
                                .map(|s| s.span.start_byte)
                                .max();
                            candidates.retain(|s| {
                                available(s, reference)
                                    && latest.is_none_or(|at| {
                                        s.attributes["binding"] != true || s.span.start_byte == at
                                    })
                            });
                            break;
                        }
                        // A recursive call is a binding in its parent's scope, found on the next iteration.
                    }
                    if candidates.is_empty() && !local_found {
                        for import in visible_imports.iter().filter(|i| {
                            (i.local == reference.name
                                && (i.imported != "*"
                                    || file.symbols.iter().any(|s| {
                                        s.name == i.local && s.attributes["import_binding"] == true
                                    })))
                                || (file.language == "rust" && i.imported == "*")
                        }) {
                            candidates.extend(module_candidates(
                                snapshot,
                                file,
                                import,
                                if import.imported == "*" {
                                    if file.language == "rust" {
                                        &reference.name
                                    } else {
                                        "default"
                                    }
                                } else {
                                    &import.imported
                                },
                                go_module,
                            ));
                        }
                    }
                    if candidates.is_empty() && !local_found && file.language == "go" {
                        for f in module_files(snapshot, file, "", go_module) {
                            candidates.extend(
                                snapshot
                                    .local(f, &reference.name)
                                    .filter(|s| top_level(f, s)),
                            );
                        }
                    }
                }
            }
            if !shadowed && file.language == "rust" && reference.qualifier.is_some() {
                candidates.extend(rust_modules::qualified(snapshot, file, reference));
            }
            candidates.retain(|s| accepts(&reference.kind, s));
            let unique: BTreeSet<_> = candidates.iter().map(|s| s.symbol_id.as_str()).collect();
            budget.memberships(unique.len())?;
            let candidates: Vec<_> = unique.into_iter().collect();
            let uncertain = candidates.iter().any(|id| {
                snapshot.symbol(id).is_some_and(|s| {
                    s.attributes["conditional"] == true
                        || s.attributes["syntax_error"] == true
                        || (s.language == "rust" && rust_modules::conditional(snapshot, s))
                })
            }) || file
                .diagnostics
                .iter()
                .any(|d| d.contains("syntax errors") || d.contains("limit"));
            let resolution = if candidates.is_empty() {
                "unresolved"
            } else if candidates.len() > 1 || dynamic_dispatch {
                "ambiguous"
            } else if uncertain {
                "inferred"
            } else {
                "exact"
            };
            let target = if candidates.len() == 1 && !dynamic_dispatch {
                Some(candidates[0])
            } else {
                None
            };
            edges.push(budget.edge(
                snapshot,
                &reference.source_id,
                target,
                &reference.kind,
                reference.span.clone(),
                resolution,
                if candidates.len() > 1 || dynamic_dispatch {
                    &candidates
                } else {
                    &[]
                },
                &format!(
                    "{}:{}: {}{}{}{}",
                    file.path,
                    reference.span.start_line,
                    reference.qualifier.as_deref().unwrap_or(""),
                    if reference.qualifier.is_some() {
                        "::"
                    } else {
                        ""
                    },
                    reference.name,
                    if dynamic_dispatch {
                        " (interface/trait dispatch candidates; runtime implementation unknown)"
                    } else if runtime_receiver && candidates.is_empty() {
                        " (runtime receiver type unknown)"
                    } else if unsupported_binding {
                        " (global/nonlocal binding unsupported)"
                    } else if generic_type {
                        " (generic type parameter; concrete type unknown)"
                    } else if shadowed {
                        " (local binding or assignment; receiver/callable type unknown)"
                    } else {
                        ""
                    }
                ),
            )?);
        }
    }
    relationships::complete(snapshot, &mut edges, budget)?;
    // Derive only from original semantic edges; no whole-graph clone is necessary.
    let semantic_count = edges.len();
    for index in 0..semantic_count {
        let e = &edges[index];
        let derived = match e.kind.as_str() {
            "DECLARES" => Some("CONTAINS"),
            "IMPORTS" => Some("DEPENDS_ON"),
            "REFERENCES"
                if e.target_id
                    .as_deref()
                    .and_then(|id| snapshot.symbol(id))
                    .is_some_and(|s| accepts("READS", s)) =>
            {
                Some("READS")
            }
            _ => None,
        };
        if let Some(kind) = derived {
            let candidate_ids: Vec<_> = e.candidates.iter().map(String::as_str).collect();
            budget.memberships(candidate_ids.len())?;
            let derived = budget.edge(
                snapshot,
                &e.source_id,
                e.target_id.as_deref(),
                kind,
                e.source_span.clone(),
                &e.resolution,
                &candidate_ids,
                &e.evidence,
            )?;
            edges.push(derived);
        }
    }
    for file in &snapshot.files {
        if let Some(owner) = file.symbols.iter().find(|s| s.kind == "file") {
            for s in file
                .symbols
                .iter()
                .filter(|s| s.attributes["exported"] == true)
            {
                edges.push(budget.edge(
                    snapshot,
                    &owner.symbol_id,
                    Some(&s.symbol_id),
                    "EXPORTS",
                    s.span.clone(),
                    "exact",
                    &[],
                    "explicit export declaration or CommonJS assignment",
                )?);
            }
        }
    }
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    edges.dedup_by(|a, b| a.id == b.id);
    Ok(edges)
}
fn accepts(kind: &str, s: &Symbol) -> bool {
    match kind {
        "CALLS" => matches!(
            s.kind.as_str(),
            "function" | "method" | "constructor" | "test" | "class"
        ),
        "USES_TYPE" | "INHERITS" | "IMPLEMENTS" => matches!(
            s.kind.as_str(),
            "class" | "struct" | "enum" | "trait" | "interface" | "type_alias"
        ),
        "READS" | "WRITES" => matches!(
            s.kind.as_str(),
            "variable" | "constant" | "field" | "property"
        ),
        "REFERENCES" => !matches!(
            s.kind.as_str(),
            "file" | "import" | "export" | "trait" | "interface" | "type_alias"
        ),
        _ => false,
    }
}
fn in_scope(s: &Symbol, r: &Reference) -> bool {
    match (
        s.attributes["scope_start"].as_u64(),
        s.attributes["scope_end"].as_u64(),
    ) {
        (Some(start), Some(end)) => {
            start as usize <= r.span.start_byte && r.span.start_byte < end as usize
        }
        _ => true,
    }
}
fn available(s: &Symbol, r: &Reference) -> bool {
    s.attributes["binding"] != true
        || s.attributes["parameter"] == true
        || s.span.start_byte <= r.span.start_byte
}
fn owners<'a>(snapshot: &Lookup<'a>, start: Option<&'a Symbol>) -> Vec<&'a Symbol> {
    let mut result = vec![];
    let mut current = start;
    while let Some(s) = current {
        if result.len() > 256 {
            break;
        }
        result.push(s);
        current = s.owner_id.as_deref().and_then(|i| snapshot.symbol(i));
    }
    result
}
fn top_level(file: &ParsedFile, s: &Symbol) -> bool {
    s.kind != "method"
        && !s.qualified_name.contains("::")
        && file
            .symbols
            .iter()
            .any(|f| f.kind == "file" && s.owner_id.as_deref() == Some(&f.symbol_id))
}
fn module_candidates<'a>(
    snapshot: &Lookup<'a>,
    file: &ParsedFile,
    import: &Import,
    name: &str,
    go_module: Option<&str>,
) -> Vec<&'a Symbol> {
    let key = (
        file.path.clone(),
        import.module.clone(),
        name.to_owned(),
        go_module.map(str::to_owned),
    );
    if let Some(found) = snapshot.candidates.borrow().get(&key) {
        return found.clone();
    }
    let found = if file.language == "rust" {
        rust_modules::imported(snapshot, file, import, name)
    } else {
        module_files(snapshot, file, &import.module, go_module)
            .into_iter()
            .flat_map(|f| {
                f.symbols.iter().filter(move |s| {
                    let name_matches = if name == "default" {
                        s.attributes["default_export"].as_bool() == Some(true)
                    } else {
                        s.name == name
                            || s.attributes["export_names"]
                                .as_array()
                                .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(name)))
                    };
                    let export_visible =
                        !matches!(f.language.as_str(), "javascript" | "typescript")
                            || s.attributes["exported"].as_bool() == Some(true);
                    name_matches
                        && top_level(f, s)
                        && export_visible
                        && (f.language != "rust" || f.path == file.path || s.visibility == "public")
                        && (f.language != "go"
                            || s.name.chars().next().is_some_and(char::is_uppercase))
                })
            })
            .collect()
    };
    snapshot.candidates.borrow_mut().insert(key, found.clone());
    found
}
fn normalized(path: &Path) -> Option<String> {
    let mut p = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(n) => p.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                if !p.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(kortex_core::public_path(&p))
}
fn module_files<'a>(
    snapshot: &Lookup<'a>,
    file: &ParsedFile,
    module: &str,
    go_module: Option<&str>,
) -> Vec<&'a ParsedFile> {
    let key = (
        file.path.clone(),
        module.to_owned(),
        go_module.map(str::to_owned),
    );
    if let Some(found) = snapshot.modules.borrow().get(&key) {
        return found.clone();
    }
    let parent = Path::new(&file.path).parent().unwrap_or(Path::new(""));
    let mut paths = Vec::new();
    match file.language.as_str() {
        "rust" => {
            let parts: Vec<_> = module.split("::").collect();
            let mut base = PathBuf::new();
            let mut at = 0;
            if parts.first() == Some(&"crate") {
                if file.path.starts_with("src/") {
                    base.push("src");
                }
                at = 1;
            } else {
                base.push(parent);
                if parts.first() == Some(&"self") {
                    at = 1;
                } else if !["lib.rs", "main.rs", "mod.rs"]
                    .iter()
                    .any(|p| file.path.ends_with(p))
                {
                    base.push(Path::new(&file.path).file_stem().unwrap_or_default());
                }
            }
            for part in &parts[at..] {
                if *part == "super" {
                    base.pop();
                } else {
                    base.push(part);
                }
            }
            if let Some(p) = normalized(&base) {
                paths.extend([
                    format!("{p}.rs"),
                    format!("{p}/mod.rs"),
                    format!("{p}/lib.rs"),
                    format!("{p}/main.rs"),
                ]);
            }
        }
        "python" => {
            let dots = module.chars().take_while(|c| *c == '.').count();
            let mut base = if dots > 0 {
                parent.to_path_buf()
            } else {
                PathBuf::new()
            };
            for _ in 1..dots {
                base.pop();
            }
            let module = module.trim_start_matches('.').replace('.', "/");
            base.push(module);
            if let Some(p) = normalized(&base) {
                paths.extend([format!("{p}.py"), format!("{p}/__init__.py")]);
            }
        }
        "typescript" | "javascript" => {
            if module.starts_with('.')
                && let Some(p) = normalized(&parent.join(module))
            {
                paths.push(p.clone());
                for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
                    paths.push(format!("{p}.{ext}"));
                    paths.push(format!("{p}/index.{ext}"));
                }
            }
        }
        "go" => {
            if module.is_empty() {
                paths.extend(
                    snapshot
                        .directory(parent)
                        .filter(|f| {
                            f.symbols
                                .iter()
                                .find(|s| s.kind == "file")
                                .map(|s| &s.attributes["package"])
                                == file
                                    .symbols
                                    .iter()
                                    .find(|s| s.kind == "file")
                                    .map(|s| &s.attributes["package"])
                        })
                        .map(|f| f.path.clone()),
                );
            } else if let Some(prefix) = go_module
                && let Some(relative) = module.strip_prefix(&format!("{prefix}/"))
            {
                paths.extend(
                    snapshot
                        .directory(Path::new(relative))
                        .map(|f| f.path.clone()),
                );
            }
        }
        _ if extended::supports(&file.language) => {
            let found = extended::imported_files(snapshot, file, module);
            snapshot.modules.borrow_mut().insert(key, found.clone());
            return found;
        }
        _ => {}
    }
    let mut found: Vec<_> = paths
        .iter()
        .filter_map(|path| snapshot.file(path))
        .filter(|f| f.language == file.language)
        .collect();
    snapshot.ordered(&mut found);
    snapshot.modules.borrow_mut().insert(key, found.clone());
    found
}
#[allow(clippy::too_many_arguments)]
fn edge(
    snapshot: &Snapshot,
    source: &str,
    target: Option<&str>,
    kind: &str,
    span: kortex_core::Span,
    resolution: &str,
    candidates: Vec<String>,
    evidence: &str,
) -> Edge {
    Edge {
        id: id(&[
            &snapshot.repo_id,
            source,
            target.unwrap_or(""),
            kind,
            &span.start_byte.to_string(),
            evidence,
        ]),
        repo_id: snapshot.repo_id.clone(),
        source_id: source.into(),
        target_id: target.map(str::to_owned),
        kind: kind.into(),
        source_span: span,
        resolution: resolution.into(),
        confidence: match resolution {
            "exact" => 1.0,
            "inferred" => 0.65,
            "ambiguous" => 0.0,
            _ => 0.0,
        },
        resolver: "kortex-lexical-import-v1".into(),
        evidence: evidence.into(),
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_traversal_normalization() {
        assert_eq!(normalized(Path::new("a/../b")).unwrap(), "b");
        assert!(normalized(Path::new("../../b")).is_none());
    }
}
