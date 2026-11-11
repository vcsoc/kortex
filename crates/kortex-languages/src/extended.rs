//! Namespace/import-aware conservative P1 resolution. Overloads remain candidate sets.
use super::{Lookup, accepts, normalized, owners};
use kortex_core::{ParsedFile, Reference, Symbol};
use std::{collections::BTreeSet, path::Path};
pub(super) fn supports(lang: &str) -> bool {
    matches!(lang, "java" | "csharp" | "c" | "cpp" | "ruby" | "php")
}
fn namespace(file: &ParsedFile) -> &str {
    file.symbols
        .iter()
        .find(|s| s.kind == "file")
        .and_then(|s| s.attributes["namespace"].as_str())
        .unwrap_or("")
}
fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.into()
    } else {
        format!("{prefix}::{name}")
    }
}
pub(super) fn imported_files<'a>(
    snapshot: &Lookup<'a>,
    file: &ParsedFile,
    module: &str,
) -> Vec<&'a ParsedFile> {
    if matches!(file.language.as_str(), "c" | "cpp" | "ruby") {
        let parent = Path::new(&file.path).parent().unwrap_or(Path::new(""));
        let path = normalized(&parent.join(module));
        let mut found: Vec<_> = path
            .iter()
            .flat_map(|p| {
                let mut found: Vec<_> = snapshot.file(p).into_iter().collect();
                if file.language == "ruby" {
                    found.extend(snapshot.file(&format!("{p}.rb")));
                }
                found
            })
            .collect();
        snapshot.ordered(&mut found);
        return found;
    }
    snapshot.extended_module(&file.language, module)
}
pub(super) fn candidates<'a>(
    snapshot: &Lookup<'a>,
    file: &'a ParsedFile,
    r: &Reference,
) -> Vec<&'a Symbol> {
    let source = snapshot.symbol(&r.source_id);
    let scopes = owners(snapshot, source);
    if source.is_some_and(|s| {
        s.attributes["dynamic_sites"]
            .as_array()
            .is_some_and(|sites| {
                sites
                    .iter()
                    .any(|v| v.as_u64() == Some(r.span.start_byte as u64))
            })
    }) {
        return vec![];
    }
    let binding_name = r
        .qualifier
        .as_deref()
        .unwrap_or(&r.name)
        .split("::")
        .next()
        .unwrap_or(&r.name);
    if (r.qualifier.is_some() || r.kind == "CALLS")
        && snapshot.local(file, binding_name).any(|s| {
            s.attributes["binding"] == true
                && super::in_scope(s, r)
                && scopes
                    .iter()
                    .any(|o| s.owner_id.as_deref() == Some(&o.symbol_id))
        })
    {
        return vec![];
    }
    let imports: Vec<_> = file
        .imports
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            file.import_scopes
                .get(*i)
                .is_some_and(|o| scopes.iter().any(|s| &s.symbol_id == o))
        })
        .map(|(_, i)| i)
        .collect();
    let mut visible_files = BTreeSet::from([file.path.as_str()]);
    if matches!(file.language.as_str(), "c" | "cpp" | "ruby") {
        let mut pending = vec![file];
        while let Some(f) = pending.pop() {
            for import in &f.imports {
                for target in super::module_files(snapshot, f, &import.module, None) {
                    if visible_files.insert(&target.path) {
                        pending.push(target);
                    }
                }
            }
        }
    }
    if matches!(file.language.as_str(), "c" | "cpp")
        && visible_files
            .iter()
            .filter_map(|path| snapshot.file(path))
            .any(|f| {
                f.symbols
                    .iter()
                    .find(|s| s.kind == "file")
                    .and_then(|s| s.attributes["macro_bindings"].as_array())
                    .is_some_and(|names| {
                        names
                            .iter()
                            .any(|name| name.as_str() == Some(r.name.as_str()))
                    })
            })
    {
        return vec![];
    }
    let mut names = BTreeSet::new();
    let mut class_receiver = false;
    if let Some(q) = &r.qualifier {
        let current_class = scopes
            .iter()
            .find(|s| matches!(s.kind.as_str(), "class" | "struct"));
        if matches!(q.as_str(), "this" | "self" | "static") {
            if let Some(class) = current_class {
                names.insert(join(&class.qualified_name, &r.name));
            }
        } else {
            class_receiver = true;
            names.insert(join(q, &r.name));
            names.insert(join(namespace(file), &join(q, &r.name)));
            for scope in &scopes {
                if scope.kind == "namespace" {
                    names.insert(join(&scope.qualified_name, &join(q, &r.name)));
                }
            }
            for import in &imports {
                if import.local == *q {
                    names.insert(join(&import.module, &r.name));
                } else if import.local.is_empty() {
                    names.insert(join(&import.module, &join(q, &r.name)));
                }
            }
        }
    } else {
        // Nearest lexical declaration set wins, including all overloads/redefinitions.
        for scope in &scopes {
            let local: Vec<_> = snapshot
                .local(file, &r.name)
                .filter(|s| {
                    s.owner_id.as_deref() == Some(&scope.symbol_id)
                        && accepts(&r.kind, s)
                        && super::in_scope(s, r)
                        && super::available(s, r)
                })
                .collect();
            if !local.is_empty() {
                return local;
            }
        }
        names.insert(join(namespace(file), &r.name));
        for scope in &scopes {
            if matches!(scope.kind.as_str(), "class" | "namespace" | "struct") {
                names.insert(join(&scope.qualified_name, &r.name));
            }
        }
        for import in &imports {
            if import.local == r.name {
                names.insert(import.module.clone());
            } else if import.local.is_empty() {
                names.insert(join(&import.module, &r.name));
            }
        }
    }
    let languages: &[&str] = if matches!(file.language.as_str(), "c" | "cpp") {
        &["c", "cpp"]
    } else {
        &[&file.language]
    };
    let mut found: Vec<_> = names
        .iter()
        .flat_map(|name| {
            languages
                .iter()
                .flat_map(move |language| snapshot.qualified(language, name))
        })
        .filter(|s| {
            !matches!(file.language.as_str(), "c" | "cpp" | "ruby")
                || visible_files.contains(s.file_path.as_str())
        })
        .filter(|s| accepts(&r.kind, s))
        .filter(|s| s.file_path == file.path || s.visibility != "private")
        .filter(|s| {
            !class_receiver
                || r.kind != "CALLS"
                || s.kind != "method"
                || s.attributes["static"] == true
        })
        .collect();
    // C/C++ prototypes and definitions are distinct evidence; a visible definition supersedes its prototype.
    let defined: BTreeSet<_> = found
        .iter()
        .filter(|s| s.attributes["declaration_only"] == false)
        .map(|s| s.qualified_name.as_str())
        .collect();
    found.retain(|s| {
        s.attributes["declaration_only"] != true || !defined.contains(s.qualified_name.as_str())
    });
    found
}
