//! Pinned lazy query dispatch beside the eagerly public Service SDK compatibility API.
use crate::{
    Service, contracts::CursorContext, execution, number, resolve_registered_selection,
    resolve_selection, string,
};
use kortex_config::Config;
use kortex_core::{Edge, Error, RequestControl, Result, Snapshot, Symbol, id, public_path};
use kortex_storage::{
    FileInventoryEntry, MetadataSource, ReadPage, SnapshotMetadata, Store, SymbolFilter,
};
use serde_json::{Value, json};
use std::{
    cell::{OnceCell, RefCell},
    path::{Path, PathBuf},
    time::Duration,
};

pub struct ReadService {
    pub root: PathBuf,
    pub config: Config,
    // SQLite Store is Send, not Sync. Interior state permits repeated MCP wire-budget
    // retries while moving exactly this connection into an explicit eager fallback.
    store: RefCell<Option<Store>>,
    eager: OnceCell<Service>,
    metadata: SnapshotMetadata,
    source: MetadataSource,
}
impl ReadService {
    pub fn open_selected(selector: Option<&str>, config_path: Option<&Path>) -> Result<Self> {
        let (root, config) = resolve_selection(selector, config_path)?;
        Self::open_at_root(&root, config)
    }
    pub fn open_registered(
        repo_id: &str,
        authorized_root: &Path,
        config_path: Option<&Path>,
    ) -> Result<Self> {
        let (root, config) = resolve_registered_selection(repo_id, authorized_root, config_path)?;
        Self::open_at_root(&root, config)
    }
    /// One read-only Store pinned by metadata. Legacy metadata may truthfully decode
    /// the full Snapshot; metadata_source reports that fallback, never invented headers.
    pub fn open_at_root(root: &Path, config: Config) -> Result<Self> {
        let (root, store) = super::open_store_at_root(root, &config)?;
        let session = store.read_session()?;
        let metadata = session.metadata().clone();
        let source = session.metadata_source();
        if metadata.root != public_path(&root) {
            return Err(Error::new(
                "INDEX_RELOCATED",
                "index belongs to another root; rebuild with index --full",
                6,
            ));
        }
        if metadata.repo_id != id(&["repository", &public_path(&root)]) {
            return Err(Error::new(
                "REPO_FORBIDDEN",
                "selection/index identity mismatch",
                8,
            ));
        }
        Ok(Self {
            root,
            config,
            store: RefCell::new(Some(store)),
            eager: OnceCell::new(),
            metadata,
            source,
        })
    }
    pub fn metadata(&self) -> &SnapshotMetadata {
        &self.metadata
    }
    pub fn metadata_source(&self) -> MetadataSource {
        self.source
    }
    /// True only after an unsupported/legacy operation explicitly materialized this reader.
    pub fn eager_materialized(&self) -> bool {
        self.eager.get().is_some()
    }
    fn with_store<T>(&self, read: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        if let Some(eager) = self.eager.get() {
            return read(&eager.store);
        }
        let borrowed = self.store.borrow();
        let store = borrowed
            .as_ref()
            .ok_or_else(|| Error::new("INTERNAL_STATE", "pinned Store is unavailable", 1))?;
        read(store)
    }
    fn check_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        if snapshot.repo_id != self.metadata.repo_id
            || snapshot.generation_id != self.metadata.generation_id
            || snapshot.root != self.metadata.root
        {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "snapshot differs from pinned metadata",
                6,
            ));
        }
        Ok(())
    }
    fn materialize(&self, control: &RequestControl) -> Result<&Service> {
        control.check()?;
        if self.eager.get().is_none() {
            let snapshot = self.with_store(|store| store.read_session()?.load())?;
            control.check()?;
            self.check_snapshot(&snapshot)?;
            // No fallible I/O or user callback occurs between taking and retaining Store.
            let store =
                self.store.borrow_mut().take().ok_or_else(|| {
                    Error::new("INTERNAL_STATE", "pinned Store is unavailable", 1)
                })?;
            let service = Service {
                root: self.root.clone(),
                config: self.config.clone(),
                store,
                snapshot,
            };
            self.eager
                .set(service)
                .map_err(|_| Error::new("INTERNAL_STATE", "reader was already materialized", 1))?;
        }
        self.eager
            .get()
            .ok_or_else(|| Error::new("INTERNAL_STATE", "eager reader is unavailable", 1))
    }
    pub fn file_inventory(&self, control: &RequestControl) -> Result<Vec<FileInventoryEntry>> {
        control.check()?;
        let inventory = self.with_store(|store| store.read_session()?.file_inventory())?;
        control.check()?;
        Ok(inventory)
    }
    pub fn get_symbol(&self, symbol_id: &str, control: &RequestControl) -> Result<Option<Symbol>> {
        control.check()?;
        let symbol = self.with_store(|store| store.read_session()?.get_symbol(symbol_id))?;
        control.check()?;
        Ok(symbol)
    }
    pub fn symbol_candidates(
        &self,
        filter: &SymbolFilter<'_>,
        after_id: Option<&str>,
        limit: usize,
        control: &RequestControl,
    ) -> Result<ReadPage<Symbol>> {
        control.check()?;
        let page = self.with_store(|store| {
            store
                .read_session()?
                .symbol_candidates(filter, after_id, limit)
        })?;
        control.check()?;
        Ok(page)
    }
    pub fn outgoing_edges(
        &self,
        symbol_id: &str,
        after_id: Option<&str>,
        limit: usize,
        control: &RequestControl,
    ) -> Result<ReadPage<Edge>> {
        control.check()?;
        let page = self.with_store(|store| {
            store
                .read_session()?
                .outgoing_edges(symbol_id, after_id, limit)
        })?;
        control.check()?;
        Ok(page)
    }
    pub fn incoming_edges(
        &self,
        symbol_id: &str,
        after_id: Option<&str>,
        limit: usize,
        control: &RequestControl,
    ) -> Result<ReadPage<Edge>> {
        control.check()?;
        let page = self.with_store(|store| {
            store
                .read_session()?
                .incoming_edges(symbol_id, after_id, limit)
        })?;
        control.check()?;
        Ok(page)
    }
    pub fn keyword(
        &self,
        text: &str,
        limit: usize,
        control: &RequestControl,
    ) -> Result<Vec<(String, f64)>> {
        control.check()?;
        let page = self.with_store(|store| store.read_session()?.keyword(text, limit))?;
        control.check()?;
        Ok(page)
    }
    pub fn freshness(&self) -> Result<String> {
        let control =
            RequestControl::new(Duration::from_millis(self.config.analysis.query_timeout_ms))?;
        self.freshness_with_control(&control)
    }
    pub fn freshness_with_control(&self, control: &RequestControl) -> Result<String> {
        control.check()?;
        if let Some(eager) = self.eager.get() {
            return eager.freshness_with_control(control);
        }
        let freshness = self.with_store(|store| {
            let session = store.read_session()?;
            if session.read_projection_source()? == MetadataSource::LegacySnapshot {
                return Ok(None);
            }
            self.projected_freshness(&session, control).map(Some)
        })?;
        match freshness {
            Some(value) => Ok(value),
            None => self.materialize(control)?.freshness_with_control(control),
        }
    }
    fn projected_freshness(
        &self,
        session: &kortex_storage::ReadSession<'_>,
        control: &RequestControl,
    ) -> Result<String> {
        if !execution::freshness_header(
            &self.root,
            &self.config,
            execution::FreshnessHeader {
                resolution_hash: &self.metadata.resolution_hash,
                config_hash: &self.metadata.config_hash,
                grammar_hash: &self.metadata.grammar_hash,
                git_head: &self.metadata.git_head,
            },
            control,
        )? {
            return Ok("stale".into());
        }
        let files = session.file_inventory_snapshot_order()?;
        control.check()?;
        execution::freshness_files(
            &self.root,
            &self.config,
            files
                .iter()
                .map(|f| (f.path.as_str(), f.content_hash.as_str())),
            control,
        )
    }
    pub fn execute(&self, operation: &str, args: &Value) -> Result<Value> {
        let control =
            RequestControl::new(Duration::from_millis(self.config.analysis.query_timeout_ms))?;
        self.execute_with_control(operation, args, &control)
    }
    /// Shared versioned operation dispatch. Query uses real projections; unsupported
    /// operations/legacy projections explicitly materialize the SAME pinned Store.
    /// Retained fallback supports repeated MCP budget retries without reopening a reader.
    pub fn execute_with_control(
        &self,
        operation: &str,
        args: &Value,
        control: &RequestControl,
    ) -> Result<Value> {
        control.check()?;
        if let Some(eager) = self.eager.get() {
            return eager.execute_with_control(operation, args, control);
        }
        if operation != "query" {
            return self
                .materialize(control)?
                .execute_with_control(operation, args, control);
        }
        let context = CursorContext {
            repo_id: &self.metadata.repo_id,
            generation_id: &self.metadata.generation_id,
            config: &self.config,
        };
        let (started, control) = execution::prepare(&context, operation, args, control)?;
        let response = self.with_store(|store| {
            let session = store.read_session()?;
            if session.read_projection_source()? == MetadataSource::LegacySnapshot { return Ok(None); }
            let freshness = self.projected_freshness(&session, &control)?;
            control.check()?;
            if args.get("require_fresh").and_then(Value::as_bool) == Some(true) && freshness != "fresh" {
                return Err(Error::new("STALE_INDEX", "reindex before querying", 5));
            }
            if self.config.search.embeddings != "disabled" {
                return Err(Error::new("LOCAL_MODEL_GATE", "local inference is experimental and automatic transport activation is gated on pretrained-artifact and relevance acceptance; use explicit SDK provider integration or set search.embeddings=disabled", 7));
            }
            let limit = number(args, "limit", 20)?;
            let offset = context.cursor_offset(operation, args)?;
            let filters = kortex_search::Filters {
                language: args.get("language").and_then(Value::as_str),
                kind: args.get("kind").and_then(Value::as_str),
                path_prefix: args.get("path_prefix").and_then(Value::as_str),
            };
            let (hits, more) = kortex_search::projected::query_projected_with_control(&session, string(args, "text")?, limit, offset, &filters, &control)?;
            let presence = session.parser_diagnostic_presence()?;
            control.check()?;
            let indexed_partial = presence.iter().any(|f| f.has_parser_diagnostics)
                || self.metadata.diagnostics.iter().any(|s| s.contains("truncated") || s.contains("refused"));
            execution::finish(&context, operation, args, execution::ResponseParts {
                started, freshness, indexed_partial, truncated: false,
                next_cursor: more.then(|| context.cursor(operation, args, offset + limit)),
                data: json!({"items":hits}),
            }, &control).map(Some)
        })?;
        match response {
            Some(value) => Ok(value),
            None => self
                .materialize(&control)?
                .execute_with_control(operation, args, &control),
        }
    }
    /// Explicit SDK compatibility conversion, consuming this SAME pinned Store.
    pub fn into_eager(self, control: &RequestControl) -> Result<Service> {
        control.check()?;
        if let Some(eager) = self.eager.into_inner() {
            return Ok(eager);
        }
        let store = self
            .store
            .into_inner()
            .ok_or_else(|| Error::new("INTERNAL_STATE", "pinned Store is unavailable", 1))?;
        let snapshot = store.read_session()?.load()?;
        control.check()?;
        if snapshot.repo_id != self.metadata.repo_id
            || snapshot.generation_id != self.metadata.generation_id
            || snapshot.root != self.metadata.root
        {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "snapshot differs from pinned metadata",
                6,
            ));
        }
        Ok(Service {
            root: self.root,
            config: self.config,
            store,
            snapshot,
        })
    }
}
