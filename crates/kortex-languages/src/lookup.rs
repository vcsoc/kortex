//! Borrowed acceleration structures for ONE immutable resolution pass. Never reused across generations.
use kortex_core::{ParsedFile, Snapshot, Symbol};
use std::{cell::RefCell, collections::HashMap, ops::Deref, path::Path};

type Named<'a> = HashMap<&'a str, HashMap<&'a str, Vec<&'a Symbol>>>;
type Modules<'a> = HashMap<(String, String, Option<String>), Vec<&'a ParsedFile>>;
type Candidates<'a> = HashMap<(String, String, String, Option<String>), Vec<&'a Symbol>>;

pub(super) struct Lookup<'a> {
    snapshot: &'a Snapshot,
    symbols: HashMap<&'a str, &'a Symbol>,
    files: HashMap<&'a str, &'a ParsedFile>,
    positions: HashMap<&'a str, usize>,
    local_names: Named<'a>,
    names: Named<'a>,
    qualified: Named<'a>,
    directories: HashMap<&'a Path, Vec<&'a ParsedFile>>,
    children: HashMap<&'a str, Vec<&'a Symbol>>,
    extended_modules: HashMap<&'a str, HashMap<String, Vec<&'a ParsedFile>>>,
    pub rust: super::rust_modules::Index<'a>,
    pub modules: RefCell<Modules<'a>>,
    pub candidates: RefCell<Candidates<'a>>,
}
impl<'a> Lookup<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Self {
        let mut index = Self {
            snapshot,
            symbols: HashMap::new(),
            files: HashMap::new(),
            positions: HashMap::new(),
            local_names: HashMap::new(),
            names: HashMap::new(),
            qualified: HashMap::new(),
            directories: HashMap::new(),
            children: HashMap::new(),
            extended_modules: HashMap::new(),
            rust: super::rust_modules::Index::new(snapshot),
            modules: RefCell::new(HashMap::new()),
            candidates: RefCell::new(HashMap::new()),
        };
        for (position, file) in snapshot.files.iter().enumerate() {
            index.files.entry(&file.path).or_insert(file);
            index.positions.entry(&file.path).or_insert(position);
            if let Some(parent) = Path::new(&file.path).parent() {
                index.directories.entry(parent).or_default().push(file);
            }
            if matches!(file.language.as_str(), "java" | "csharp" | "php") {
                let mut modules = std::collections::BTreeSet::new();
                let namespace = file
                    .symbols
                    .iter()
                    .find(|s| s.kind == "file")
                    .and_then(|s| s.attributes["namespace"].as_str())
                    .unwrap_or("");
                modules.insert(namespace.to_owned());
                for symbol in &file.symbols {
                    let mut name = symbol.qualified_name.as_str();
                    modules.insert(name.to_owned());
                    while let Some((parent, _)) = name.rsplit_once("::") {
                        modules.insert(parent.to_owned());
                        name = parent;
                    }
                }
                for module in modules {
                    index
                        .extended_modules
                        .entry(&file.language)
                        .or_default()
                        .entry(module)
                        .or_default()
                        .push(file);
                }
            }
            for symbol in &file.symbols {
                if let Some(owner) = symbol.owner_id.as_deref() {
                    index.children.entry(owner).or_default().push(symbol);
                }
                // Match Snapshot::symbol's first-match behavior; collisions never overwrite evidence.
                index.symbols.entry(&symbol.symbol_id).or_insert(symbol);
                index
                    .local_names
                    .entry(&file.path)
                    .or_default()
                    .entry(&symbol.name)
                    .or_default()
                    .push(symbol);
                index
                    .names
                    .entry(&file.language)
                    .or_default()
                    .entry(&symbol.name)
                    .or_default()
                    .push(symbol);
                index
                    .qualified
                    .entry(&file.language)
                    .or_default()
                    .entry(&symbol.qualified_name)
                    .or_default()
                    .push(symbol);
            }
        }
        index
    }
    pub fn all_symbols(&self) -> impl Iterator<Item = &'a Symbol> {
        self.snapshot.files.iter().flat_map(|f| &f.symbols)
    }
    pub fn symbol(&self, id: &str) -> Option<&'a Symbol> {
        self.symbols.get(id).copied()
    }
    pub fn file(&self, path: &str) -> Option<&'a ParsedFile> {
        self.files.get(path).copied()
    }
    pub fn local(
        &self,
        file: &ParsedFile,
        name: &str,
    ) -> std::iter::Copied<std::slice::Iter<'_, &'a Symbol>> {
        self.local_names
            .get(file.path.as_str())
            .and_then(|names| names.get(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .copied()
    }
    pub fn named(
        &self,
        language: &str,
        name: &str,
    ) -> std::iter::Copied<std::slice::Iter<'_, &'a Symbol>> {
        self.names
            .get(language)
            .and_then(|names| names.get(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .copied()
    }
    pub fn qualified(
        &self,
        language: &str,
        name: &str,
    ) -> std::iter::Copied<std::slice::Iter<'_, &'a Symbol>> {
        self.qualified
            .get(language)
            .and_then(|names| names.get(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .copied()
    }
    pub fn children(&self, owner: &str) -> impl Iterator<Item = &'a Symbol> {
        self.children.get(owner).into_iter().flatten().copied()
    }
    pub fn extended_module(&self, language: &str, module: &str) -> Vec<&'a ParsedFile> {
        self.extended_modules
            .get(language)
            .and_then(|modules| modules.get(module))
            .cloned()
            .unwrap_or_default()
    }
    pub fn directory(&self, path: &Path) -> impl Iterator<Item = &'a ParsedFile> {
        self.directories.get(path).into_iter().flatten().copied()
    }
    pub fn ordered(&self, files: &mut Vec<&'a ParsedFile>) {
        files.sort_by_key(|file| self.positions.get(file.path.as_str()).copied());
        files.dedup_by(|a, b| a.path == b.path);
    }
}
impl Deref for Lookup<'_> {
    type Target = Snapshot;
    fn deref(&self) -> &Self::Target {
        self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &str, language: &str, namespace: &str, names: &[&str]) -> ParsedFile {
        ParsedFile {
            path: path.into(),
            language: language.into(),
            content_hash: String::new(),
            size_bytes: 0,
            symbols: names
                .iter()
                .enumerate()
                .map(|(i, name)| Symbol {
                    symbol_id: format!("{path}:{i}"),
                    symbol_revision_id: String::new(),
                    repo_id: "test".into(),
                    kind: if i == 0 { "file" } else { "function" }.into(),
                    language: language.into(),
                    name: name.rsplit("::").next().unwrap().into(),
                    qualified_name: (*name).into(),
                    file_path: path.into(),
                    span: Default::default(),
                    signature: String::new(),
                    visibility: "public".into(),
                    attributes: serde_json::json!({"namespace": namespace}),
                    content_hash: String::new(),
                    doc_comment: None,
                    owner_id: (i > 0).then(|| format!("{path}:0")),
                })
                .collect(),
            references: vec![],
            imports: vec![],
            import_scopes: vec![],
            import_spans: vec![],
            diagnostics: vec![],
        }
    }
    #[test]
    fn borrowed_indexes_match_linear_queries_and_preserve_overloads() {
        let snapshot = Snapshot {
            files: vec![
                file(
                    "z.java",
                    "java",
                    "Lib",
                    &["z.java", "Lib::Auth::run", "Lib::Auth::run"],
                ),
                file("dir/a.java", "java", "Other", &["dir/a.java", "Other::run"]),
                file("dir/b.cs", "csharp", "Lib", &["dir/b.cs", "Lib::Auth::run"]),
            ],
            ..Default::default()
        };
        let index = Lookup::new(&snapshot);
        for f in &snapshot.files {
            assert!(std::ptr::eq(index.file(&f.path).unwrap(), f));
            for s in &f.symbols {
                assert!(std::ptr::eq(
                    index.symbol(&s.symbol_id).unwrap(),
                    snapshot.symbol(&s.symbol_id).unwrap()
                ));
                assert_eq!(
                    index.local(f, &s.name).collect::<Vec<_>>(),
                    f.symbols
                        .iter()
                        .filter(|t| t.name == s.name)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    index.named(&f.language, &s.name).collect::<Vec<_>>(),
                    snapshot
                        .files
                        .iter()
                        .filter(|g| g.language == f.language)
                        .flat_map(|g| &g.symbols)
                        .filter(|t| t.name == s.name)
                        .collect::<Vec<_>>()
                );
            }
            for module in [
                "",
                "Lib",
                "Lib::Auth",
                "Lib::Auth::run",
                "Lib::Auth::run::unknown",
                "Other",
                "missing",
            ] {
                let linear: Vec<_> = snapshot
                    .files
                    .iter()
                    .filter(|g| {
                        g.language == f.language
                            && (g.symbols[0].attributes["namespace"] == module
                                || g.symbols.iter().any(|s| {
                                    s.qualified_name == module
                                        || s.qualified_name.starts_with(&format!("{module}::"))
                                }))
                    })
                    .collect();
                assert_eq!(index.extended_module(&f.language, module), linear);
            }
        }
        assert_eq!(index.qualified("java", "Lib::Auth::run").count(), 2);
        assert_eq!(index.children("z.java:0").count(), 2);
        assert_eq!(index.directory(Path::new("dir")).count(), 2);
        let mut reversed = vec![&snapshot.files[2], &snapshot.files[0], &snapshot.files[2]];
        index.ordered(&mut reversed);
        assert_eq!(reversed, vec![&snapshot.files[0], &snapshot.files[2]]);
    }
    #[test]
    fn module_and_candidate_cache_is_pass_local_and_keeps_missing_results() {
        let mut snapshot = Snapshot {
            files: vec![
                file("main.py", "python", "", &["main.py"]),
                file("target.py", "python", "", &["target.py", "run"]),
            ],
            ..Default::default()
        };
        let import = kortex_core::Import {
            local: "run".into(),
            module: "target".into(),
            imported: "run".into(),
        };
        {
            let index = Lookup::new(&snapshot);
            for _ in 0..100 {
                assert_eq!(
                    super::super::module_candidates(
                        &index,
                        &snapshot.files[0],
                        &import,
                        "run",
                        None
                    )
                    .len(),
                    1
                );
                assert!(
                    super::super::module_candidates(
                        &index,
                        &snapshot.files[0],
                        &import,
                        "missing",
                        None
                    )
                    .is_empty()
                );
            }
            assert_eq!(index.modules.borrow().len(), 1);
            assert_eq!(index.candidates.borrow().len(), 2);
        }
        snapshot.files.pop();
        let next = Lookup::new(&snapshot);
        assert!(next.modules.borrow().is_empty() && next.candidates.borrow().is_empty());
        assert!(
            super::super::module_candidates(&next, &snapshot.files[0], &import, "run", None)
                .is_empty()
        );
    }
}
