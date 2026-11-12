//! Prepared, transaction-scoped publication. Durability and FK policy stay enabled.
use crate::{Store, WriterLock, db_error};
use kortex_core::{Error, Result, Snapshot};
use rusqlite::params;
use std::time::{Duration, Instant};

/// Wall-clock phase observations, not acceptance percentiles. No source/config data.
#[derive(Debug, Default, Clone)]
pub struct CommitTimings {
    pub snapshot_serialization: Duration,
    pub generation_insert: Duration,
    pub secondary_indexes_drop: Duration,
    pub secondary_indexes_build: Duration,
    pub files_nodes_fts_insert: Duration,
    pub edges_sort: Duration,
    pub edges_insert: Duration,
    pub read_projections_insert: Duration,
    pub publication: Duration,
    pub fts_delete: Duration,
    pub edges_delete: Duration,
    pub nodes_delete: Duration,
    pub files_delete: Duration,
    pub generations_delete: Duration,
    /// COMMIT includes FULL durability and any synchronous automatic WAL checkpoint.
    pub durable_commit: Duration,
}

// Must exactly match schema 1. No UNIQUE index or FK parent index may be listed.
const LOOKUP_INDEXES: &[(&str, &str)] = &[
    (
        "idx_nodes_lookup",
        "CREATE INDEX idx_nodes_lookup ON nodes(generation_id,kind,name)",
    ),
    (
        "idx_nodes_file",
        "CREATE INDEX idx_nodes_file ON nodes(generation_id,file_path)",
    ),
    (
        "idx_nodes_qualified",
        "CREATE INDEX idx_nodes_qualified ON nodes(generation_id,qualified_name)",
    ),
    (
        "idx_edges_source",
        "CREATE INDEX idx_edges_source ON edges(generation_id,source_id,kind)",
    ),
    (
        "idx_edges_target",
        "CREATE INDEX idx_edges_target ON edges(generation_id,target_id,kind)",
    ),
];

fn retire_children(
    tx: &rusqlite::Transaction<'_>,
    generation: &str,
    timings: &mut CommitTimings,
) -> Result<()> {
    let start = Instant::now();
    tx.execute(
        "DELETE FROM symbol_fts WHERE generation_id<>?1",
        [generation],
    )
    .map_err(db_error)?;
    timings.fts_delete = start.elapsed();
    // Edges precede nodes: all FK checks stay enabled, without endpoint cascade
    // amplification. Files have no node references in schema 1.
    let start = Instant::now();
    tx.execute("DELETE FROM edges WHERE generation_id<>?1", [generation])
        .map_err(db_error)?;
    timings.edges_delete = start.elapsed();
    let start = Instant::now();
    tx.execute("DELETE FROM nodes WHERE generation_id<>?1", [generation])
        .map_err(db_error)?;
    timings.nodes_delete = start.elapsed();
    let start = Instant::now();
    tx.execute("DELETE FROM files WHERE generation_id<>?1", [generation])
        .map_err(db_error)?;
    timings.files_delete = start.elapsed();
    Ok(())
}

impl Store {
    /// Same atomic operation as commit, with coarse phase timings for diagnostics.
    pub fn commit_profiled(
        &mut self,
        snapshot: &Snapshot,
        lock: &WriterLock,
    ) -> Result<CommitTimings> {
        if lock.directory != self.directory {
            return Err(Error::new(
                "WRITER_LOCK_MISMATCH",
                "writer lock belongs to another repository",
                8,
            ));
        }
        crate::location::check_index_directory(&self.directory, "repository", false)?;
        for name in [
            "index.db",
            "index.db-wal",
            "index.db-shm",
            "index.db-journal",
            "writer.lock",
        ] {
            crate::location::check_file(&self.directory.join(name))?;
        }
        // SQLite's ~2 MiB default thrashes the graph's random-ID primary and FK
        // indexes at reference scale. A bounded connection-local 64 MiB cache
        // reduces dirty-page churn; spill, WAL, FULL sync and all FK checks stay on.
        self.conn
            .pragma_update(None, "cache_size", -65_536)
            .map_err(db_error)?;
        let mut timings = CommitTimings::default();
        let start = Instant::now();
        let raw = serde_json::to_string(snapshot)?;
        timings.snapshot_serialization = start.elapsed();
        let tx = self.conn.transaction().map_err(db_error)?;
        let start = Instant::now();
        tx.execute(
            "INSERT OR IGNORE INTO repositories(id,root,created_at) VALUES(?1,?2,datetime('now'))",
            params![snapshot.repo_id, snapshot.root],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO generations(id,repo_id,status,git_head,config_hash,grammar_hash,started_at,snapshot) VALUES(?1,?2,'building',?3,?4,?5,datetime('now'),?6)", params![snapshot.generation_id,snapshot.repo_id,snapshot.git_head,snapshot.config_hash,snapshot.grammar_hash,raw]).map_err(db_error)?;
        // SQLite has copied the bound snapshot; do not retain its large second representation
        // throughout every row insertion and prior-generation cleanup.
        drop(raw);
        crate::reads::write_metadata(&tx, snapshot)?;
        timings.generation_insert = start.elapsed();
        let start = Instant::now();
        // These are exclusively non-unique lookup indexes, NOT primary/foreign-key
        // constraints. Bulk rebuilding avoids random secondary B-tree maintenance
        // for every retired/inserted row. DDL is inside the same WAL transaction:
        // pinned readers keep the old schema, and rollback restores all indexes.
        for (name, _) in LOOKUP_INDEXES {
            tx.execute_batch(&format!("DROP INDEX {name}"))
                .map_err(db_error)?;
        }
        timings.secondary_indexes_drop = start.elapsed();
        // WAL readers still see the old committed pages. Within this single
        // transaction, retire old children before rebuilding indexes so we do not
        // grow every B-tree to two full generations. Keep old generation parents
        // until the pointer switch; every failure rolls all retired rows back.
        retire_children(&tx, &snapshot.generation_id, &mut timings)?;
        #[cfg(test)]
        crate::publication_tests::checkpoint("retired");
        let start = Instant::now();
        {
            // Statements must be finalized before COMMIT; no per-row SQL reparsing.
            let mut files = tx.prepare("INSERT INTO files(id,generation_id,path,language,content_hash,size_bytes,facts_json) VALUES(?1,?2,?3,?4,?5,?6,?7)").map_err(db_error)?;
            let mut nodes = tx.prepare("INSERT INTO nodes(id,generation_id,repo_id,kind,name,qualified_name,file_path,start_line,end_line,data_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)").map_err(db_error)?;
            let mut fts = tx.prepare("INSERT INTO symbol_fts(symbol_id,generation_id,name,qualified_name,file_path,text) VALUES(?1,?2,?3,?4,?5,?6)").map_err(db_error)?;
            for file in &snapshot.files {
                files
                    .execute(params![
                        kortex_core::id(&[&snapshot.repo_id, &file.path]),
                        snapshot.generation_id,
                        file.path,
                        file.language,
                        file.content_hash,
                        file.size_bytes as i64,
                        serde_json::to_string(file)?
                    ])
                    .map_err(db_error)?;
                for s in &file.symbols {
                    nodes
                        .execute(params![
                            s.symbol_id,
                            snapshot.generation_id,
                            s.repo_id,
                            s.kind,
                            s.name,
                            s.qualified_name,
                            s.file_path,
                            s.span.start_line as i64,
                            s.span.end_line as i64,
                            serde_json::to_string(s)?
                        ])
                        .map_err(db_error)?;
                    fts.execute(params![
                        s.symbol_id,
                        snapshot.generation_id,
                        s.name,
                        s.qualified_name,
                        s.file_path,
                        format!(
                            "{} {} {}",
                            s.signature,
                            s.doc_comment.as_deref().unwrap_or(""),
                            s.attributes["source_snippet"].as_str().unwrap_or("")
                        )
                    ])
                    .map_err(db_error)?;
                }
            }
        }
        timings.files_nodes_fts_insert = start.elapsed();
        let start = Instant::now();
        // Only sort borrowed pointers, never clone facts. Sorted primary-key
        // insertion avoids random dirty-page churn; snapshot ordering is untouched.
        let mut ordered_edges: Vec<_> = snapshot.edges.iter().collect();
        ordered_edges.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        timings.edges_sort = start.elapsed();
        let start = Instant::now();
        {
            let mut edges = tx.prepare("INSERT INTO edges(id,generation_id,repo_id,source_id,target_id,kind,resolution,confidence,data_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)").map_err(db_error)?;
            for e in ordered_edges {
                let raw = serde_json::to_string(e)?;
                edges
                    .execute(params![
                        e.id,
                        snapshot.generation_id,
                        e.repo_id,
                        e.source_id,
                        e.target_id,
                        e.kind,
                        e.resolution,
                        e.confidence,
                        raw
                    ])
                    .map_err(db_error)?;
            }
        }
        timings.edges_insert = start.elapsed();
        let start = Instant::now();
        crate::reads::write_projections(&tx, snapshot)?;
        timings.read_projections_insert = start.elapsed();
        #[cfg(test)]
        crate::publication_tests::checkpoint("before_indexes");
        let start = Instant::now();
        for (_, sql) in LOOKUP_INDEXES {
            tx.execute_batch(sql).map_err(db_error)?;
        }
        timings.secondary_indexes_build = start.elapsed();
        let start = Instant::now();
        tx.execute(
            "UPDATE generations SET status='committed',committed_at=datetime('now') WHERE id=?1",
            [&snapshot.generation_id],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO index_state(repo_id,active_generation_id,index_format_version) VALUES(?1,?2,1) ON CONFLICT(repo_id) DO UPDATE SET active_generation_id=excluded.active_generation_id",params![snapshot.repo_id,snapshot.generation_id]).map_err(db_error)?;
        timings.publication = start.elapsed();
        #[cfg(test)]
        crate::publication_tests::checkpoint("published");
        let start = Instant::now();
        tx.execute(
            "DELETE FROM generations WHERE id<>?1",
            [&snapshot.generation_id],
        )
        .map_err(db_error)?;
        timings.generations_delete = start.elapsed();
        let start = Instant::now();
        tx.commit().map_err(db_error)?;
        timings.durable_commit = start.elapsed();
        Ok(timings)
    }
}
