//! Additive schema-1 projections over one already-pinned read-only WAL connection.
use crate::{Store, db_error};
use kortex_core::{Edge, Error, Result, Snapshot, Symbol};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
#[path = "read_projections.rs"]
mod projections;
pub(crate) use projections::write_projections;
pub use projections::{
    FileDiagnosticPresence, OrderedEdge, OrderedEdgePage, QueryCandidateFilter,
    QueryCandidateInventory, SymbolInventoryEntry,
};

/// Exact Snapshot header, not a substitute with missing diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub repo_id: String,
    pub root: String,
    pub generation_id: String,
    pub git_head: Option<String>,
    pub config_hash: String,
    pub grammar_hash: String,
    pub resolution_hash: String,
    pub diagnostics: Vec<String>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
}
impl SnapshotMetadata {
    pub(crate) fn from_snapshot(s: &Snapshot) -> Self {
        Self {
            repo_id: s.repo_id.clone(),
            root: s.root.clone(),
            generation_id: s.generation_id.clone(),
            git_head: s.git_head.clone(),
            config_hash: s.config_hash.clone(),
            grammar_hash: s.grammar_hash.clone(),
            resolution_hash: s.resolution_hash.clone(),
            diagnostics: s.diagnostics.clone(),
            file_count: s.files.len(),
            symbol_count: s.symbols().count(),
            edge_count: s.edges.len(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSource {
    Projection,
    /// Reading an old index required full Snapshot deserialization, on this connection.
    LegacySnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInventoryEntry {
    pub id: String,
    pub path: String,
    pub language: String,
    pub content_hash: String,
    pub size_bytes: usize,
}
/// All present predicates are ANDed. Strings and prefixes are case-sensitive literals.
#[derive(Debug, Default, Clone)]
pub struct SymbolFilter<'a> {
    pub name: Option<&'a str>,
    pub qualified_name: Option<&'a str>,
    pub name_prefix: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub kind: Option<&'a str>,
}
#[derive(Debug, Clone)]
pub struct ReadPage<T> {
    pub items: Vec<T>,
    /// Exclusive ID cursor; valid only within this generation, filter and direction.
    pub next_after_id: Option<String>,
}

pub struct ReadSession<'a> {
    store: &'a Store,
    metadata: SnapshotMetadata,
    source: MetadataSource,
}
fn corrupt(e: impl std::fmt::Display) -> Error {
    Error::new("CORRUPT_INDEX", e.to_string(), 6)
}
pub(crate) fn bounded_limit(limit: usize) -> Result<i64> {
    if !(1..=1000).contains(&limit) {
        return Err(Error::new("READ_LIMIT", "read limit must be 1..=1000", 2));
    }
    Ok(limit as i64)
}
impl Store {
    /// Reuses the read-only Store transaction; never opens a second connection.
    /// Writable Store handles are deliberately refused: their autocommit reads do not pin.
    pub fn read_session(&self) -> Result<ReadSession<'_>> {
        if self.conn.is_autocommit() || !self.conn.is_readonly("main").map_err(db_error)? {
            return Err(Error::new(
                "PINNED_READ_REQUIRED",
                "open a read-only Store for a pinned session",
                2,
            ));
        }
        // This first read pins both the generation and the WAL schema/pages.
        let (generation, repo): (String, String) = self.conn.query_row(
            "SELECT g.id,g.repo_id FROM index_state s JOIN generations g ON g.id=s.active_generation_id WHERE g.status='committed'",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional().map_err(db_error)?.ok_or_else(|| Error::new("INDEX_NOT_FOUND", "no committed generation", 3))?;
        let has_projection: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='generation_metadata')", [], |r| r.get(0)
        ).map_err(db_error)?;
        let raw: Option<String> = if has_projection {
            self.conn
                .query_row(
                    "SELECT metadata_json FROM generation_metadata WHERE generation_id=?1",
                    [&generation],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?
        } else {
            None
        };
        let (metadata, source) = match raw {
            Some(raw) => (
                serde_json::from_str::<SnapshotMetadata>(&raw).map_err(corrupt)?,
                MetadataSource::Projection,
            ),
            None => (
                SnapshotMetadata::from_snapshot(&self.load()?),
                MetadataSource::LegacySnapshot,
            ),
        };
        if metadata.generation_id != generation || metadata.repo_id != repo {
            return Err(corrupt("metadata identity differs from active generation"));
        }
        Ok(ReadSession {
            store: self,
            metadata,
            source,
        })
    }
}
impl ReadSession<'_> {
    pub fn metadata(&self) -> &SnapshotMetadata {
        &self.metadata
    }
    pub fn metadata_source(&self) -> MetadataSource {
        self.source
    }
    /// Column-only inventory, ordered by path then ID; no parse/symbol JSON is read.
    pub fn file_inventory(&self) -> Result<Vec<FileInventoryEntry>> {
        let mut stmt = self.store.conn.prepare("SELECT id,path,language,content_hash,size_bytes FROM files WHERE generation_id=?1 ORDER BY path,id").map_err(db_error)?;
        stmt.query_map([&self.metadata.generation_id], |r| {
            let size: i64 = r.get(4)?;
            let size_bytes = usize::try_from(size).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Integer,
                    Box::new(e),
                )
            })?;
            Ok(FileInventoryEntry {
                id: r.get(0)?,
                path: r.get(1)?,
                language: r.get(2)?,
                content_hash: r.get(3)?,
                size_bytes,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    pub fn get_symbol(&self, id: &str) -> Result<Option<Symbol>> {
        let raw: Option<String> = self
            .store
            .conn
            .query_row(
                "SELECT data_json FROM nodes WHERE generation_id=?1 AND id=?2",
                params![self.metadata.generation_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(corrupt))
            .transpose()
    }
    pub fn symbol_candidates(
        &self,
        filter: &SymbolFilter<'_>,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<ReadPage<Symbol>> {
        let bound = bounded_limit(limit)?;
        let mut stmt = self.store.conn.prepare("SELECT data_json FROM nodes WHERE generation_id=?1 AND (?2 IS NULL OR id>?2) AND (?3 IS NULL OR name=?3) AND (?4 IS NULL OR qualified_name=?4) AND (?5 IS NULL OR substr(name,1,length(?5))=?5) AND (?6 IS NULL OR file_path=?6) AND (?7 IS NULL OR kind=?7) ORDER BY id LIMIT ?8").map_err(db_error)?;
        let rows = stmt
            .query_map(
                params![
                    self.metadata.generation_id,
                    after_id,
                    filter.name,
                    filter.qualified_name,
                    filter.name_prefix,
                    filter.file_path,
                    filter.kind,
                    bound + 1
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?;
        let mut items = rows
            .map(|r| serde_json::from_str::<Symbol>(&r.map_err(db_error)?).map_err(corrupt))
            .collect::<Result<Vec<_>>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next_after_id = if more {
            items.last().map(|s| s.symbol_id.clone())
        } else {
            None
        };
        Ok(ReadPage {
            items,
            next_after_id,
        })
    }
    pub fn outgoing_edges(
        &self,
        id: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<ReadPage<Edge>> {
        self.edges(id, after_id, limit, false)
    }
    /// Matches actual targets only; ambiguous candidate IDs are not invented incoming edges.
    pub fn incoming_edges(
        &self,
        id: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<ReadPage<Edge>> {
        self.edges(id, after_id, limit, true)
    }
    fn edges(
        &self,
        id: &str,
        after_id: Option<&str>,
        limit: usize,
        incoming: bool,
    ) -> Result<ReadPage<Edge>> {
        let bound = bounded_limit(limit)?;
        let sql = if incoming {
            "SELECT data_json FROM edges WHERE generation_id=?1 AND target_id=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4"
        } else {
            "SELECT data_json FROM edges WHERE generation_id=?1 AND source_id=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4"
        };
        let mut stmt = self.store.conn.prepare(sql).map_err(db_error)?;
        let rows = stmt
            .query_map(
                params![self.metadata.generation_id, id, after_id, bound + 1],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?;
        let mut items = rows
            .map(|r| serde_json::from_str::<Edge>(&r.map_err(db_error)?).map_err(corrupt))
            .collect::<Result<Vec<_>>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next_after_id = if more {
            items.last().map(|e| e.id.clone())
        } else {
            None
        };
        Ok(ReadPage {
            items,
            next_after_id,
        })
    }
    pub fn keyword(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        bounded_limit(limit)?;
        self.store
            .keyword(&self.metadata.generation_id, query, limit)
    }
    /// Diagnostic digest of ALL actual FTS values, excluding the physical generation tag.
    /// O(symbols); not a latency-optimized query operation.
    pub fn keyword_fingerprint(&self) -> Result<String> {
        keyword_fingerprint(&self.store.conn, Some(&self.metadata.generation_id))
    }
    /// Explicit same-generation fallback for operations that still require all facts.
    pub fn load(&self) -> Result<Snapshot> {
        self.store.load()
    }
}

pub(crate) fn keyword_fingerprint(
    conn: &rusqlite::Connection,
    generation: Option<&str>,
) -> Result<String> {
    let sql = if generation.is_some() {
        "SELECT symbol_id,name,qualified_name,file_path,text FROM symbol_fts WHERE generation_id=?1 ORDER BY symbol_id"
    } else {
        "SELECT symbol_id,name,qualified_name,file_path,text FROM symbol_fts WHERE ?1 IS NULL ORDER BY symbol_id"
    };
    let mut stmt = conn.prepare(sql).map_err(corrupt)?;
    let rows = stmt
        .query_map([generation], |r| {
            Ok([
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ])
        })
        .map_err(corrupt)?;
    let mut previous = None;
    let mut hashes = Vec::new();
    for row in rows {
        let values = row.map_err(corrupt)?;
        if previous.as_ref() == Some(&values[0]) {
            return Err(corrupt("duplicate FTS identity"));
        }
        previous = Some(values[0].clone());
        hashes.push(kortex_core::hash(serde_json::to_vec(&values)?));
    }
    Ok(kortex_core::hash(serde_json::to_vec(&hashes)?))
}

pub(crate) fn write_metadata(tx: &rusqlite::Transaction<'_>, snapshot: &Snapshot) -> Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS generation_metadata(generation_id TEXT PRIMARY KEY REFERENCES generations(id) ON DELETE CASCADE,metadata_json TEXT NOT NULL)").map_err(db_error)?;
    tx.execute(
        "INSERT INTO generation_metadata(generation_id,metadata_json) VALUES(?1,?2)",
        params![
            snapshot.generation_id,
            serde_json::to_string(&SnapshotMetadata::from_snapshot(snapshot))?
        ],
    )
    .map_err(db_error)?;
    Ok(())
}
