//! Optional exact-order and diagnostic evidence, published atomically with schema 1.
use super::*;
use kortex_core::RequestControl;

#[path = "query_projections.rs"]
mod query;

#[cfg(test)]
#[path = "read_candidate_control_tests.rs"]
mod control_tests;

fn check_control(control: Option<&RequestControl>) -> Result<()> {
    control.map_or(Ok(()), RequestControl::check)
}
fn candidate_row_checkpoint(control: Option<&RequestControl>) -> Result<()> {
    #[cfg(test)]
    control_tests::row_checkpoint();
    check_control(control)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInventoryEntry {
    pub symbol_id: String,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub language: String,
    pub kind: String,
    pub start_byte: usize,
}
impl SymbolInventoryEntry {
    fn from_symbol(s: &Symbol) -> Self {
        Self {
            symbol_id: s.symbol_id.clone(),
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            file_path: s.file_path.clone(),
            language: s.language.clone(),
            kind: s.kind.clone(),
            start_byte: s.span.start_byte,
        }
    }
}
/// Case-sensitive literal predicates, applied before ranking (not before the global FTS500).
#[derive(Debug, Default, Clone)]
pub struct QueryCandidateFilter<'a> {
    pub language: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub path_prefix: Option<&'a str>,
}
#[derive(Debug, Clone)]
pub struct QueryCandidateInventory {
    /// Exact lexical/FTS union after filters, in original Snapshot symbol order.
    pub items: Vec<SymbolInventoryEntry>,
    /// Original global FTS500, including entries excluded by the filters.
    pub fts: Vec<(String, f64)>,
    /// LegacySnapshot explicitly identifies the existing full same-Store fallback.
    pub source: MetadataSource,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnosticPresence {
    pub id: String,
    pub path: String,
    pub has_parser_diagnostics: bool,
}
#[derive(Debug, Clone)]
pub struct OrderedEdge {
    /// Zero-based position in the complete Snapshot.edges vector, not adjacency position.
    pub ordinal: usize,
    pub edge: Edge,
}
#[derive(Debug, Clone)]
pub struct OrderedEdgePage {
    pub items: Vec<OrderedEdge>,
    pub next_after_ordinal: Option<usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    files: usize,
    symbols: usize,
    edges: usize,
}
fn integer(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(corrupt)
}
fn unsigned(value: i64) -> Result<usize> {
    usize::try_from(value).map_err(corrupt)
}
fn validate_order(
    conn: &rusqlite::Connection,
    generation: &str,
    table: &str,
    expected: usize,
) -> Result<()> {
    // GROUP BY streams the ordinal index. Unlike the combined count(DISTINCT)/min/max
    // aggregate, it needs no second temporary B-tree and still detects duplicate ordinals.
    let sql = format!(
        "SELECT coalesce(sum(n),0),count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),coalesce(max(invalid),0) FROM (SELECT ordinal,count(*) n,typeof(ordinal)<>'integer' invalid FROM {table} WHERE generation_id=?1 GROUP BY ordinal)"
    );
    let (rows, distinct, min, max, invalid): (i64, i64, i64, i64, bool) = conn
        .query_row(&sql, [generation], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(corrupt)?;
    let expected = integer(expected)?;
    if rows != expected || distinct != expected || min != 0 || max != expected - 1 || invalid {
        return Err(corrupt("incomplete read order projection"));
    }
    Ok(())
}
impl ReadSession<'_> {
    /// Explicit evidence source for these newer projections, independent of header metadata.
    pub fn read_projection_source(&self) -> Result<MetadataSource> {
        if let Some(source) = self.store.projection_source.get() {
            return Ok(*source);
        }
        let conn = &self.store.conn;
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='generation_read_manifest')", [], |r| r.get(0)).map_err(corrupt)?;
        let raw: Option<String> = if exists {
            conn.query_row(
                "SELECT manifest_json FROM generation_read_manifest WHERE generation_id=?1",
                [&self.metadata.generation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(corrupt)?
        } else {
            None
        };
        let Some(raw) = raw else {
            // Old generations can lack all evidence. Partial NEW evidence is corruption,
            // not permission to downgrade to a legacy/clean interpretation.
            for table in [
                "read_symbol_inventory",
                "read_edge_order",
                "read_file_diagnostics",
                "read_file_order",
            ] {
                let exists: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                        [table],
                        |r| r.get(0),
                    )
                    .map_err(corrupt)?;
                if exists {
                    let present: bool = conn
                        .query_row(
                            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE generation_id=?1)"),
                            [&self.metadata.generation_id],
                            |r| r.get(0),
                        )
                        .map_err(corrupt)?;
                    if present {
                        return Err(corrupt("read projection rows lack their manifest"));
                    }
                }
            }
            let _ = self
                .store
                .projection_source
                .set(MetadataSource::LegacySnapshot);
            return Ok(MetadataSource::LegacySnapshot);
        };
        let m: Manifest = serde_json::from_str(&raw).map_err(corrupt)?;
        if ![1, 2].contains(&m.version)
            || m.files != self.metadata.file_count
            || m.symbols != self.metadata.symbol_count
            || m.edges != self.metadata.edge_count
        {
            return Err(corrupt("read projection manifest mismatch"));
        }
        // Completeness is evidence, not an assumption from the existence of a table.
        for (table, count) in [
            ("read_symbol_inventory", m.symbols),
            ("read_edge_order", m.edges),
        ] {
            validate_order(conn, &self.metadata.generation_id, table, count)?;
        }
        let n: i64 = conn.query_row("SELECT count(*) FROM read_file_diagnostics WHERE generation_id=?1 AND typeof(has_diagnostics)='integer' AND has_diagnostics IN(0,1)", [&self.metadata.generation_id], |r| r.get(0)).map_err(corrupt)?;
        if n != integer(m.files)? {
            return Err(corrupt("incomplete parser diagnostic evidence"));
        }
        if m.version == 2 {
            validate_order(
                conn,
                &self.metadata.generation_id,
                "read_file_order",
                m.files,
            )?;
            let broken: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM read_file_order p LEFT JOIN files f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 AND f.id IS NULL)", [&self.metadata.generation_id], |r|r.get(0)).map_err(corrupt)?;
            if broken {
                return Err(corrupt("file order identity mismatch"));
            }
        }
        // Verify normalized identities too: a missing join must not become a clean/empty page.
        let broken: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM read_symbol_inventory p LEFT JOIN nodes n ON n.id=p.id AND n.generation_id=p.generation_id WHERE p.generation_id=?1 AND (n.id IS NULL OR typeof(p.start_byte)<>'integer' OR p.start_byte<0 OR typeof(p.language)<>'text')) OR EXISTS(SELECT 1 FROM read_edge_order p LEFT JOIN edges e ON e.id=p.id AND e.generation_id=p.generation_id WHERE p.generation_id=?1 AND e.id IS NULL) OR EXISTS(SELECT 1 FROM read_file_diagnostics p LEFT JOIN files f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 AND f.id IS NULL)", [&self.metadata.generation_id], |r| r.get(0)).map_err(corrupt)?;
        if broken {
            return Err(corrupt("read projection identity/type mismatch"));
        }
        // Version 1 had no file-vector order evidence. Do not silently use sorted paths
        // for freshness or conceal a full load from a caller deciding whether to go eager.
        let source = if m.version == 2 {
            MetadataSource::Projection
        } else {
            MetadataSource::LegacySnapshot
        };
        let _ = self.store.projection_source.set(source);
        Ok(source)
    }
    /// O(symbols) lightweight rows in Snapshot order. Apply Rust Unicode lexical semantics
    /// BEFORE pagination; no ASCII-only SQLite lower/collation is substituted here.
    pub fn symbol_inventory(&self) -> Result<Vec<SymbolInventoryEntry>> {
        if self.read_projection_source()? == MetadataSource::LegacySnapshot {
            return Ok(self
                .load()?
                .symbols()
                .map(SymbolInventoryEntry::from_symbol)
                .collect());
        }
        let mut stmt = self.store.conn.prepare("SELECT n.id,n.name,n.qualified_name,n.file_path,p.language,n.kind,p.start_byte FROM read_symbol_inventory p JOIN nodes n ON n.id=p.id AND n.generation_id=p.generation_id WHERE p.generation_id=?1 ORDER BY p.ordinal").map_err(corrupt)?;
        let rows = stmt
            .query_map([&self.metadata.generation_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(corrupt)?;
        rows.map(|r| {
            let (symbol_id, name, qualified_name, file_path, language, kind, start) =
                r.map_err(corrupt)?;
            Ok(SymbolInventoryEntry {
                symbol_id,
                name,
                qualified_name,
                file_path,
                language,
                kind,
                start_byte: unsigned(start)?,
            })
        })
        .collect()
    }
    /// Streams all lightweight columns but allocates only exact lexical/FTS candidates.
    /// Rust Unicode lowercase is evaluated on actual UTF-8 strings, never SQL lower/LIKE.
    /// Legacy projections retain the explicit same-Store inventory/full fallback.
    pub fn query_candidate_inventory(
        &self,
        text: &str,
        filter: &QueryCandidateFilter<'_>,
    ) -> Result<QueryCandidateInventory> {
        self.query_candidate_inventory_inner(text, filter, None, false)
    }
    /// Same exact inventory with the caller's cooperative cancellation/deadline.
    /// Each row/filter step checks control; one SQLite step, legacy load, or individual
    /// Unicode lowercase operation remains indivisible (no hard preemption claim).
    pub fn query_candidate_inventory_with_control(
        &self,
        text: &str,
        filter: &QueryCandidateFilter<'_>,
        control: &RequestControl,
    ) -> Result<QueryCandidateInventory> {
        self.query_candidate_inventory_inner(text, filter, Some(control), false)
    }
    pub(super) fn query_candidate_inventory_inner(
        &self,
        text: &str,
        filter: &QueryCandidateFilter<'_>,
        control: Option<&RequestControl>,
        query_scope: bool,
    ) -> Result<QueryCandidateInventory> {
        check_control(control)?;
        if text.len() > 4096 || text.trim().is_empty() {
            return Err(Error::new("QUERY_LIMIT", "query 1..4096 bytes required", 2));
        }
        let (source, fast) = if query_scope {
            self.query_projection()?
        } else {
            (self.read_projection_source()?, false)
        };
        check_control(control)?;
        let fts = self.keyword(text, 500)?;
        check_control(control)?;
        let needed: std::collections::BTreeSet<_> = fts.iter().map(|(id, _)| id.as_str()).collect();
        let lower = text.to_lowercase();
        let matches =
            |id: &str, name: &str, qualified: &str, path: &str, language: &str, kind: &str| {
                filter.language.is_none_or(|v| language == v)
                    && filter.kind.is_none_or(|v| kind == v)
                    && filter.path_prefix.is_none_or(|v| path.starts_with(v))
                    && (needed.contains(id)
                        || name == text
                        || qualified == text
                        || name.to_lowercase().starts_with(&lower)
                        || qualified.to_lowercase().contains(&lower)
                        || path.to_lowercase().contains(&lower))
            };
        let items = if source == MetadataSource::LegacySnapshot {
            let inventory = if query_scope {
                self.load()?
                    .symbols()
                    .map(SymbolInventoryEntry::from_symbol)
                    .collect()
            } else {
                self.symbol_inventory()?
            };
            check_control(control)?;
            let mut items = Vec::new();
            for s in inventory {
                candidate_row_checkpoint(control)?;
                if matches(
                    &s.symbol_id,
                    &s.name,
                    &s.qualified_name,
                    &s.file_path,
                    &s.language,
                    &s.kind,
                ) {
                    items.push(s);
                }
            }
            items
        } else {
            let sql = if fast {
                "SELECT id,name,qualified_name,file_path,language,kind,start_byte FROM query_symbol_inventory WHERE generation_id=?1 ORDER BY ordinal"
            } else {
                "SELECT n.id,n.name,n.qualified_name,n.file_path,p.language,n.kind,p.start_byte FROM read_symbol_inventory p JOIN nodes n ON n.id=p.id AND n.generation_id=p.generation_id WHERE p.generation_id=?1 ORDER BY p.ordinal"
            };
            let mut stmt = self.store.conn.prepare(sql).map_err(corrupt)?;
            let mut rows = stmt
                .query([&self.metadata.generation_id])
                .map_err(corrupt)?;
            let mut items = Vec::new();
            while let Some(row) = rows.next().map_err(corrupt)? {
                candidate_row_checkpoint(control)?;
                // Validate even excluded rows: malformed UTF-8/types must not be hidden by filters.
                let id = row.get_ref(0).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let name = row.get_ref(1).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let qualified = row.get_ref(2).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let path = row.get_ref(3).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let language = row.get_ref(4).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let kind = row.get_ref(5).map_err(corrupt)?.as_str().map_err(corrupt)?;
                let start_byte = unsigned(row.get(6).map_err(corrupt)?)?;
                if matches(id, name, qualified, path, language, kind) {
                    items.push(SymbolInventoryEntry {
                        symbol_id: id.into(),
                        name: name.into(),
                        qualified_name: qualified.into(),
                        file_path: path.into(),
                        language: language.into(),
                        kind: kind.into(),
                        start_byte,
                    });
                }
            }
            items
        };
        check_control(control)?;
        Ok(QueryCandidateInventory { items, fts, source })
    }
    /// O(files), exact original Snapshot file-vector order, without ParsedFile JSON.
    /// Check read_projection_source first when full legacy materialization is undesirable.
    pub fn file_inventory_snapshot_order(&self) -> Result<Vec<FileInventoryEntry>> {
        self.file_inventory_for_source(self.read_projection_source()?)
    }
    pub(super) fn file_inventory_for_source(
        &self,
        source: MetadataSource,
    ) -> Result<Vec<FileInventoryEntry>> {
        if source == MetadataSource::LegacySnapshot {
            let snapshot = self.load()?;
            return Ok(snapshot
                .files
                .into_iter()
                .map(|f| FileInventoryEntry {
                    id: kortex_core::id(&[&snapshot.repo_id, &f.path]),
                    path: f.path,
                    language: f.language,
                    content_hash: f.content_hash,
                    size_bytes: f.size_bytes,
                })
                .collect());
        }
        let mut stmt = self.store.conn.prepare("SELECT f.id,f.path,f.language,f.content_hash,f.size_bytes FROM read_file_order p JOIN files f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 ORDER BY p.ordinal").map_err(corrupt)?;
        stmt.query_map([&self.metadata.generation_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(corrupt)?
        .map(|row| {
            let (id, path, language, content_hash, size) = row.map_err(corrupt)?;
            Ok(FileInventoryEntry {
                id,
                path,
                language,
                content_hash,
                size_bytes: unsigned(size)?,
            })
        })
        .collect()
    }
    /// Every file has explicit true/false evidence; absence of top-level diagnostics is irrelevant.
    pub fn parser_diagnostic_presence(&self) -> Result<Vec<FileDiagnosticPresence>> {
        self.diagnostics_for_source(self.read_projection_source()?)
    }
    pub(super) fn diagnostics_for_source(
        &self,
        source: MetadataSource,
    ) -> Result<Vec<FileDiagnosticPresence>> {
        if source == MetadataSource::LegacySnapshot {
            let snapshot = self.load()?;
            let mut rows: Vec<_> = snapshot
                .files
                .iter()
                .map(|f| FileDiagnosticPresence {
                    id: kortex_core::id(&[&snapshot.repo_id, &f.path]),
                    path: f.path.clone(),
                    has_parser_diagnostics: !f.diagnostics.is_empty(),
                })
                .collect();
            rows.sort_by(|a, b| (&a.path, &a.id).cmp(&(&b.path, &b.id)));
            return Ok(rows);
        }
        let mut stmt = self.store.conn.prepare("SELECT f.id,f.path,p.has_diagnostics FROM read_file_diagnostics p JOIN files f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 ORDER BY f.path,f.id").map_err(corrupt)?;
        stmt.query_map([&self.metadata.generation_id], |r| {
            Ok(FileDiagnosticPresence {
                id: r.get(0)?,
                path: r.get(1)?,
                has_parser_diagnostics: r.get::<_, i64>(2)? == 1,
            })
        })
        .map_err(corrupt)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(corrupt)
    }
    pub fn outgoing_edges_ordered(
        &self,
        id: &str,
        after_ordinal: Option<usize>,
        limit: usize,
    ) -> Result<OrderedEdgePage> {
        self.edges_ordered(id, after_ordinal, limit, false)
    }
    /// Actual target only, not ambiguous candidates. Original global edge order is retained.
    pub fn incoming_edges_ordered(
        &self,
        id: &str,
        after_ordinal: Option<usize>,
        limit: usize,
    ) -> Result<OrderedEdgePage> {
        self.edges_ordered(id, after_ordinal, limit, true)
    }
    fn edges_ordered(
        &self,
        id: &str,
        after: Option<usize>,
        limit: usize,
        incoming: bool,
    ) -> Result<OrderedEdgePage> {
        let bound = bounded_limit(limit)?;
        let cursor = after.map(integer).transpose()?;
        let mut items = if self.read_projection_source()? == MetadataSource::LegacySnapshot {
            self.load()?
                .edges
                .into_iter()
                .enumerate()
                .filter(|(ordinal, e)| {
                    after.is_none_or(|a| *ordinal > a)
                        && if incoming {
                            e.target_id.as_deref() == Some(id)
                        } else {
                            e.source_id == id
                        }
                })
                .take(limit + 1)
                .map(|(ordinal, edge)| OrderedEdge { ordinal, edge })
                .collect::<Vec<_>>()
        } else {
            let sql = if incoming {
                "SELECT p.ordinal,e.data_json FROM edges e JOIN read_edge_order p ON p.id=e.id AND p.generation_id=e.generation_id WHERE e.generation_id=?1 AND e.target_id=?2 AND (?3 IS NULL OR p.ordinal>?3) ORDER BY p.ordinal LIMIT ?4"
            } else {
                "SELECT p.ordinal,e.data_json FROM edges e JOIN read_edge_order p ON p.id=e.id AND p.generation_id=e.generation_id WHERE e.generation_id=?1 AND e.source_id=?2 AND (?3 IS NULL OR p.ordinal>?3) ORDER BY p.ordinal LIMIT ?4"
            };
            let mut stmt = self.store.conn.prepare(sql).map_err(corrupt)?;
            stmt.query_map(
                params![self.metadata.generation_id, id, cursor, bound + 1],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(corrupt)?
            .map(|r| {
                let (ordinal, raw) = r.map_err(corrupt)?;
                Ok(OrderedEdge {
                    ordinal: unsigned(ordinal)?,
                    edge: serde_json::from_str(&raw).map_err(corrupt)?,
                })
            })
            .collect::<Result<Vec<_>>>()?
        };
        let more = items.len() > limit;
        items.truncate(limit);
        let next_after_ordinal = if more {
            items.last().map(|e| e.ordinal)
        } else {
            None
        };
        Ok(OrderedEdgePage {
            items,
            next_after_ordinal,
        })
    }
}

pub(crate) fn write_projections(tx: &rusqlite::Transaction<'_>, s: &Snapshot) -> Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS generation_read_manifest(generation_id TEXT PRIMARY KEY REFERENCES generations(id) ON DELETE CASCADE,manifest_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS read_symbol_inventory(generation_id TEXT NOT NULL,id TEXT NOT NULL,ordinal INTEGER NOT NULL CHECK(ordinal>=0),language TEXT NOT NULL,start_byte INTEGER NOT NULL CHECK(start_byte>=0),PRIMARY KEY(generation_id,id),UNIQUE(generation_id,ordinal),FOREIGN KEY(id,generation_id) REFERENCES nodes(id,generation_id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS read_edge_order(generation_id TEXT NOT NULL,id TEXT NOT NULL,ordinal INTEGER NOT NULL CHECK(ordinal>=0),PRIMARY KEY(generation_id,id),UNIQUE(generation_id,ordinal),FOREIGN KEY(id,generation_id) REFERENCES edges(id,generation_id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS read_file_diagnostics(generation_id TEXT NOT NULL,id TEXT NOT NULL,has_diagnostics INTEGER NOT NULL CHECK(has_diagnostics IN(0,1)),PRIMARY KEY(generation_id,id),FOREIGN KEY(id,generation_id) REFERENCES files(id,generation_id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS read_file_order(generation_id TEXT NOT NULL,id TEXT NOT NULL,ordinal INTEGER NOT NULL CHECK(ordinal>=0),PRIMARY KEY(generation_id,id),UNIQUE(generation_id,ordinal),FOREIGN KEY(id,generation_id) REFERENCES files(id,generation_id) ON DELETE CASCADE);").map_err(db_error)?;
    {
        let mut nodes = tx
            .prepare("INSERT INTO read_symbol_inventory VALUES(?1,?2,?3,?4,?5)")
            .map_err(db_error)?;
        for (ordinal, node) in s.symbols().enumerate() {
            nodes
                .execute(params![
                    s.generation_id,
                    node.symbol_id,
                    integer(ordinal)?,
                    node.language,
                    integer(node.span.start_byte)?
                ])
                .map_err(db_error)?;
        }
        let mut edges = tx
            .prepare("INSERT INTO read_edge_order VALUES(?1,?2,?3)")
            .map_err(db_error)?;
        for (ordinal, edge) in s.edges.iter().enumerate() {
            edges
                .execute(params![s.generation_id, edge.id, integer(ordinal)?])
                .map_err(db_error)?;
        }
        let mut files = tx
            .prepare("INSERT INTO read_file_diagnostics VALUES(?1,?2,?3)")
            .map_err(db_error)?;
        let mut file_order = tx
            .prepare("INSERT INTO read_file_order VALUES(?1,?2,?3)")
            .map_err(db_error)?;
        for (ordinal, file) in s.files.iter().enumerate() {
            file_order
                .execute(params![
                    s.generation_id,
                    kortex_core::id(&[&s.repo_id, &file.path]),
                    integer(ordinal)?
                ])
                .map_err(db_error)?;
            files
                .execute(params![
                    s.generation_id,
                    kortex_core::id(&[&s.repo_id, &file.path]),
                    !file.diagnostics.is_empty()
                ])
                .map_err(db_error)?;
        }
    }
    tx.execute(
        "INSERT INTO generation_read_manifest VALUES(?1,?2)",
        params![
            s.generation_id,
            serde_json::to_string(&Manifest {
                version: 2,
                files: s.files.len(),
                symbols: s.symbols().count(),
                edges: s.edges.len()
            })?
        ],
    )
    .map_err(db_error)?;
    query::write_query_projections(tx, s)?;
    Ok(())
}
