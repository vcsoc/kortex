//! Explicit receiver facts only. Interface dispatch returns candidates, never a guessed exact call.
use super::{Lookup, in_scope, owners};
use kortex_core::{ParsedFile, Reference, Symbol};
use std::path::Path;
fn same_go_package(a: &ParsedFile, b: &ParsedFile) -> bool {
    Path::new(&a.path).parent() == Path::new(&b.path).parent()
        && a.symbols.first().map(|s| &s.attributes["package"])
            == b.symbols.first().map(|s| &s.attributes["package"])
}
fn type_targets<'a>(
    snapshot: &Lookup<'a>,
    file: &'a ParsedFile,
    ty: &str,
    go_module: Option<&str>,
) -> Vec<&'a Symbol> {
    let (qualifier, name) = super::split_type(ty);
    let mut result: Vec<_> = snapshot
        .local(file, name)
        .filter(|s| matches!(s.kind.as_str(), "struct" | "trait" | "interface"))
        .collect();
    if file.language == "go" && qualifier.is_none() {
        result.extend(
            snapshot
                .directory(Path::new(&file.path).parent().unwrap_or(Path::new("")))
                .filter(|f| f.path != file.path && same_go_package(f, file))
                .flat_map(|f| snapshot.local(f, name))
                .filter(|s| matches!(s.kind.as_str(), "struct" | "interface")),
        );
    }
    for import in &file.imports {
        if import.local == name || qualifier == Some(import.local.as_str()) {
            result.extend(super::module_candidates(
                snapshot,
                file,
                import,
                if import.imported == "*" {
                    name
                } else {
                    &import.imported
                },
                go_module,
            ));
        }
    }
    result
}
pub(super) fn candidates<'a>(
    snapshot: &Lookup<'a>,
    file: &'a ParsedFile,
    r: &Reference,
    go_module: Option<&str>,
) -> Option<(Vec<&'a Symbol>, bool)> {
    if !matches!(file.language.as_str(), "go" | "rust") || r.kind != "CALLS" {
        return None;
    }
    let q = r.qualifier.as_deref()?;
    let scopes = owners(snapshot, snapshot.symbol(&r.source_id));
    let ty = if q == "self" {
        scopes
            .iter()
            .find_map(|s| s.attributes["receiver_type"].as_str())?
    } else {
        let binding = snapshot
            .local(file, q)
            .filter(|s| {
                s.attributes["binding"] == true
                    && in_scope(s, r)
                    && scopes
                        .iter()
                        .any(|o| s.owner_id.as_deref() == Some(&o.symbol_id))
            })
            .max_by_key(|s| s.attributes["scope_start"].as_u64().unwrap_or(0))?;
        binding.attributes["declared_type"].as_str()?
    };
    let mut types = type_targets(snapshot, file, ty, go_module);
    for scope in &scopes {
        if let Some(bounds) = scope.attributes["trait_bounds"][ty].as_array() {
            for bound in bounds.iter().filter_map(|b| b.as_str()) {
                types.extend(type_targets(snapshot, file, bound, go_module));
            }
        }
    }
    let dynamic = types
        .iter()
        .any(|s| matches!(s.kind.as_str(), "trait" | "interface"));
    let mut result = vec![];
    for method in snapshot
        .named(&file.language, &r.name)
        .filter(|s| s.attributes["receiver_type"].is_string())
    {
        let Some(f) = snapshot.file(&method.file_path) else {
            continue;
        };
        {
            let receiver = method.attributes["receiver_type"].as_str().unwrap_or("");
            if dynamic {
                if file.language == "rust" {
                    if let Some(trait_name) = method.attributes["implements_trait"].as_str() {
                        let implemented = type_targets(snapshot, f, trait_name, go_module);
                        if types
                            .iter()
                            .any(|t| implemented.iter().any(|i| i.symbol_id == t.symbol_id))
                        {
                            result.push(method);
                        }
                    }
                } else {
                    // Reject method sets whose explicit signatures cannot implement the interface.
                    // Remaining structural candidates are still uncertain, including a singleton.
                    for interface in types.iter().filter(|s| s.kind == "interface") {
                        let required: Vec<_> = snapshot
                            .children(&interface.symbol_id)
                            .filter(|s| s.kind == "method")
                            .collect();
                        if !required.is_empty()
                            && required.iter().all(|required| {
                                snapshot
                                    .directory(Path::new(&f.path).parent().unwrap_or(Path::new("")))
                                    .filter(|g| same_go_package(g, f))
                                    .flat_map(|g| snapshot.local(g, &required.name))
                                    .any(|s| {
                                        s.attributes["receiver_type"].as_str() == Some(receiver)
                                            && s.attributes["method_shape"]
                                                == required.attributes["method_shape"]
                                    })
                            })
                        {
                            result.push(method);
                            break;
                        }
                    }
                }
            } else if types.iter().any(|t| {
                t.name == receiver
                    && snapshot.file(&t.file_path).is_some_and(|decl| {
                        if file.language == "go" {
                            same_go_package(decl, f)
                        } else {
                            decl.path == f.path
                        }
                    })
            }) {
                result.push(method);
            }
        }
    }
    Some((result, dynamic))
}
