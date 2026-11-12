//! Optional exact-order and diagnostic evidence, published atomically with schema 1.
use super::*;

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
impl ReadSession<'_> {
    /// Explicit evidence source for these newer projections, independent of header metadata.
    pub fn read_projection_source(&self) -> Result<MetadataSource> {
        if let Some(source) = self.projection_source.get() {
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
            let _ = self.projection_source.set(MetadataSource::LegacySnapshot);
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
        for (sql, count) in [
            (
                "SELECT count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),count(DISTINCT ordinal) FROM read_symbol_inventory WHERE generation_id=?1",
                m.symbols,
            ),
            (
                "SELECT count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),count(DISTINCT ordinal) FROM read_edge_order WHERE generation_id=?1",
                m.edges,
            ),
        ] {
            let (n, min, max, distinct): (i64, i64, i64, i64) = conn
                .query_row(sql, [&self.metadata.generation_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(corrupt)?;
            let count = integer(count)?;
            if n != count || distinct != count || min != 0 || max != count - 1 {
                return Err(corrupt("incomplete read order projection"));
            }
        }
        let n: i64 = conn.query_row("SELECT count(*) FROM read_file_diagnostics WHERE generation_id=?1 AND typeof(has_diagnostics)='integer' AND has_diagnostics IN(0,1)", [&self.metadata.generation_id], |r| r.get(0)).map_err(corrupt)?;
        if n != integer(m.files)? {
            return Err(corrupt("incomplete parser diagnostic evidence"));
        }
        if m.version == 2 {
            let (n,min,max,distinct): (i64,i64,i64,i64) = conn.query_row("SELECT count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1),count(DISTINCT ordinal) FROM read_file_order WHERE generation_id=?1", [&self.metadata.generation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(corrupt)?;
            let count = integer(m.files)?;
            if n != count || min != 0 || max != count - 1 || distinct != count {
                return Err(corrupt("incomplete file order evidence"));
            }
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
        let _ = self.projection_source.set(source);
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
    /// O(files), exact original Snapshot file-vector order, without ParsedFile JSON.
    /// Check read_projection_source first when full legacy materialization is undesirable.
    pub fn file_inventory_snapshot_order(&self) -> Result<Vec<FileInventoryEntry>> {
        if self.read_projection_source()? == MetadataSource::LegacySnapshot {
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
        if self.read_projection_source()? == MetadataSource::LegacySnapshot {
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
    Ok(())
}
