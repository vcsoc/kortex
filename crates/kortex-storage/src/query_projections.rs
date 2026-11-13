//! Query-only evidence: deliberately no edge-table access or shared full-graph approval.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryManifest {
    version: u32,
    files: usize,
    symbols: usize,
}
fn table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(corrupt)
}
fn has_rows(conn: &rusqlite::Connection, table: &str, generation: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE generation_id=?1)"),
        [generation],
        |r| r.get(0),
    )
    .map_err(corrupt)
}
fn manifest_raw(
    conn: &rusqlite::Connection,
    table: &str,
    generation: &str,
) -> Result<Option<String>> {
    if !table_exists(conn, table)? {
        return Ok(None);
    }
    let mut stmt = conn
        .prepare(&format!(
            "SELECT manifest_json FROM {table} WHERE generation_id=?1"
        ))
        .map_err(corrupt)?;
    let mut rows = stmt.query([generation]).map_err(corrupt)?;
    let raw = rows
        .next()
        .map_err(corrupt)?
        .map(|r| r.get(0))
        .transpose()
        .map_err(corrupt)?;
    if rows.next().map_err(corrupt)?.is_some() {
        return Err(corrupt("duplicate query evidence manifest"));
    }
    Ok(raw)
}
fn identities(
    conn: &rusqlite::Connection,
    generation: &str,
    table: &str,
    facts: &str,
) -> Result<()> {
    // Facts' primary-key index covers this join: no large data_json table fetch.
    let broken: bool = conn.query_row(&format!(
        "SELECT EXISTS(SELECT 1 FROM {table} p LEFT JOIN {facts} f ON f.id=p.id AND f.generation_id=p.generation_id WHERE p.generation_id=?1 AND (typeof(p.id)<>'text' OR f.id IS NULL)) OR EXISTS(SELECT 1 FROM {table} WHERE generation_id=?1 GROUP BY id HAVING count(*)<>1)"
    ), [generation], |r| r.get(0)).map_err(corrupt)?;
    if broken {
        return Err(corrupt("query evidence identity mismatch"));
    }
    Ok(())
}
impl ReadSession<'_> {
    /// Query evidence ONLY. Success is not a statement about graph/edge health.
    pub fn query_scope_source(&self) -> Result<MetadataSource> {
        self.query_projection().map(|(source, _)| source)
    }
    pub(super) fn query_projection(&self) -> Result<(MetadataSource, bool)> {
        if let Some(source) = self.store.query_projection.get() {
            return Ok(*source);
        }
        let conn = &self.store.conn;
        let generation = &self.metadata.generation_id;
        let fast_raw = manifest_raw(conn, "generation_query_manifest", generation)?;
        let fast = if let Some(raw) = fast_raw {
            let m: QueryManifest = serde_json::from_str(&raw).map_err(corrupt)?;
            if m.version != 1
                || m.files != self.metadata.file_count
                || m.symbols != self.metadata.symbol_count
            {
                return Err(corrupt("query manifest mismatch"));
            }
            validate_order(conn, generation, "query_symbol_inventory", m.symbols)?;
            identities(conn, generation, "query_symbol_inventory", "nodes")?;
            true
        } else {
            if has_rows(conn, "query_symbol_inventory", generation)? {
                return Err(corrupt("query rows lack their manifest"));
            }
            false
        };
        let raw = manifest_raw(conn, "generation_read_manifest", generation)?;
        let source = if let Some(raw) = raw {
            let m: Manifest = serde_json::from_str(&raw).map_err(corrupt)?;
            if ![1, 2].contains(&m.version)
                || m.files != self.metadata.file_count
                || m.symbols != self.metadata.symbol_count
                || m.edges != self.metadata.edge_count
            {
                return Err(corrupt("read projection manifest mismatch"));
            }
            for (table, expected) in [("files", m.files), ("nodes", m.symbols)] {
                let count: i64 = conn
                    .query_row(
                        &format!("SELECT count(*) FROM {table} WHERE generation_id=?1"),
                        [generation],
                        |r| r.get(0),
                    )
                    .map_err(corrupt)?;
                if count != integer(expected)? {
                    return Err(corrupt("query fact count mismatch"));
                }
            }
            if !fast {
                validate_order(conn, generation, "read_symbol_inventory", m.symbols)?;
                identities(conn, generation, "read_symbol_inventory", "nodes")?;
                let invalid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM read_symbol_inventory WHERE generation_id=?1 AND (typeof(start_byte)<>'integer' OR start_byte<0 OR typeof(language)<>'text'))", [generation], |r| r.get(0)).map_err(corrupt)?;
                if invalid {
                    return Err(corrupt("query symbol type mismatch"));
                }
            }
            let (total, valid): (i64, i64) = conn.query_row("SELECT count(*),coalesce(sum(typeof(has_diagnostics)='integer' AND has_diagnostics IN(0,1)),0) FROM read_file_diagnostics WHERE generation_id=?1", [generation], |r| Ok((r.get(0)?,r.get(1)?))).map_err(corrupt)?;
            if total != integer(m.files)? || total != valid {
                return Err(corrupt("incomplete parser diagnostic evidence"));
            }
            identities(conn, generation, "read_file_diagnostics", "files")?;
            if m.version == 2 {
                validate_order(conn, generation, "read_file_order", m.files)?;
                identities(conn, generation, "read_file_order", "files")?;
                MetadataSource::Projection
            } else {
                MetadataSource::LegacySnapshot
            }
        } else {
            // Only absence of ALL used evidence permits the honest legacy load.
            // Edge metadata is intentionally not used or certified by query scope.
            for table in [
                "read_symbol_inventory",
                "read_file_order",
                "read_file_diagnostics",
                "query_symbol_inventory",
            ] {
                if has_rows(conn, table, generation)? {
                    return Err(corrupt("query evidence lacks read manifest"));
                }
            }
            if fast {
                return Err(corrupt("query manifest lacks file evidence manifest"));
            }
            MetadataSource::LegacySnapshot
        };
        let result = (source, fast);
        let _ = self.store.query_projection.set(result);
        Ok(result)
    }
    /// Materialize a selected fact and compare ALL raw projected fields, not folded keys.
    pub fn query_symbol(&self, entry: &SymbolInventoryEntry) -> Result<Symbol> {
        self.query_scope_source()?;
        let symbol = self
            .get_symbol(&entry.symbol_id)?
            .ok_or_else(|| corrupt("selected query symbol missing"))?;
        if SymbolInventoryEntry::from_symbol(&symbol) != *entry {
            return Err(corrupt(
                "selected query inventory differs from pinned symbol fact",
            ));
        }
        Ok(symbol)
    }
    /// Original file-vector order, with query-only validation (never full edge validation).
    pub fn query_file_inventory_snapshot_order(&self) -> Result<Vec<FileInventoryEntry>> {
        self.file_inventory_for_source(self.query_scope_source()?)
    }
    /// Explicit parser partial evidence for every file; no edge health inference.
    pub fn query_parser_diagnostic_presence(&self) -> Result<Vec<FileDiagnosticPresence>> {
        self.diagnostics_for_source(self.query_scope_source()?)
    }
    /// Exact lexical/FTS500 union; selected facts still require full raw-field comparison.
    pub fn query_scoped_candidate_inventory(
        &self,
        text: &str,
        filter: &QueryCandidateFilter<'_>,
    ) -> Result<QueryCandidateInventory> {
        self.query_candidate_inventory_inner(text, filter, None, true)
    }
    pub fn query_scoped_candidate_inventory_with_control(
        &self,
        text: &str,
        filter: &QueryCandidateFilter<'_>,
        control: &RequestControl,
    ) -> Result<QueryCandidateInventory> {
        self.query_candidate_inventory_inner(text, filter, Some(control), true)
    }
}
pub(super) fn write_query_projections(tx: &rusqlite::Transaction<'_>, s: &Snapshot) -> Result<()> {
    // Optional side projection only. Old readers retain their unchanged v2 manifest.
    // Raw UTF-8 fields only: no persistent Unicode-version-dependent folding keys.
    tx.execute_batch("CREATE TABLE IF NOT EXISTS generation_query_manifest(generation_id TEXT PRIMARY KEY REFERENCES generations(id) ON DELETE CASCADE,manifest_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS query_symbol_inventory(generation_id TEXT NOT NULL REFERENCES generations(id) ON DELETE CASCADE,ordinal INTEGER NOT NULL CHECK(ordinal>=0),id TEXT NOT NULL,name TEXT NOT NULL,qualified_name TEXT NOT NULL,file_path TEXT NOT NULL,language TEXT NOT NULL,kind TEXT NOT NULL,start_byte INTEGER NOT NULL CHECK(start_byte>=0),PRIMARY KEY(generation_id,ordinal),UNIQUE(generation_id,id),FOREIGN KEY(id,generation_id) REFERENCES nodes(id,generation_id) ON DELETE CASCADE) WITHOUT ROWID;").map_err(db_error)?;
    let mut rows = tx
        .prepare("INSERT INTO query_symbol_inventory VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
        .map_err(db_error)?;
    for (ordinal, node) in s.symbols().enumerate() {
        rows.execute(params![
            s.generation_id,
            integer(ordinal)?,
            node.symbol_id,
            node.name,
            node.qualified_name,
            node.file_path,
            node.language,
            node.kind,
            integer(node.span.start_byte)?
        ])
        .map_err(db_error)?;
    }
    tx.execute(
        "INSERT INTO generation_query_manifest VALUES(?1,?2)",
        params![
            s.generation_id,
            serde_json::to_string(&QueryManifest {
                version: 1,
                files: s.files.len(),
                symbols: s.symbols().count()
            })?
        ],
    )
    .map_err(db_error)?;
    Ok(())
}
