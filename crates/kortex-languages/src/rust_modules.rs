//! Rust file/inline module paths and bounded public re-export traversal.
use super::Lookup;
use kortex_core::{Import, ParsedFile, Reference, Snapshot, Symbol};
use std::collections::{BTreeSet, HashMap};

pub(super) struct Index<'a> {
    symbols: HashMap<String, Vec<&'a Symbol>>,
    reexports: HashMap<String, Vec<&'a Import>>,
    conditional_prefixes: Vec<String>,
}
impl<'a> Index<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Self {
        let mut index = Self {
            symbols: HashMap::new(),
            reexports: HashMap::new(),
            conditional_prefixes: vec![],
        };
        let mut symbols = HashMap::new();
        for s in snapshot.symbols() {
            symbols.entry(s.symbol_id.as_str()).or_insert(s);
        }
        for f in snapshot.files.iter().filter(|f| f.language == "rust") {
            for s in &f.symbols {
                let name = canonical(f, s);
                if s.kind == "module" && s.attributes["conditional"] == true {
                    index.conditional_prefixes.push(format!("{name}::"));
                }
                index.symbols.entry(name).or_default().push(s);
            }
            for (i, import) in f.imports.iter().enumerate() {
                let public = f
                    .symbols
                    .iter()
                    .find(|s| s.kind == "file")
                    .and_then(|s| s.attributes["rust_reexports"].as_array())
                    .is_some_and(|items| items.iter().any(|v| v.as_u64() == Some(i as u64)));
                if !public {
                    continue;
                }
                let owner = f
                    .import_scopes
                    .get(i)
                    .and_then(|id| symbols.get(id.as_str()))
                    .copied();
                let owner_module = owner
                    .filter(|s| s.kind == "module")
                    .map(|s| s.qualified_name.as_str())
                    .unwrap_or("");
                let base = join(&file_module(f), owner_module);
                index.reexports.entry(base).or_default().push(import);
            }
        }
        index
    }
}
fn file_module(file: &ParsedFile) -> String {
    let p = file.path.strip_prefix("src/").unwrap_or(&file.path);
    let mut parts: Vec<_> = p.trim_end_matches(".rs").split('/').collect();
    if parts
        .last()
        .is_some_and(|p| matches!(*p, "lib" | "main" | "mod"))
    {
        parts.pop();
    }
    parts.join("::")
}
fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.into()
    } else if b.is_empty() {
        a.into()
    } else {
        format!("{a}::{b}")
    }
}
fn absolute(base: &str, path: &str) -> String {
    let mut parts: Vec<_> = base.split("::").filter(|s| !s.is_empty()).collect();
    let mut at = 0;
    let path_parts: Vec<_> = path.split("::").collect();
    if path_parts.first() == Some(&"crate") {
        parts.clear();
        at = 1;
    } else if path_parts.first() == Some(&"self") {
        at = 1;
    }
    for p in &path_parts[at..] {
        if *p == "super" {
            parts.pop();
        } else if !p.is_empty() {
            parts.push(p);
        }
    }
    parts.join("::")
}
fn canonical(file: &ParsedFile, s: &Symbol) -> String {
    join(&file_module(file), &s.qualified_name)
}
fn source_module(snapshot: &Lookup<'_>, file: &ParsedFile, r: &Reference) -> String {
    let scopes = super::owners(snapshot, snapshot.symbol(&r.source_id));
    let inline = scopes
        .iter()
        .find(|s| s.kind == "module")
        .map(|s| s.qualified_name.as_str())
        .unwrap_or("");
    join(&file_module(file), inline)
}
fn lookup<'a>(
    snapshot: &Lookup<'a>,
    source: &ParsedFile,
    target: &str,
    depth: usize,
    visited: &mut BTreeSet<String>,
) -> Vec<&'a Symbol> {
    if depth > 32 || !visited.insert(target.into()) {
        return vec![];
    }
    let mut result: Vec<_> = snapshot
        .rust
        .symbols
        .get(target)
        .into_iter()
        .flatten()
        .copied()
        .filter(|s| s.file_path == source.path || s.visibility == "public")
        .collect();
    if let Some((module, name)) = target.rsplit_once("::") {
        for import in snapshot.rust.reexports.get(module).into_iter().flatten() {
            if import.local != name && import.imported != "*" {
                continue;
            }
            let target = absolute(
                module,
                &join(
                    &import.module,
                    if import.imported == "*" {
                        name
                    } else {
                        &import.imported
                    },
                ),
            );
            result.extend(lookup(snapshot, source, &target, depth + 1, visited));
        }
    }
    result
}
pub(super) fn conditional(snapshot: &Lookup<'_>, target: &Symbol) -> bool {
    let Some(file) = snapshot.file(&target.file_path) else {
        return true;
    };
    let name = canonical(file, target);
    snapshot
        .rust
        .conditional_prefixes
        .iter()
        .any(|prefix| name.starts_with(prefix))
}
pub(super) fn imported<'a>(
    snapshot: &Lookup<'a>,
    file: &ParsedFile,
    import: &Import,
    name: &str,
) -> Vec<&'a Symbol> {
    let mut visited = BTreeSet::new();
    lookup(
        snapshot,
        file,
        &absolute(&file_module(file), &join(&import.module, name)),
        0,
        &mut visited,
    )
}
pub(super) fn qualified<'a>(
    snapshot: &Lookup<'a>,
    file: &ParsedFile,
    r: &Reference,
) -> Vec<&'a Symbol> {
    let Some(q) = r.qualifier.as_deref() else {
        return vec![];
    };
    let base = source_module(snapshot, file, r);
    let scopes = super::owners(snapshot, snapshot.symbol(&r.source_id));
    let mut paths = vec![absolute(&base, &join(q, &r.name))];
    for (i, import) in file.imports.iter().enumerate() {
        if import.local == q
            && file
                .import_scopes
                .get(i)
                .is_some_and(|id| scopes.iter().any(|s| &s.symbol_id == id))
        {
            paths.push(absolute(
                &file_module(file),
                &join(&join(&import.module, &import.imported), &r.name),
            ));
        }
    }
    paths
        .into_iter()
        .flat_map(|p| lookup(snapshot, file, &p, 0, &mut BTreeSet::new()))
        .filter(|s| !matches!(s.kind.as_str(), "field" | "property"))
        .collect()
}
