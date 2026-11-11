//! Shared application services used by CLI and MCP. No transport-specific semantics.
mod changes;
mod contracts;
mod intelligence;
#[cfg(test)]
mod intelligence_tests;
mod rules;
mod test_map;
pub use contracts::{input_schema, response_schema};
#[cfg(test)]
mod changes_tests;

use kortex_config::Config;
use kortex_core::{Error, Result, Snapshot, Symbol, hash};
use kortex_graph::{Direction, Limits};
use kortex_storage::{Store, registry::Registry};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

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
        let status = Service::open(Path::new(&root))
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
impl Service {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_config(root, None)
    }
    pub fn open_with_config(root: &Path, config_path: Option<&Path>) -> Result<Self> {
        let invocation_root = kortex_repo::root(root)?;
        let config = Config::load(&invocation_root, config_path)?;
        let root = config.resolved_root(&invocation_root)?;
        Self::open_at_root(&root, config)
    }
    /// Use an already resolved root/config without reapplying relocation or Git expansion.
    pub fn open_at_root(root: &Path, config: Config) -> Result<Self> {
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
        if kortex_repo::inputs::resolution_fingerprint(&self.root) != self.snapshot.resolution_hash
            || self.config.fingerprint()? != self.snapshot.config_hash
            || self.snapshot.grammar_hash != kortex_core::PARSER_VERSION
            || kortex_repo::head(&self.root) != self.snapshot.git_head
        {
            return Ok("stale".into());
        }
        let inv = kortex_repo::discover(&self.root, &self.config)?;
        let before: Vec<_> = self
            .snapshot
            .files
            .iter()
            .map(|f| (&f.path, &f.content_hash))
            .collect();
        let after: Vec<_> = inv
            .files
            .iter()
            .map(|f| (&f.path, &f.content_hash))
            .collect();
        Ok(if before == after { "fresh" } else { "stale" }.into())
    }
    pub fn lookup(&self, selector: &str) -> Result<&Symbol> {
        let mut found: Vec<_> = self
            .snapshot
            .symbols()
            .filter(|s| {
                s.symbol_id == selector || s.qualified_name == selector || s.name == selector
            })
            .collect();
        if found.is_empty()
            && let Some((path, name)) = selector.rsplit_once(':')
        {
            if let Ok(line) = name.parse::<usize>() {
                found = self
                    .snapshot
                    .symbols()
                    .filter(|s| {
                        s.file_path == path
                            && s.span.start_line <= line
                            && s.span.end_line >= line
                            && s.kind != "file"
                    })
                    .collect();
                found.sort_by_key(|s| s.span.end_byte - s.span.start_byte);
                found.truncate(1);
            } else {
                found = self
                    .snapshot
                    .symbols()
                    .filter(|s| s.file_path == path && (s.name == name || s.qualified_name == name))
                    .collect();
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
        let started = Instant::now();
        contracts::validate_input(operation, args)?;
        let timeout = number(
            args,
            "timeout_ms",
            self.config.analysis.query_timeout_ms as usize,
        )? as u64;
        if timeout > self.config.analysis.query_timeout_ms {
            return Err(Error::new(
                "QUERY_LIMIT",
                "timeout exceeds repository policy",
                2,
            ));
        }
        let deadline = started + std::time::Duration::from_millis(timeout);
        if args
            .get("generation_id")
            .and_then(Value::as_str)
            .is_some_and(|id| id != self.snapshot.generation_id)
        {
            return Err(Error::new(
                "GENERATION_MISMATCH",
                "requested generation is not the pinned reader generation",
                2,
            ));
        }
        let freshness = self.freshness()?;
        if Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "freshness check exceeded query deadline",
                9,
            ));
        }
        if args.get("require_fresh").and_then(Value::as_bool) == Some(true) && freshness != "fresh"
        {
            return Err(Error::new("STALE_INDEX", "reindex before querying", 5));
        }
        let mut truncated = false;
        let mut next_cursor = None;
        let mut data = match operation {
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
                let mut files: Vec<_> = self
                    .snapshot
                    .files
                    .iter()
                    .filter(|file| file.path.starts_with(prefix))
                    .collect();
                files.sort_by(|a, b| a.path.cmp(&b.path));
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
                if self.config.search.embeddings != "disabled" {
                    return Err(Error::new(
                        "LOCAL_MODEL_GATE",
                        "local inference is experimental and transport integration is gated on dependency audit and pretrained-artifact acceptance; set search.embeddings=disabled for keyword queries",
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
                let (hits, more) = kortex_search::query_filtered(
                    &self.store,
                    &self.snapshot,
                    string(args, "text")?,
                    limit,
                    offset,
                    &filters,
                )?;
                if more {
                    next_cursor = Some(contracts::cursor(self, operation, args, offset + limit));
                }
                json!({"items":hits})
            }
            "context" => {
                let symbol = self.lookup(string(args, "symbol")?)?;
                let callers: Vec<_> = self
                    .snapshot
                    .edges
                    .iter()
                    .filter(|e| {
                        e.target_id.as_deref() == Some(&symbol.symbol_id) && e.kind == "CALLS"
                    })
                    .take(501)
                    .collect();
                let callees: Vec<_> = self
                    .snapshot
                    .edges
                    .iter()
                    .filter(|e| e.source_id == symbol.symbol_id && e.kind == "CALLS")
                    .take(501)
                    .collect();
                truncated = callers.len() > 500 || callees.len() > 500;
                let references: Vec<_> = self
                    .snapshot
                    .edges
                    .iter()
                    .filter(|e| {
                        e.target_id.as_deref() == Some(&symbol.symbol_id) && e.kind == "REFERENCES"
                    })
                    .take(501)
                    .collect();
                let types: Vec<_> = self
                    .snapshot
                    .edges
                    .iter()
                    .filter(|e| e.source_id == symbol.symbol_id && e.kind == "USES_TYPE")
                    .take(501)
                    .collect();
                let associations = kortex_graph::traverse(
                    &self.snapshot,
                    &symbol.symbol_id,
                    Direction::Upstream,
                    &self.limits(args)?,
                    false,
                )?;
                truncated |= references.len() > 500 || types.len() > 500 || associations.truncated;
                let test_associations =
                    test_map::associations(self, &associations.items, deadline)?;
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
                let target = self.lookup(string(args, "symbol")?)?;
                let limits = self.limits(args)?;
                let direction = Direction::parse(
                    args.get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("upstream"),
                )?;
                let t = kortex_graph::traverse(
                    &self.snapshot,
                    &target.symbol_id,
                    direction,
                    &limits,
                    false,
                )?;
                truncated = t.truncated;
                let direct = t.items.iter().filter(|i| i.depth == 1).count();
                let indirect = t.items.len() - direct;
                let associations = test_map::associations(self, &t.items, deadline)?;
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
                let from = self.lookup(string(args, "from")?)?;
                let to = self.lookup(string(args, "to")?)?;
                let limits = self.limits(args)?;
                let t = kortex_graph::traverse(
                    &self.snapshot,
                    &from.symbol_id,
                    Direction::Downstream,
                    &limits,
                    false,
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
                let target = self.lookup(string(args, "symbol")?)?;
                let t = kortex_graph::traverse(
                    &self.snapshot,
                    &target.symbol_id,
                    Direction::Both,
                    &self.limits(args)?,
                    true,
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
            "changes" => self.changes(args)?,
            "routes" => {
                let result = intelligence::discover(self, args, deadline)?;
                let (result, cursor) = intelligence::paginate(self, args, result)?;
                next_cursor = cursor;
                result
            }
            "check" => rules::evaluate(self, args, deadline)?,
            _ => return Err(Error::new("UNKNOWN_OPERATION", operation, 2)),
        };
        if Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "operation exceeded query deadline",
                9,
            ));
        }
        data["schema_version"] = json!("1.0");
        data["operation"] = json!(operation);
        if operation == "check" && freshness != "fresh" {
            data["complete"] = json!(false);
            data["passed"] = json!(false);
            data["partial"] = json!(true);
            if data["exit_code"] == 0 {
                data["exit_code"] = json!(7);
            }
        }
        truncated |= data["truncated"].as_bool().unwrap_or(false);
        let partial = data["partial"].as_bool().unwrap_or(false)
            || self
                .snapshot
                .files
                .iter()
                .any(|f| !f.diagnostics.is_empty())
            || self
                .snapshot
                .diagnostics
                .iter()
                .any(|s| s.contains("truncated") || s.contains("refused"));
        let mut warnings = if freshness == "fresh" {
            vec![]
        } else {
            vec!["Index does not match repository state; citations refer to indexed generation."]
        };
        if operation == "changes"
            && let Some(items) = data["warnings"].as_array()
        {
            warnings.extend(items.iter().filter_map(Value::as_str));
        }
        let request_id = hash(format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut response = json!({"schema_version":"1.0","request_id":request_id,"repo_id":self.snapshot.repo_id,"generation_id":self.snapshot.generation_id,"index_status":freshness,"partial":partial,"truncated":truncated,"warnings":warnings,"data":data,"page":{"next_cursor":next_cursor},"timing_ms":started.elapsed().as_millis()});
        let budget = number(
            args,
            "max_response_bytes",
            self.config.mcp.max_response_bytes,
        )?;
        if budget > self.config.mcp.max_response_bytes {
            return Err(Error::new(
                "QUERY_LIMIT",
                "response budget exceeds repository policy",
                2,
            ));
        }
        while serde_json::to_vec(&response)?.len() > budget {
            if !matches!(operation, "query" | "routes" | "files")
                || response["data"]["items"]
                    .as_array()
                    .is_none_or(|items| items.len() <= 1)
            {
                return Err(Error::new(
                    "RESPONSE_BUDGET",
                    "response exceeds byte budget; narrow the query or reduce limit/depth",
                    7,
                ));
            }
            let items = response["data"]["items"].as_array_mut().unwrap();
            items.pop();
            let count = items.len();
            response["page"]["next_cursor"] = json!(contracts::cursor(
                self,
                operation,
                args,
                contracts::cursor_offset(self, operation, args)? + count
            ));
            response["truncated"] = json!(true);
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "QUERY_TIMEOUT",
                    "response encoding exceeded deadline",
                    9,
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "response encoding exceeded deadline",
                9,
            ));
        }
        Ok(response)
    }
    fn changes(&self, args: &Value) -> Result<Value> {
        self.change_analysis(args)
    }
}
fn commit(root: &Path, reference: &str) -> Result<String> {
    if reference.len() > 1024 {
        return Err(Error::new("INVALID_REF", "reference too long", 2));
    }
    Ok(kortex_repo::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
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
