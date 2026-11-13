//! Shared application services used by CLI and MCP. No transport-specific semantics.
mod changes;
mod contracts;
mod execution;
mod intelligence;
#[cfg(test)]
mod intelligence_tests;
mod read_service;
mod rules;
mod test_map;
pub use read_service::ReadService;
#[cfg(test)]
mod read_query_tests;
#[cfg(test)]
mod read_service_tests;
pub use contracts::{input_schema, response_schema};
#[cfg(test)]
mod changes_tests;
#[cfg(all(test, feature = "local-embeddings"))]
mod local_query_tests;
#[cfg(test)]
mod selection_tests;

use kortex_config::Config;
use kortex_core::{Error, RequestControl, Result, Snapshot, Symbol, hash};
use kortex_graph::{Direction, Limits};
use kortex_storage::{Store, registry::Registry};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// Unsupported non-source files are routinely skipped; every other discovery
// diagnostic may indicate that source evidence is missing.
fn discovery_diagnostic_is_partial(diagnostic: &str) -> bool {
    !diagnostic.ends_with(": unsupported language")
}

pub struct Service {
    pub root: PathBuf,
    pub config: Config,
    pub store: Store,
    pub snapshot: Snapshot,
}
pub fn select_repo(selector: Option<&str>) -> Result<PathBuf> {
    if let Some(s) = selector {
        let p = Path::new(s);
        if p.is_dir() {
            return kortex_repo::root(p);
        }
        for (id, root, _) in Registry::open()?.list()? {
            if id == s {
                return kortex_repo::root(Path::new(&root));
            }
        }
        return Err(Error::new("REPO_NOT_FOUND", s, 3));
    }
    let current = std::env::current_dir()?;
    for ancestor in current.ancestors() {
        if ancestor.join(".kortex/index.db").is_file() {
            return kortex_repo::root(ancestor);
        }
    }
    let repos = Registry::open()?.list()?;
    if repos.len() == 1 {
        return kortex_repo::root(Path::new(&repos[0].1));
    }
    Err(Error::new(
        if repos.is_empty() {
            "REPO_NOT_FOUND"
        } else {
            "AMBIGUOUS_REPO"
        },
        "select a repository explicitly with --repo or repo input",
        if repos.is_empty() { 3 } else { 4 },
    ))
}
pub fn list_repos() -> Result<Value> {
    collect_repos(None)
}
fn collect_repos(deadline: Option<Instant>) -> Result<Value> {
    let mut items = vec![];
    for (repo_id, root, generation_id) in Registry::open()?.list()? {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "registry freshness enumeration exceeded deadline",
                9,
            ));
        }
        let status = Service::open_registered(&repo_id, Path::new(&root), None)
            .and_then(|s| s.freshness())
            .unwrap_or_else(|_| "unavailable".into());
        items.push(json!({"repo_id":repo_id,"root":root,"generation_id":generation_id,"index_status":status}));
    }
    Ok(json!({"schema_version":"1.0","data":{"items":items},"partial":false,"truncated":false}))
}
/// Bounded registry API. The old `list_repos` remains an unpaged compatibility adapter.
pub fn list_repos_page(args: &Value) -> Result<Value> {
    contracts::validate_input("list_repos", args)?;
    let started = Instant::now();
    let deadline =
        started + std::time::Duration::from_millis(number(args, "timeout_ms", 10000)? as u64);
    let all = collect_repos(Some(deadline))?;
    let fingerprint = hash(serde_json::to_vec(&all["data"]["items"])?);
    let offset = match args.get("cursor").filter(|v| !v.is_null()) {
        None => 0,
        Some(value) => {
            let token = value
                .as_str()
                .ok_or_else(|| Error::new("INVALID_CURSOR", "expected cursor string", 2))?;
            let parts: Vec<_> = token.split('.').collect();
            if parts.len() != 3 || parts[0] != "kr1" || parts[1] != fingerprint {
                return Err(Error::new(
                    "INVALID_CURSOR",
                    "repository list changed; restart pagination",
                    2,
                ));
            }
            parts[2]
                .parse::<usize>()
                .map_err(|_| Error::new("INVALID_CURSOR", "invalid registry offset", 2))?
        }
    };
    if offset > 100_000 {
        return Err(Error::new(
            "INVALID_CURSOR",
            "registry offset exceeds policy",
            2,
        ));
    }
    let limit = number(args, "limit", 20)?;
    let items = all["data"]["items"].as_array().unwrap();
    let page: Vec<_> = items.iter().skip(offset).take(limit).cloned().collect();
    let more = items.len() > offset.saturating_add(page.len());
    let cursor = more.then(|| format!("kr1.{fingerprint}.{}", offset + page.len()));
    let result = json!({"schema_version":"1.0","request_id":hash(format!("registry-{}",SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())),"repo_id":null,"generation_id":null,"index_status":"unavailable","data":{"schema_version":"1.0","operation":"list_repos","items":page},"page":{"next_cursor":cursor},"partial":false,"truncated":false,"warnings":[],"timing_ms":started.elapsed().as_millis()});
    if started.elapsed().as_millis() >= number(args, "timeout_ms", 10000)? as u128 {
        return Err(Error::new(
            "QUERY_TIMEOUT",
            "registry enumeration exceeded deadline",
            9,
        ));
    }
    if serde_json::to_vec(&result)?.len() > number(args, "max_response_bytes", 262144)? {
        return Err(Error::new(
            "RESPONSE_BUDGET",
            "registry page exceeds byte budget; reduce limit",
            7,
        ));
    }
    Ok(result)
}
/// Resolve configuration without opening index storage, including for corrupt-index repair.
/// Registered identities retain their recorded origin and cannot be redirected by configuration.
pub fn resolve_selection(
    selector: Option<&str>,
    config_path: Option<&Path>,
) -> Result<(PathBuf, Config)> {
    if let Some(selector) = selector {
        if Path::new(selector).is_dir() {
            return resolve_path_selection(Path::new(selector), config_path);
        }
        let row = Registry::open()?
            .list()?
            .into_iter()
            .find(|row| row.0 == selector)
            .ok_or_else(|| Error::new("REPO_NOT_FOUND", selector, 3))?;
        return resolve_registered_selection(&row.0, Path::new(&row.1), config_path);
    }
    let current = std::env::current_dir()?;
    for ancestor in current.ancestors() {
        if ancestor.join(".kortex/index.db").is_file() {
            // Prefer the registry selection of this exact index, not a guessed ancestor config.
            let root = ancestor.canonicalize()?;
            let repo_id = kortex_core::id(&["repository", &kortex_core::public_path(&root)]);
            if Registry::open()?.list()?.iter().any(|row| row.0 == repo_id) {
                return resolve_registered_selection(&repo_id, &root, config_path);
            }
            return resolve_path_selection(ancestor, config_path);
        }
    }
    let rows = Registry::open()?.list()?;
    if let [row] = rows.as_slice() {
        return resolve_registered_selection(&row.0, Path::new(&row.1), config_path);
    }
    Err(Error::new(
        if rows.is_empty() {
            "REPO_NOT_FOUND"
        } else {
            "AMBIGUOUS_REPO"
        },
        "select a repository explicitly with --repo or repo input",
        if rows.is_empty() { 3 } else { 4 },
    ))
}
/// Legacy entries use only their exact root, never ancestor configuration discovery.
fn resolve_registered_selection(
    repo_id: &str,
    authorized_root: &Path,
    config_path: Option<&Path>,
) -> Result<(PathBuf, Config)> {
    let registry = Registry::open()?;
    let row = registry
        .list()?
        .into_iter()
        .find(|row| row.0 == repo_id)
        .ok_or_else(|| Error::new("REPO_NOT_FOUND", repo_id, 3))?;
    let root = authorized_root.canonicalize()?;
    let registered_root = Path::new(&row.1).canonicalize()?;
    let expected_id = kortex_core::id(&["repository", &kortex_core::public_path(&root)]);
    if root != registered_root || repo_id != expected_id {
        return Err(Error::new(
            "REPO_FORBIDDEN",
            "authorized registry root/identity mismatch",
            8,
        ));
    }
    let selection = registry.selection(repo_id)?;
    if let Some(selection) = &selection
        && (!selection.effective_root.is_absolute()
            || selection.effective_root != root
            || !selection.invocation_root.is_absolute()
            || selection
                .explicit_config
                .as_ref()
                .is_some_and(|path| !path.is_absolute()))
    {
        return Err(Error::new(
            "REPO_FORBIDDEN",
            "invalid registered selection provenance",
            8,
        ));
    }
    let invocation = selection
        .as_ref()
        .map_or(root.as_path(), |s| s.invocation_root.as_path());
    let explicit = config_path.or_else(|| {
        selection
            .as_ref()
            .and_then(|s| s.explicit_config.as_deref())
    });
    let config = Config::load(invocation, explicit)?;
    if config.resolved_root(invocation)? != root {
        return Err(Error::new(
            "REPO_FORBIDDEN",
            "configuration changed the registered root; explicitly reindex the new selection",
            8,
        ));
    }
    Ok((root, config))
}
fn resolve_path_selection(root: &Path, config_path: Option<&Path>) -> Result<(PathBuf, Config)> {
    let invocation_root = kortex_repo::root(root)?;
    let config = Config::load(&invocation_root, config_path)?;
    let root = config.resolved_root(&invocation_root)?;
    Ok((root, config))
}
impl Service {
    /// Select through the same no-Store policy used by administrative callers.
    pub fn open_selected(selector: Option<&str>, config_path: Option<&Path>) -> Result<Self> {
        let (root, config) = resolve_selection(selector, config_path)?;
        let expected_id = kortex_core::id(&["repository", &kortex_core::public_path(&root)]);
        let service = Self::open_at_root(&root, config)?;
        if service.snapshot.repo_id != expected_id {
            return Err(Error::new(
                "REPO_FORBIDDEN",
                "selection/index identity mismatch",
                8,
            ));
        }
        Ok(service)
    }
    /// Reopen exactly an authorized registered identity. Configuration may not redirect it.
    pub fn open_registered(
        repo_id: &str,
        authorized_root: &Path,
        config_path: Option<&Path>,
    ) -> Result<Self> {
        let (root, config) = resolve_registered_selection(repo_id, authorized_root, config_path)?;
        let service = Self::open_at_root(&root, config)?;
        if service.snapshot.repo_id != repo_id {
            return Err(Error::new(
                "REPO_FORBIDDEN",
                "registry/index identity mismatch",
                8,
            ));
        }
        Ok(service)
    }
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_config(root, None)
    }
    pub fn open_with_config(root: &Path, config_path: Option<&Path>) -> Result<Self> {
        let (root, config) = resolve_path_selection(root, config_path)?;
        Self::open_at_root(&root, config)
    }
    /// Use an already resolved root/config without reapplying relocation or Git expansion.
    pub fn open_at_root(root: &Path, config: Config) -> Result<Self> {
        let (root, store) = open_store_at_root(root, &config)?;
        let snapshot = store.load()?;
        if snapshot.root != kortex_core::public_path(&root) {
            return Err(Error::new(
                "INDEX_RELOCATED",
                "index belongs to another root; rebuild with index --full",
                6,
            ));
        }
        Ok(Self {
            root,
            config,
            store,
            snapshot,
        })
    }
    pub fn freshness(&self) -> Result<String> {
        let control = RequestControl::new(std::time::Duration::from_millis(
            self.config.analysis.query_timeout_ms,
        ))?;
        self.freshness_with_control(&control)
    }
    pub fn freshness_with_control(&self, control: &RequestControl) -> Result<String> {
        if !execution::freshness_header(
            &self.root,
            &self.config,
            execution::FreshnessHeader {
                resolution_hash: &self.snapshot.resolution_hash,
                config_hash: &self.snapshot.config_hash,
                grammar_hash: &self.snapshot.grammar_hash,
                git_head: &self.snapshot.git_head,
            },
            control,
        )? {
            return Ok("stale".into());
        }
        execution::freshness_files(
            &self.root,
            &self.config,
            self.snapshot
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.content_hash.as_str())),
            control,
        )
    }
    pub fn lookup(&self, selector: &str) -> Result<&Symbol> {
        let control = RequestControl::new(std::time::Duration::from_millis(
            self.config.analysis.query_timeout_ms,
        ))?;
        self.lookup_with_control(selector, &control)
    }
    pub fn lookup_with_control(&self, selector: &str, control: &RequestControl) -> Result<&Symbol> {
        let mut found = vec![];
        for s in self.snapshot.symbols() {
            control.check()?;
            if s.symbol_id == selector || s.qualified_name == selector || s.name == selector {
                found.push(s);
            }
        }
        if found.is_empty()
            && let Some((path, name)) = selector.rsplit_once(':')
        {
            let line = name.parse::<usize>().ok();
            for s in self.snapshot.symbols() {
                control.check()?;
                if s.file_path == path
                    && match line {
                        Some(line) => {
                            s.span.start_line <= line && s.span.end_line >= line && s.kind != "file"
                        }
                        None => s.name == name || s.qualified_name == name,
                    }
                {
                    found.push(s);
                }
            }
            if line.is_some() {
                kortex_search::sort_with_control(
                    &mut found,
                    |a, b| {
                        (a.span.end_byte - a.span.start_byte)
                            .cmp(&(b.span.end_byte - b.span.start_byte))
                    },
                    control,
                )?;
                found.truncate(1);
            }
        }
        match found.as_slice() {
            [s] => Ok(s),
            [] => Err(Error::new("SYMBOL_NOT_FOUND", selector, 3)),
            _ => Err(Error::new(
                "AMBIGUOUS_SYMBOL",
                format!(
                    "{selector}: {} matches; use file:name or symbol_id",
                    found.len()
                ),
                4,
            )),
        }
    }
    fn matching_edges(
        &self,
        control: &RequestControl,
        predicate: impl Fn(&kortex_core::Edge) -> bool,
    ) -> Result<Vec<&kortex_core::Edge>> {
        let mut edges = vec![];
        for edge in &self.snapshot.edges {
            control.check()?;
            if predicate(edge) {
                edges.push(edge);
                if edges.len() == 501 {
                    break;
                }
            }
        }
        Ok(edges)
    }
    fn limits(&self, args: &Value) -> Result<Limits> {
        let depth = number(args, "depth", self.config.analysis.default_depth)?;
        let max_nodes = number(args, "max_nodes", self.config.analysis.max_nodes)?;
        if depth > self.config.analysis.max_depth || max_nodes > self.config.analysis.max_nodes {
            return Err(Error::new(
                "QUERY_LIMIT",
                "requested bounds exceed policy",
                2,
            ));
        }
        Ok(Limits {
            depth,
            max_nodes,
            timeout_ms: number(
                args,
                "timeout_ms",
                self.config.analysis.query_timeout_ms as usize,
            )? as u64,
        })
    }
    pub fn execute(&self, operation: &str, args: &Value) -> Result<Value> {
        let control = RequestControl::new(std::time::Duration::from_millis(
            self.config.analysis.query_timeout_ms,
        ))?;
        self.execute_with_control(operation, args, &control)
    }
    /// Cooperative control shared with transports and all interruptible analysis loops.
    /// Kernel I/O, SQLite statements and individual native operators remain indivisible.
    pub fn execute_with_control(
        &self,
        operation: &str,
        args: &Value,
        control: &RequestControl,
    ) -> Result<Value> {
        self.execute_inner(operation, args, None, control)
    }
    /// Explicit SDK opt-in with a real, configured local provider. Transport defaults remain
    /// gated until the owner approves pretrained artifacts and relevance acceptance.
    /// All shared freshness, schema, cursor, deadline and output-budget semantics still apply.
    pub fn execute_with_local_model(
        &self,
        args: &Value,
        provider: &mut dyn kortex_search::hybrid::LocalEmbedder,
        cache: &mut kortex_search::hybrid::EmbeddingCache,
    ) -> Result<Value> {
        let control = RequestControl::new(std::time::Duration::from_millis(
            self.config.analysis.query_timeout_ms,
        ))?;
        self.execute_with_local_model_and_control(args, provider, cache, &control)
    }
    pub fn execute_with_local_model_and_control(
        &self,
        args: &Value,
        provider: &mut dyn kortex_search::hybrid::LocalEmbedder,
        cache: &mut kortex_search::hybrid::EmbeddingCache,
        control: &RequestControl,
    ) -> Result<Value> {
        control.check()?;
        if self.config.search.embeddings != "local" {
            return Err(Error::new(
                "LOCAL_MODEL_GATE",
                "trusted local-model configuration is required",
                7,
            ));
        }
        let manifest = kortex_search::hybrid::read_manifest(
            Path::new(&self.config.search.model_path),
            &self.config.search.model_checksum,
        )?;
        if &manifest != provider.model().manifest()
            || manifest.revision != self.config.search.model_version
        {
            return Err(Error::new(
                "INVALID_LOCAL_MODEL",
                "provider differs from the configured manifest/revision",
                2,
            ));
        }
        control.check()?;
        self.execute_inner("query", args, Some((provider, cache)), control)
    }
    fn execute_inner(
        &self,
        operation: &str,
        args: &Value,
        local: Option<(
            &mut dyn kortex_search::hybrid::LocalEmbedder,
            &mut kortex_search::hybrid::EmbeddingCache,
        )>,
        control: &RequestControl,
    ) -> Result<Value> {
        let context = contracts::CursorContext::from(self);
        let (started, control) = execution::prepare(&context, operation, args, control)?;
        let deadline = control.deadline();
        let freshness = self.freshness_with_control(&control)?;
        control.check()?;
        if args.get("require_fresh").and_then(Value::as_bool) == Some(true) && freshness != "fresh"
        {
            return Err(Error::new("STALE_INDEX", "reindex before querying", 5));
        }
        let mut truncated = false;
        let mut next_cursor = None;
        let data = match operation {
            "status" => {
                json!({"root":self.snapshot.root,"files":self.snapshot.files.len(),"symbols":self.snapshot.symbols().count(),"edges":self.snapshot.edges.len(),"git_head":self.snapshot.git_head,"diagnostics":self.snapshot.diagnostics,"parser_diagnostics":self.snapshot.files.iter().flat_map(|f|&f.diagnostics).collect::<Vec<_>>(),"integrity":self.store.integrity()?})
            }
            "files" => {
                let limit = number(args, "limit", 20)?;
                let offset = contracts::cursor_offset(self, operation, args)?;
                let prefix = args
                    .get("path_prefix")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut files = vec![];
                for file in &self.snapshot.files {
                    control.check()?;
                    if file.path.starts_with(prefix) {
                        files.push(file);
                    }
                }
                kortex_search::sort_with_control(&mut files, |a, b| a.path.cmp(&b.path), &control)?;
                let more = files.len() > offset.saturating_add(limit);
                let items: Vec<_> = files.into_iter().skip(offset).take(limit).map(|file|json!({"path":file.path,"language":file.language,"size_bytes":file.size_bytes,"content_hash":file.content_hash})).collect();
                if more {
                    next_cursor = Some(contracts::cursor(
                        self,
                        operation,
                        args,
                        offset + items.len(),
                    ));
                }
                json!({"items":items})
            }
            "query" => {
                if self.config.search.embeddings != "disabled" && local.is_none() {
                    return Err(Error::new(
                        "LOCAL_MODEL_GATE",
                        "local inference is experimental and automatic transport activation is gated on pretrained-artifact and relevance acceptance; use explicit SDK provider integration or set search.embeddings=disabled",
                        7,
                    ));
                }
                let limit = number(args, "limit", 20)?;
                let offset = contracts::cursor_offset(self, operation, args)?;
                let filters = kortex_search::Filters {
                    language: args.get("language").and_then(Value::as_str),
                    kind: args.get("kind").and_then(Value::as_str),
                    path_prefix: args.get("path_prefix").and_then(Value::as_str),
                };
                let (hits, more, retrieval) = if let Some((provider, cache)) = local {
                    let result = kortex_search::retrieval::query_hybrid_with_control(
                        &self.store,
                        &self.snapshot,
                        string(args, "text")?,
                        provider,
                        cache,
                        &kortex_search::retrieval::HybridOptions {
                            filters,
                            limit,
                            offset,
                            deadline,
                            weights: kortex_search::hybrid::RankingWeights {
                                lexical: self.config.search.keyword_weight,
                                vector: self.config.search.vector_weight,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        &control,
                    )?;
                    let retrieval = json!({"algorithm":result.algorithm,"model_fingerprint":result.model_fingerprint,"model_id":result.model_id,"model_revision":result.model_revision,"candidates_examined":result.candidates_examined,"experimental":true,"pretrained_quality_accepted":false});
                    (result.items, result.more, Some((retrieval, result.partial)))
                } else {
                    let (hits, more) = kortex_search::query_filtered_with_control(
                        &self.store,
                        &self.snapshot,
                        string(args, "text")?,
                        limit,
                        offset,
                        &filters,
                        &control,
                    )?;
                    (hits, more, None)
                };
                if more {
                    next_cursor = Some(contracts::cursor(self, operation, args, offset + limit));
                }
                let mut data = json!({"items":hits});
                if let Some((retrieval, partial)) = retrieval {
                    data["retrieval"] = retrieval;
                    data["partial"] = json!(partial);
                }
                data
            }
            "context" => {
                let symbol = self.lookup_with_control(string(args, "symbol")?, &control)?;
                let callers = self.matching_edges(&control, |e| {
                    e.target_id.as_deref() == Some(&symbol.symbol_id) && e.kind == "CALLS"
                })?;
                let callees = self.matching_edges(&control, |e| {
                    e.source_id == symbol.symbol_id && e.kind == "CALLS"
                })?;
                truncated = callers.len() > 500 || callees.len() > 500;
                let references = self.matching_edges(&control, |e| {
                    e.target_id.as_deref() == Some(&symbol.symbol_id) && e.kind == "REFERENCES"
                })?;
                let types = self.matching_edges(&control, |e| {
                    e.source_id == symbol.symbol_id && e.kind == "USES_TYPE"
                })?;
                let associations = kortex_graph::traverse_with_control(
                    &self.snapshot,
                    &symbol.symbol_id,
                    Direction::Upstream,
                    &self.limits(args)?,
                    false,
                    &control,
                )?;
                truncated |= references.len() > 500 || types.len() > 500 || associations.truncated;
                let test_associations =
                    test_map::associations(self, &associations.items, &control)?;
                let tests: Vec<_> = test_associations
                    .iter()
                    .filter_map(|r| {
                        r["symbol_id"]
                            .as_str()
                            .and_then(|id| self.snapshot.symbol(id))
                    })
                    .collect();
                json!({"symbol":symbol,"owner":symbol.owner_id.as_deref().and_then(|id|self.snapshot.symbol(id)),"callers":callers.into_iter().take(500).collect::<Vec<_>>(),"callees":callees.into_iter().take(500).collect::<Vec<_>>(),"tests":tests,"references":references.into_iter().take(500).collect::<Vec<_>>(),"types":types.into_iter().take(500).collect::<Vec<_>>(),"test_associations":test_associations})
            }
            "impact" => {
                let target = self.lookup_with_control(string(args, "symbol")?, &control)?;
                let limits = self.limits(args)?;
                let direction = Direction::parse(
                    args.get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("upstream"),
                )?;
                let t = kortex_graph::traverse_with_control(
                    &self.snapshot,
                    &target.symbol_id,
                    direction,
                    &limits,
                    false,
                    &control,
                )?;
                truncated = t.truncated;
                let direct = t.items.iter().filter(|i| i.depth == 1).count();
                let indirect = t.items.len() - direct;
                let associations = test_map::associations(self, &t.items, &control)?;
                let mut likely_test_associations = associations;
                for reach in &t.items {
                    if self
                        .snapshot
                        .symbol(&reach.symbol_id)
                        .is_some_and(|s| s.kind == "test")
                        && !likely_test_associations
                            .iter()
                            .any(|a| a["symbol_id"] == reach.symbol_id)
                    {
                        likely_test_associations.push(json!({"symbol_id":reach.symbol_id,"depth":reach.depth,"confidence":reach.confidence,"edge_path":reach.edge_path,"test_evidence":{"framework":"indexed-test-kind","framework_verified":false,"heuristic":true,"basis":"persisted test node classification plus reverse dependency path; not independent test-framework verification"}}));
                    }
                }
                let tests: Vec<_> = likely_test_associations
                    .iter()
                    .filter_map(|r| {
                        r["symbol_id"]
                            .as_str()
                            .and_then(|id| self.snapshot.symbol(id))
                    })
                    .collect();
                let symbols: Vec<_> = t
                    .items
                    .iter()
                    .filter_map(|r| self.snapshot.symbol(&r.symbol_id))
                    .collect();
                json!({"target":target,"risk":if t.items.len()>100{"high"}else if t.items.len()>10{"medium"}else{"low"},"risk_is_heuristic":true,"direct":direct,"indirect":indirect,"partial":t.ambiguous_relationships>0 || t.unresolved_relationships>0,"traversal":t,"symbols":symbols,"likely_tests":tests,"likely_test_associations":likely_test_associations})
            }
            "trace" => {
                let from = self.lookup_with_control(string(args, "from")?, &control)?;
                let to = self.lookup_with_control(string(args, "to")?, &control)?;
                let limits = self.limits(args)?;
                let t = kortex_graph::traverse_with_control(
                    &self.snapshot,
                    &from.symbol_id,
                    Direction::Downstream,
                    &limits,
                    false,
                    &control,
                )?;
                let path = if from.symbol_id == to.symbol_id {
                    Some(vec![])
                } else {
                    t.items
                        .iter()
                        .find(|r| r.symbol_id == to.symbol_id)
                        .map(|r| {
                            r.edge_path
                                .iter()
                                .filter_map(|id| self.snapshot.edges.iter().find(|e| e.id == *id))
                                .collect::<Vec<_>>()
                        })
                };
                truncated = t.truncated;
                json!({"from":from,"to":to,"found":path.is_some(),"path":path,"search_bounded":true,"partial":t.ambiguous_relationships>0 || t.unresolved_relationships>0})
            }
            "graph" => {
                let target = self.lookup_with_control(string(args, "symbol")?, &control)?;
                let t = kortex_graph::traverse_with_control(
                    &self.snapshot,
                    &target.symbol_id,
                    Direction::Both,
                    &self.limits(args)?,
                    true,
                    &control,
                )?;
                truncated = t.truncated;
                let mut nodes = vec![target];
                nodes.extend(
                    t.items
                        .iter()
                        .filter_map(|r| self.snapshot.symbol(&r.symbol_id)),
                );
                nodes.sort_by(|a, b| a.symbol_id.cmp(&b.symbol_id));
                json!({"nodes":nodes,"edges":t.edges,"partial":t.ambiguous_relationships>0 || t.unresolved_relationships>0,"ambiguous_relationships":t.ambiguous_relationships,"unresolved_relationships":t.unresolved_relationships})
            }
            "changes" => self.change_analysis(args, &control)?,
            "routes" => {
                let result = intelligence::discover(self, args, &control)?;
                let (result, cursor) = intelligence::paginate(self, args, result)?;
                next_cursor = cursor;
                result
            }
            "check" => rules::evaluate(self, args, &control)?,
            _ => return Err(Error::new("UNKNOWN_OPERATION", operation, 2)),
        };
        execution::finish(
            &context,
            operation,
            args,
            execution::ResponseParts {
                started,
                freshness,
                indexed_partial: self
                    .snapshot
                    .files
                    .iter()
                    .any(|f| !f.diagnostics.is_empty())
                    || self
                        .snapshot
                        .diagnostics
                        .iter()
                        .any(|s| discovery_diagnostic_is_partial(s)),
                truncated,
                next_cursor,
                data,
            },
            &control,
        )
    }
    #[cfg(test)]
    fn changes(&self, args: &Value) -> Result<Value> {
        let control = RequestControl::new(std::time::Duration::from_millis(
            self.config.analysis.query_timeout_ms,
        ))?;
        self.change_analysis(args, &control)
    }
}
fn open_store_at_root(root: &Path, config: &Config) -> Result<(PathBuf, Store)> {
    config.validate()?;
    let root = root.canonicalize()?;
    if !root.is_dir() || root.to_str().is_none() {
        return Err(Error::new(
            "PATH_REFUSED",
            "effective root must be a UTF-8 directory",
            8,
        ));
    }
    let store = Store::open_with_storage(&root, &config.index.storage, false)?;
    Ok((root, store))
}
fn commit(root: &Path, reference: &str, control: &RequestControl) -> Result<String> {
    if reference.len() > 1024 {
        return Err(Error::new("INVALID_REF", "reference too long", 2));
    }
    Ok(kortex_repo::git_with_control(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
        control,
    )?
    .trim()
    .into())
}
pub fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or_else(|| {
            Error::new(
                "INVALID_INPUT",
                format!("{key}: expected nonempty string <=4096 bytes"),
                2,
            )
        })
}
pub fn number(args: &Value, key: &str, default: usize) -> Result<usize> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| {
                Error::new(
                    "INVALID_INPUT",
                    format!("{key}: expected nonnegative integer"),
                    2,
                )
            }),
    }
}
pub fn boolean(args: &Value, key: &str, default: bool) -> Result<bool> {
    match args.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| Error::new("INVALID_INPUT", format!("{key}: expected boolean"), 2)),
    }
}
pub fn error_json(error: &Error) -> Value {
    json!({"schema_version":"1.0","error":{"code":error.code,"message":error.message,"exit_code":error.exit}})
}

pub fn dot(graph: &Value) -> String {
    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
    }
    let mut out = String::from("digraph kortex {\n");
    if let Some(nodes) = graph["nodes"].as_array() {
        for n in nodes {
            out.push_str(&format!(
                "  \"{}\" [label=\"{}\"];\n",
                escape(n["symbol_id"].as_str().unwrap_or("")),
                escape(n["qualified_name"].as_str().unwrap_or(""))
            ));
        }
    }
    if let Some(edges) = graph["edges"].as_array() {
        for e in edges {
            if let Some(target) = e["target_id"].as_str() {
                out.push_str(&format!(
                    "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
                    escape(e["source_id"].as_str().unwrap_or("")),
                    escape(target),
                    escape(e["kind"].as_str().unwrap_or(""))
                ));
            }
        }
    }
    out.push_str("}\n");
    out
}
