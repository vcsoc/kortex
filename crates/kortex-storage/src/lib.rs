//! SQLite generation store. Commit publishes all graph/FTS records atomically.
/// Explicit scratch-only API. Not used by default Store, CLI or indexer.
pub mod experimental;
pub mod location;
mod maintenance;
mod publication;
mod reads;
pub use reads::{
    FileDiagnosticPresence, FileInventoryEntry, MetadataSource, OrderedEdge, OrderedEdgePage,
    QueryCandidateFilter, QueryCandidateInventory, ReadPage, ReadSession, SnapshotMetadata,
    SymbolFilter, SymbolInventoryEntry,
};
#[cfg(test)]
#[path = "placement_tests.rs"]
mod placement_integration_tests;
#[cfg(test)]
mod query_scope_tests;
#[cfg(test)]
mod read_cache_tests;
#[cfg(test)]
mod read_candidate_tests;
#[cfg(test)]
mod read_tests;
#[cfg(test)]
mod test_support;
pub use publication::CommitTimings;
#[cfg(test)]
mod publication_tests;
#[cfg(test)]
mod recovery;
pub mod registry;
pub use maintenance::RepairReport;
pub use registry::RepositorySelection;
#[cfg(test)]
mod boundary_tests;
#[cfg(all(test, unix))]
mod permission_policy_tests;
#[cfg(test)]
mod readiness_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod safety_tests;
use fs2::FileExt;
use kortex_core::{Error, Result, Snapshot, Symbol};
use rusqlite::{Connection, OptionalExtension, params};
#[cfg(test)]
use std::fs;
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};

fn db_error(e: rusqlite::Error) -> Error {
    Error::new("STORAGE_ERROR", e.to_string(), 6)
}
pub struct Store {
    conn: Connection,
    // Successful validation belongs to this immutable read-only transaction, not a request.
    projection_source: std::cell::OnceCell<MetadataSource>,
    // Query approval must never authorize full graph evidence.
    query_projection: std::cell::OnceCell<(MetadataSource, bool)>,
    pub directory: PathBuf,
}
pub struct WriterLock {
    file: File,
    directory: PathBuf,
}
impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}
impl Store {
    /// Compatible default repository-local placement.
    pub fn open(root: &Path, create: bool) -> Result<Self> {
        Self::open_with_storage(root, "repository", create)
    }
    pub fn directory_for(root: &Path, storage: &str) -> Result<PathBuf> {
        location::directory_for(root, storage)
    }
    pub fn open_with_storage(root: &Path, storage: &str, create: bool) -> Result<Self> {
        kortex_repo::placement::require_derived_data_support()?;
        let directory = Self::directory_for(root, storage)?;
        if !create
            && directory
                .symlink_metadata()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(Error::new("INDEX_NOT_FOUND", "run kortex index first", 3));
        }
        location::check_index_directory(&directory, storage, create)?;
        let db = directory.join("index.db");
        for name in [
            "index.db",
            "index.db-wal",
            "index.db-shm",
            "index.db-journal",
            "writer.lock",
        ] {
            location::check_file(&directory.join(name))?;
        }
        location::identity(&directory, root, false)?;
        if !create && !db.is_file() {
            return Err(Error::new("INDEX_NOT_FOUND", "run kortex index first", 3));
        }
        let _initialization_lock = if create {
            Some(acquire_writer(&directory)?)
        } else {
            None
        };
        let conn = if create {
            // Create privately and without following symlinks before SQLite sees the path.
            drop(location::open_private(&db, false)?);
            Connection::open_with_flags(
                &db,
                rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
        } else {
            Connection::open_with_flags(
                &db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
        }
        .map_err(db_error)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(db_error)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version > 1 {
            return Err(Error::new(
                "INDEX_VERSION",
                "index created by newer Kortex; refusing migration",
                6,
            ));
        }
        if create {
            // Refuse unsupported schemas before any database-writing PRAGMA.
            maintenance::migrate(&conn, &directory, version)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
                .map_err(db_error)?;
            location::identity(&directory, root, true)?;
        } else if version != 1 {
            return Err(Error::new(
                "INDEX_VERSION",
                "unsupported index schema; rebuild the index",
                6,
            ));
        } else {
            conn.execute_batch("BEGIN DEFERRED").map_err(db_error)?;
        }
        Ok(Self {
            conn,
            projection_source: std::cell::OnceCell::new(),
            query_projection: std::cell::OnceCell::new(),
            directory,
        })
    }
    pub fn writer_lock(&self) -> Result<WriterLock> {
        acquire_writer(&self.directory)
    }
    pub fn load(&self) -> Result<Snapshot> {
        let raw:Option<String>=self.conn.query_row("SELECT g.snapshot FROM index_state s JOIN generations g ON g.id=s.active_generation_id WHERE g.status='committed'",[],|r|r.get(0)).optional().map_err(db_error)?;
        match raw {
            Some(s) => {
                serde_json::from_str(&s).map_err(|e| Error::new("CORRUPT_INDEX", e.to_string(), 6))
            }
            None => Err(Error::new("INDEX_NOT_FOUND", "no committed generation", 3)),
        }
    }
    pub fn commit(&mut self, snapshot: &Snapshot, lock: &WriterLock) -> Result<()> {
        self.commit_profiled(snapshot, lock).map(|_| ())
    }
    pub fn keyword(
        &self,
        generation: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64)>> {
        let tokens: Vec<_> = query
            .split_whitespace()
            .take(32)
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect();
        if tokens.is_empty() {
            return Ok(vec![]);
        }
        let mut stmt=self.conn.prepare("SELECT symbol_id,bm25(symbol_fts) FROM symbol_fts WHERE symbol_fts MATCH ?1 AND generation_id=?2 ORDER BY bm25(symbol_fts),symbol_id LIMIT ?3").map_err(db_error)?;
        stmt.query_map(
            params![tokens.join(" OR "), generation, limit as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    pub fn integrity(&self) -> Result<String> {
        let physical: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if physical != "ok" {
            return Ok(physical);
        }
        let broken_foreign_key = self
            .conn
            .prepare("PRAGMA foreign_key_check")
            .map_err(db_error)?
            .exists([])
            .map_err(db_error)?;
        if broken_foreign_key {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "foreign key integrity failure",
                6,
            ));
        }
        let states: i64 = self
            .conn
            .query_row("SELECT count(*) FROM index_state", [], |r| r.get(0))
            .map_err(db_error)?;
        if states == 0 {
            return Ok(physical);
        }
        if states != 1 {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "index contains multiple repository states",
                6,
            ));
        }
        let snapshot = self.load()?;
        for (table, expected) in [
            ("files", snapshot.files.len()),
            ("nodes", snapshot.symbols().count()),
            ("edges", snapshot.edges.len()),
            ("symbol_fts", snapshot.symbols().count()),
        ] {
            let actual: i64 = self
                .conn
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE generation_id=?1"),
                    [&snapshot.generation_id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if actual != expected as i64 {
                return Err(Error::new(
                    "CORRUPT_INDEX",
                    format!("active generation {table} count differs from committed snapshot"),
                    6,
                ));
            }
        }
        // FTS identity fields are UNINDEXED. A correlated NOT EXISTS scanned the
        // entire FTS table for each node (quadratic at reference scale). Materialize
        // the identity set once; NULLs cannot mask a missing identity with NOT IN.
        let missing: i64 = self.conn.query_row("SELECT count(*) FROM nodes n WHERE n.generation_id=?1 AND (n.id IS NULL OR n.id NOT IN(SELECT symbol_id FROM symbol_fts WHERE generation_id=?1 AND symbol_id IS NOT NULL))", [&snapshot.generation_id], |r| r.get(0)).map_err(db_error)?;
        if missing != 0 {
            return Err(Error::new(
                "CORRUPT_INDEX",
                "active generation FTS identities differ from graph",
                6,
            ));
        }
        Ok(physical)
    }
    pub fn symbol_rows(&self) -> Result<Vec<Symbol>> {
        let snap = self.load()?;
        Ok(snap.symbols().cloned().collect())
    }
}
fn acquire_writer(directory: &Path) -> Result<WriterLock> {
    location::check_directory(directory, false)?;
    let path = directory.join("writer.lock");
    let file = location::open_private(&path, false)?;
    file.try_lock_exclusive()
        .map_err(|_| Error::new("WRITER_BUSY", "another index writer holds the lock", 7))?;
    Ok(WriterLock {
        file,
        directory: directory.to_owned(),
    })
}
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS repositories(id TEXT PRIMARY KEY,root TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS generations(id TEXT PRIMARY KEY,repo_id TEXT NOT NULL REFERENCES repositories(id),status TEXT NOT NULL CHECK(status IN ('building','committed','failed')),git_head TEXT,config_hash TEXT NOT NULL,grammar_hash TEXT NOT NULL,started_at TEXT NOT NULL,committed_at TEXT,snapshot TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files(id TEXT,generation_id TEXT REFERENCES generations(id) ON DELETE CASCADE,path TEXT NOT NULL,language TEXT,content_hash TEXT NOT NULL,size_bytes INTEGER NOT NULL,facts_json TEXT NOT NULL,PRIMARY KEY(id,generation_id));
CREATE TABLE IF NOT EXISTS nodes(id TEXT,generation_id TEXT REFERENCES generations(id) ON DELETE CASCADE,repo_id TEXT NOT NULL REFERENCES repositories(id),kind TEXT NOT NULL,name TEXT NOT NULL,qualified_name TEXT,file_path TEXT,start_line INTEGER,end_line INTEGER,data_json TEXT NOT NULL,PRIMARY KEY(id,generation_id));
CREATE TABLE IF NOT EXISTS edges(id TEXT,generation_id TEXT REFERENCES generations(id) ON DELETE CASCADE,repo_id TEXT NOT NULL REFERENCES repositories(id),source_id TEXT NOT NULL,target_id TEXT,kind TEXT NOT NULL,resolution TEXT NOT NULL,confidence REAL NOT NULL CHECK(confidence>=0 AND confidence<=1),data_json TEXT NOT NULL,PRIMARY KEY(id,generation_id),FOREIGN KEY(source_id,generation_id) REFERENCES nodes(id,generation_id),FOREIGN KEY(target_id,generation_id) REFERENCES nodes(id,generation_id));
CREATE TABLE IF NOT EXISTS symbol_aliases(repo_id TEXT,old_id TEXT,new_id TEXT,confidence REAL NOT NULL,generation_id TEXT);
CREATE TABLE IF NOT EXISTS index_state(repo_id TEXT PRIMARY KEY REFERENCES repositories(id),active_generation_id TEXT REFERENCES generations(id),index_format_version INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_nodes_lookup ON nodes(generation_id,kind,name);
CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(generation_id,file_path);
CREATE INDEX IF NOT EXISTS idx_nodes_qualified ON nodes(generation_id,qualified_name);
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(generation_id,source_id,kind);
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(generation_id,target_id,kind);
CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(symbol_id UNINDEXED,generation_id UNINDEXED,name,qualified_name,file_path,text);
INSERT OR IGNORE INTO schema_migrations VALUES(1,datetime('now'));
PRAGMA user_version=1;
";

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(g: &str) -> Snapshot {
        Snapshot {
            repo_id: "repo".into(),
            root: "/sample".into(),
            generation_id: g.into(),
            ..Snapshot::default()
        }
    }
    #[test]
    fn generations_lock_restart_rollback() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path(), true).unwrap();
        let l = s.writer_lock().unwrap();
        assert!(s.writer_lock().is_err());
        match Store::open(d.path(), true) {
            Ok(_) => panic!("schema initializer bypassed writer lock"),
            Err(e) => assert_eq!(e.code, "WRITER_BUSY"),
        };
        s.commit(&sample("one"), &l).unwrap();
        assert_eq!(s.load().unwrap().generation_id, "one");
        assert!(s.commit(&sample("one"), &l).is_err());
        assert_eq!(s.load().unwrap().generation_id, "one");
        s.commit(&sample("two"), &l).unwrap();
        drop(l);
        drop(s);
        let s = Store::open(d.path(), false).unwrap();
        assert_eq!(s.load().unwrap().generation_id, "two");
        assert_eq!(s.integrity().unwrap(), "ok");
    }
    #[test]
    fn corrupt_db_diagnostic() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".kortex")).unwrap();
        fs::write(d.path().join(".kortex/index.db"), "invalid database").unwrap();
        assert!(Store::open(d.path(), false).is_err());
    }
}
