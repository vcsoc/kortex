//! ISOLATED schema-2 differential experiment. Never opened by Store or migrated from schema 1.
//! Scratch directories only; no adoption, migration, production-format or performance claim.
use crate::{WriterLock, acquire_writer, db_error, location, reads::SnapshotMetadata};
use kortex_core::{Edge, Error, ParsedFile, Result, Snapshot, Symbol};
use kortex_repo::placement::{DirectoryAnchor, require_derived_data_support};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const SCHEMA: &str = "
PRAGMA application_id=1263559986;
PRAGMA user_version=2;
CREATE TABLE identity(id INTEGER PRIMARY KEY CHECK(id=1),nonce TEXT NOT NULL);
CREATE TABLE generations(id TEXT PRIMARY KEY,status TEXT NOT NULL CHECK(status IN('building','committed')),header_json TEXT NOT NULL,layout_json TEXT NOT NULL);
CREATE TABLE state(id INTEGER PRIMARY KEY CHECK(id=1),head TEXT NOT NULL REFERENCES generations(id));
CREATE TABLE files(id TEXT PRIMARY KEY,data_json TEXT NOT NULL);
CREATE TABLE nodes(id TEXT PRIMARY KEY,file_id TEXT NOT NULL REFERENCES files(id) DEFERRABLE INITIALLY DEFERRED,data_json TEXT NOT NULL);
CREATE TABLE edges(id TEXT PRIMARY KEY,source_id TEXT NOT NULL REFERENCES nodes(id) DEFERRABLE INITIALLY DEFERRED,target_id TEXT REFERENCES nodes(id) DEFERRABLE INITIALLY DEFERRED,data_json TEXT NOT NULL);
CREATE INDEX nodes_file ON nodes(file_id);
CREATE INDEX edges_source ON edges(source_id);
CREATE INDEX edges_target ON edges(target_id);
CREATE VIRTUAL TABLE symbol_fts USING fts5(symbol_id UNINDEXED,name,qualified_name,file_path,text);
";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Layout {
    files: Vec<(String, Vec<String>)>,
    edges: Vec<String>,
}
#[derive(Debug, PartialEq, Eq)]
struct DirectoryIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}
fn directory_identity(path: &Path) -> Result<DirectoryIdentity> {
    let path = path.canonicalize()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = path.metadata()?;
        Ok(DirectoryIdentity {
            path,
            device: m.dev(),
            inode: m.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        require_derived_data_support()?;
        unreachable!()
    }
}
type Fingerprints = BTreeMap<String, String>;
#[derive(Debug, PartialEq, Eq)]
struct Facts {
    files: Fingerprints,
    nodes: Fingerprints,
    edges: Fingerprints,
    fts: Fingerprints,
    header: Option<String>,
}
/// Sealed provenance from actual pinned rows. No constructor, setters or old edge objects.
/// Not Clone: cached ParsedFiles can be moved out without copying semantic facts.
pub struct Baseline {
    directory: DirectoryIdentity,
    nonce: String,
    generation: String,
    facts: Facts,
    files: Vec<ParsedFile>,
}
impl Baseline {
    pub fn generation_id(&self) -> &str {
        &self.generation
    }
    /// Mutating returned files cannot change the sealed actual-store fingerprints.
    pub fn split_cached_files(mut self) -> (Vec<ParsedFile>, Self) {
        (std::mem::take(&mut self.files), self)
    }
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DifferentialChanges {
    pub files_written: usize,
    pub files_deleted: usize,
    pub nodes_written: usize,
    pub nodes_deleted: usize,
    pub edges_written: usize,
    pub edges_deleted: usize,
    pub fts_written: usize,
    pub fts_deleted: usize,
}
/// Diagnostic wall-clock phases only. All actual-row/header/FTS verification is retained.
#[derive(Debug, Default, Clone)]
pub struct DifferentialTimings {
    pub validate_incoming: Duration,
    pub actual_fingerprints: Duration,
    pub baseline_binding: Duration,
    pub retire: Duration,
    pub files_compare_write: Duration,
    pub nodes_fts_compare_write: Duration,
    pub edges_compare_write: Duration,
    pub publication_fk_check: Duration,
    pub durable_commit: Duration,
}
pub struct DifferentialStore {
    conn: Connection,
    directory: PathBuf,
    writable: bool,
}
fn refused(message: &str) -> Error {
    Error::new("EXPERIMENT_REFUSED", message, 8)
}
fn corrupt(message: impl std::fmt::Display) -> Error {
    Error::new("CORRUPT_INDEX", message.to_string(), 6)
}
impl DifferentialStore {
    /// Exclusively creates a NEW explicit scratch directory under a private parent.
    /// Neither an existing directory nor any schema-1 database can be adopted.
    pub fn create_scratch(directory: &Path) -> Result<Self> {
        require_derived_data_support()?;
        let parent = directory
            .parent()
            .ok_or_else(|| refused("scratch directory needs a private parent"))?;
        let parent = parent.canonicalize()?;
        let _anchor = DirectoryAnchor::acquire(&parent, false)?;
        let name = directory
            .file_name()
            .ok_or_else(|| refused("scratch directory needs a name"))?;
        let directory = parent.join(name);
        #[cfg(unix)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        let anchor = DirectoryAnchor::acquire(&directory, false)?;
        let _lock = acquire_writer(&directory)?;
        drop(anchor.create_new("differential.db")?);
        let conn = Connection::open_with_flags(
            directory.join("differential.db"),
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        configure(&conn, true)?;
        let nonce = kortex_core::id(&[
            &directory.to_string_lossy(),
            &std::process::id().to_string(),
            &format!("{:?}", SystemTime::now()),
        ]);
        conn.execute_batch("BEGIN IMMEDIATE").map_err(db_error)?;
        conn.execute_batch(SCHEMA).map_err(db_error)?;
        conn.execute("INSERT INTO identity VALUES(1,?1)", [&nonce])
            .map_err(db_error)?;
        conn.execute_batch("COMMIT").map_err(db_error)?;
        Ok(Self {
            conn,
            directory,
            writable: true,
        })
    }
    pub fn open_scratch(directory: &Path, writable: bool) -> Result<Self> {
        require_derived_data_support()?;
        let directory = directory.canonicalize()?;
        let anchor = DirectoryAnchor::acquire(&directory, false)?;
        drop(anchor.open_existing("differential.db", writable)?);
        for name in [
            "differential.db-wal",
            "differential.db-shm",
            "differential.db-journal",
            "writer.lock",
        ] {
            location::check_file(&directory.join(name))?;
        }
        let _initialization_lock = if writable {
            Some(acquire_writer(&directory)?)
        } else {
            None
        };
        let flags = if writable {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        } | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let conn = Connection::open_with_flags(directory.join("differential.db"), flags)
            .map_err(db_error)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        let app: i64 = conn
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(db_error)?;
        if version != 2 || app != 1263559986 {
            return Err(refused("not an isolated differential scratch database"));
        }
        configure(&conn, writable)?;
        if !writable {
            conn.execute_batch("BEGIN DEFERRED").map_err(db_error)?;
        }
        Ok(Self {
            conn,
            directory,
            writable,
        })
    }
    pub fn writer_lock(&self) -> Result<WriterLock> {
        acquire_writer(&self.directory)
    }
    /// Read only. Fingerprints are computed from actual JSON rows, not supplied snapshots.
    pub fn baseline(&self) -> Result<Baseline> {
        if self.writable {
            return Err(refused(
                "baseline requires a pinned read-only scratch handle",
            ));
        }
        let (header, layout) = read_header(&self.conn)?;
        let facts = fingerprints(&self.conn)?;
        let files = load_files(&self.conn, &layout)?;
        let nonce = self
            .conn
            .query_row("SELECT nonce FROM identity WHERE id=1", [], |r| r.get(0))
            .map_err(db_error)?;
        Ok(Baseline {
            directory: directory_identity(&self.directory)?,
            nonce,
            generation: header.generation_id,
            facts,
            files,
        })
    }
    pub fn load(&self) -> Result<Snapshot> {
        if self.writable {
            return Err(refused("load requires a pinned read-only scratch handle"));
        }
        let (h, layout) = read_header(&self.conn)?;
        let files = load_files(&self.conn, &layout)?;
        let mut rows = json_rows::<Edge>(&self.conn, "edges")?;
        let mut edges = Vec::with_capacity(layout.edges.len());
        for id in &layout.edges {
            edges.push(
                rows.remove(id)
                    .ok_or_else(|| corrupt("missing layout edge"))?,
            );
        }
        if !rows.is_empty()
            || h.file_count != files.len()
            || h.symbol_count != files.iter().map(|f| f.symbols.len()).sum::<usize>()
            || h.edge_count != edges.len()
        {
            return Err(corrupt("layout/count mismatch"));
        }
        Ok(Snapshot {
            repo_id: h.repo_id,
            root: h.root,
            generation_id: h.generation_id,
            git_head: h.git_head,
            config_hash: h.config_hash,
            grammar_hash: h.grammar_hash,
            resolution_hash: h.resolution_hash,
            diagnostics: h.diagnostics,
            files,
            edges,
        })
    }
    /// FTS lives on the same MVCC connection as facts/header; no generation retagging.
    pub fn keyword(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        if self.writable {
            return Err(refused(
                "keyword requires a pinned read-only scratch handle",
            ));
        }
        let limit = crate::reads::bounded_limit(limit)?;
        let tokens: Vec<_> = query
            .split_whitespace()
            .take(32)
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect();
        if tokens.is_empty() {
            return Ok(vec![]);
        }
        let mut stmt = self.conn.prepare("SELECT symbol_id,bm25(symbol_fts) FROM symbol_fts WHERE symbol_fts MATCH ?1 ORDER BY bm25(symbol_fts),symbol_id LIMIT ?2").map_err(db_error)?;
        stmt.query_map(params![tokens.join(" OR "), limit], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    /// Diagnostic digest over every actual FTS column, on this pinned connection.
    pub fn keyword_fingerprint(&self) -> Result<String> {
        if self.writable {
            return Err(refused(
                "fingerprint requires a pinned read-only scratch handle",
            ));
        }
        crate::reads::keyword_fingerprint(&self.conn, None)
    }
    /// Publishes COMPLETE recomputed facts; hashes/revisions never suppress semantic changes.
    /// None is allowed only for the empty scratch store. A supplied baseline is sealed.
    pub fn publish(
        &mut self,
        snapshot: &Snapshot,
        baseline: Option<&Baseline>,
        lock: &WriterLock,
    ) -> Result<DifferentialChanges> {
        self.publish_profiled(snapshot, baseline, lock)
            .map(|(changes, _)| changes)
    }
    /// Same complete, tamper-checked publication with phase timings; no shortcut by hash.
    pub fn publish_profiled(
        &mut self,
        snapshot: &Snapshot,
        baseline: Option<&Baseline>,
        lock: &WriterLock,
    ) -> Result<(DifferentialChanges, DifferentialTimings)> {
        let mut timings = DifferentialTimings::default();
        let start = Instant::now();
        if !self.writable || lock.directory != self.directory {
            return Err(refused(
                "writable scratch handle and matching writer lock required",
            ));
        }
        DirectoryAnchor::acquire(&self.directory, false)?;
        for name in [
            "differential.db",
            "differential.db-wal",
            "differential.db-shm",
            "differential.db-journal",
            "writer.lock",
        ] {
            location::check_file(&self.directory.join(name))?;
        }
        let layout = validate(snapshot)?;
        timings.validate_incoming = start.elapsed();
        let start = Instant::now();
        let identity = directory_identity(&self.directory)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let head: Option<String> = tx
            .query_row("SELECT head FROM state WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(db_error)?;
        let nonce: String = tx
            .query_row("SELECT nonce FROM identity WHERE id=1", [], |r| r.get(0))
            .map_err(db_error)?;
        timings.baseline_binding = start.elapsed();
        let start = Instant::now();
        let actual = fingerprints(&tx)?;
        timings.actual_fingerprints = start.elapsed();
        let start = Instant::now();
        match (head.as_deref(), baseline) {
            (None, None)
                if actual.files.is_empty()
                    && actual.nodes.is_empty()
                    && actual.edges.is_empty()
                    && actual.fts.is_empty() => {}
            (Some(head), Some(b))
                if head == b.generation
                    && b.directory == identity
                    && b.nonce == nonce
                    && b.facts == actual =>
            {
                let (old, _) = read_header(&tx)?;
                if old.repo_id != snapshot.repo_id || old.root != snapshot.root {
                    return Err(refused("repository identity cannot change"));
                }
            }
            _ => {
                return Err(Error::new(
                    "BASELINE_REFUSED",
                    "missing, stale, foreign or changed actual-store baseline",
                    8,
                ));
            }
        }
        tx.execute(
            "INSERT INTO generations VALUES(?1,'building',?2,?3)",
            params![
                snapshot.generation_id,
                serde_json::to_string(&SnapshotMetadata::from_snapshot(snapshot))?,
                serde_json::to_string(&layout)?
            ],
        )
        .map_err(db_error)?;
        timings.baseline_binding += start.elapsed();
        let start = Instant::now();
        let mut changes = DifferentialChanges::default();
        let file_ids: BTreeSet<_> = layout.files.iter().map(|f| f.0.as_str()).collect();
        let node_ids: BTreeSet<_> = snapshot.symbols().map(|s| s.symbol_id.as_str()).collect();
        let edge_ids: BTreeSet<_> = snapshot.edges.iter().map(|e| e.id.as_str()).collect();
        for id in actual
            .edges
            .keys()
            .filter(|id| !edge_ids.contains(id.as_str()))
        {
            tx.execute("DELETE FROM edges WHERE id=?1", [id])
                .map_err(db_error)?;
            changes.edges_deleted += 1;
        }
        for id in actual
            .nodes
            .keys()
            .filter(|id| !node_ids.contains(id.as_str()))
        {
            tx.execute("DELETE FROM symbol_fts WHERE symbol_id=?1", [id])
                .map_err(db_error)?;
            tx.execute("DELETE FROM nodes WHERE id=?1", [id])
                .map_err(db_error)?;
            changes.nodes_deleted += 1;
            changes.fts_deleted += 1;
        }
        for id in actual
            .files
            .keys()
            .filter(|id| !file_ids.contains(id.as_str()))
        {
            tx.execute("DELETE FROM files WHERE id=?1", [id])
                .map_err(db_error)?;
            changes.files_deleted += 1;
        }
        checkpoint("retired");
        timings.retire = start.elapsed();
        let start = Instant::now();
        for (file, (id, _)) in snapshot.files.iter().zip(&layout.files) {
            let raw = file_facts(file)?;
            if actual.files.get(id) != Some(&row_fingerprint(&raw, None, None)?) {
                tx.execute("INSERT INTO files VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data_json=excluded.data_json", params![id, raw]).map_err(db_error)?;
                changes.files_written += 1;
            }
        }
        checkpoint("files");
        timings.files_compare_write = start.elapsed();
        let start = Instant::now();
        for (file, (file_id, _)) in snapshot.files.iter().zip(&layout.files) {
            for s in &file.symbols {
                let raw = serde_json::to_string(s)?;
                if actual.nodes.get(&s.symbol_id)
                    != Some(&row_fingerprint(&raw, Some(file_id), None)?)
                {
                    tx.execute("INSERT INTO nodes VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET file_id=excluded.file_id,data_json=excluded.data_json", params![s.symbol_id, file_id, raw]).map_err(db_error)?;
                    // The complete actual FTS scan above proves absence, including orphan
                    // rows. Avoid an unindexed O(nodes) delete for each new symbol only
                    // when that checked corpus contains no such identity.
                    let removed = if actual.fts.contains_key(&s.symbol_id) {
                        tx.execute("DELETE FROM symbol_fts WHERE symbol_id=?1", [&s.symbol_id])
                            .map_err(db_error)?
                    } else {
                        0
                    };
                    tx.execute("INSERT INTO symbol_fts(symbol_id,name,qualified_name,file_path,text) VALUES(?1,?2,?3,?4,?5)", params![s.symbol_id,s.name,s.qualified_name,s.file_path,format!("{} {} {}",s.signature,s.doc_comment.as_deref().unwrap_or(""),s.attributes["source_snippet"].as_str().unwrap_or(""))]).map_err(db_error)?;
                    changes.nodes_written += 1;
                    changes.fts_written += 1;
                    changes.fts_deleted += removed;
                }
            }
        }
        checkpoint("nodes");
        timings.nodes_fts_compare_write = start.elapsed();
        let start = Instant::now();
        for e in &snapshot.edges {
            let raw = serde_json::to_string(e)?;
            if actual.edges.get(&e.id)
                != Some(&row_fingerprint(
                    &raw,
                    Some(&e.source_id),
                    e.target_id.as_deref(),
                )?)
            {
                tx.execute("INSERT INTO edges VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET source_id=excluded.source_id,target_id=excluded.target_id,data_json=excluded.data_json", params![e.id,e.source_id,e.target_id,raw]).map_err(db_error)?;
                changes.edges_written += 1;
            }
        }
        checkpoint("edges");
        timings.edges_compare_write = start.elapsed();
        let start = Instant::now();
        tx.execute(
            "UPDATE generations SET status='committed' WHERE id=?1",
            [&snapshot.generation_id],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET head=excluded.head",
            [&snapshot.generation_id],
        )
        .map_err(db_error)?;
        tx.execute(
            "DELETE FROM generations WHERE id<>?1",
            [&snapshot.generation_id],
        )
        .map_err(db_error)?;
        checkpoint("published");
        let broken = tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(db_error)?
            .exists([])
            .map_err(db_error)?;
        if broken {
            return Err(corrupt("differential foreign key failure"));
        }
        timings.publication_fk_check = start.elapsed();
        let start = Instant::now();
        tx.commit().map_err(db_error)?;
        timings.durable_commit = start.elapsed();
        Ok((changes, timings))
    }
}
fn configure(conn: &Connection, writable: bool) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(db_error)?;
    conn.execute_batch("PRAGMA foreign_keys=ON")
        .map_err(db_error)?;
    if writable {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL")
            .map_err(db_error)?;
    }
    Ok(())
}
fn read_header(conn: &Connection) -> Result<(SnapshotMetadata, Layout)> {
    let row: Option<(String,String)> = conn.query_row("SELECT g.header_json,g.layout_json FROM state s JOIN generations g ON g.id=s.head WHERE g.status='committed'", [], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
    let (h, l) = row.ok_or_else(|| Error::new("INDEX_NOT_FOUND", "no scratch generation", 3))?;
    Ok((
        serde_json::from_str(&h).map_err(corrupt)?,
        serde_json::from_str(&l).map_err(corrupt)?,
    ))
}
fn row_fingerprint(raw: &str, owner: Option<&str>, target: Option<&str>) -> Result<String> {
    Ok(kortex_core::hash(&serde_json::to_vec(&(
        raw, owner, target,
    ))?))
}
fn fingerprints(conn: &Connection) -> Result<Facts> {
    fn rows(conn: &Connection, sql: &str) -> Result<Fingerprints> {
        let mut stmt = conn.prepare(sql).map_err(db_error)?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(db_error)?
        .map(|row| {
            let (id, raw, owner, target) = row.map_err(db_error)?;
            Ok((
                id,
                row_fingerprint(&raw, owner.as_deref(), target.as_deref())?,
            ))
        })
        .collect()
    }
    let header: Option<(String, String)> = conn
        .query_row(
            "SELECT g.header_json,g.layout_json FROM state s JOIN generations g ON g.id=s.head",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_error)?;
    let mut fts = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT symbol_id,name,qualified_name,file_path,text FROM symbol_fts")
        .map_err(db_error)?;
    let fts_rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                [
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ],
            ))
        })
        .map_err(db_error)?;
    for row in fts_rows {
        let (id, values) = row.map_err(db_error)?;
        if fts
            .insert(id, kortex_core::hash(&serde_json::to_vec(&values)?))
            .is_some()
        {
            return Err(corrupt("duplicate FTS identity"));
        }
    }
    Ok(Facts {
        files: rows(conn, "SELECT id,data_json,NULL,NULL FROM files")?,
        nodes: rows(conn, "SELECT id,data_json,file_id,NULL FROM nodes")?,
        edges: rows(conn, "SELECT id,data_json,source_id,target_id FROM edges")?,
        fts,
        header: header
            .map(|h| serde_json::to_vec(&h).map(|v| kortex_core::hash(&v)))
            .transpose()?,
    })
}
fn json_rows<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    table: &str,
) -> Result<BTreeMap<String, T>> {
    let mut stmt = conn
        .prepare(&format!("SELECT id,data_json FROM {table}"))
        .map_err(db_error)?;
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(db_error)?
        .map(|row| {
            let (id, raw) = row.map_err(db_error)?;
            Ok((id, serde_json::from_str(&raw).map_err(corrupt)?))
        })
        .collect()
}
fn load_files(conn: &Connection, layout: &Layout) -> Result<Vec<ParsedFile>> {
    let mut files = json_rows::<ParsedFile>(conn, "files")?;
    let mut nodes = json_rows::<Symbol>(conn, "nodes")?;
    let mut ordered = Vec::with_capacity(layout.files.len());
    for (id, symbols) in &layout.files {
        let mut file = files
            .remove(id)
            .ok_or_else(|| corrupt("missing layout file"))?;
        if !file.symbols.is_empty() {
            return Err(corrupt("file row duplicates normalized symbols"));
        }
        for id in symbols {
            file.symbols.push(
                nodes
                    .remove(id)
                    .ok_or_else(|| corrupt("missing layout node"))?,
            );
        }
        ordered.push(file);
    }
    if !files.is_empty() || !nodes.is_empty() {
        return Err(corrupt("extra rows outside layout"));
    }
    Ok(ordered)
}
fn validate(s: &Snapshot) -> Result<Layout> {
    let mut files = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for file in &s.files {
        if !files.insert(&file.path) {
            return Err(corrupt("duplicate file path"));
        }
        for node in &file.symbols {
            if !nodes.insert(node.symbol_id.as_str())
                || node.repo_id != s.repo_id
                || node.file_path != file.path
            {
                return Err(corrupt("duplicate or foreign node"));
            }
        }
    }
    for file in &s.files {
        for node in &file.symbols {
            if node
                .owner_id
                .as_ref()
                .is_some_and(|id| !nodes.contains(id.as_str()))
            {
                return Err(corrupt("dangling node owner"));
            }
        }
        for reference in &file.references {
            if !nodes.contains(reference.source_id.as_str()) {
                return Err(corrupt("dangling reference source"));
            }
        }
    }
    for edge in &s.edges {
        if !edges.insert(&edge.id)
            || edge.repo_id != s.repo_id
            || !nodes.contains(edge.source_id.as_str())
            || edge
                .target_id
                .as_ref()
                .is_some_and(|id| !nodes.contains(id.as_str()))
            || edge
                .candidates
                .iter()
                .any(|id| !nodes.contains(id.as_str()))
            || !edge.confidence.is_finite()
            || !(0.0..=1.0).contains(&edge.confidence)
        {
            return Err(corrupt("duplicate, foreign, dangling or invalid edge"));
        }
    }
    Ok(Layout {
        files: s
            .files
            .iter()
            .map(|f| {
                (
                    kortex_core::id(&[&s.repo_id, &f.path]),
                    f.symbols.iter().map(|s| s.symbol_id.clone()).collect(),
                )
            })
            .collect(),
        edges: s.edges.iter().map(|e| e.id.clone()).collect(),
    })
}
/// Serialize parse facts by reference, without cloning symbols only to discard them.
fn file_facts(f: &ParsedFile) -> Result<String> {
    #[derive(Serialize)]
    struct Facts<'a> {
        path: &'a str,
        language: &'a str,
        content_hash: &'a str,
        size_bytes: usize,
        symbols: &'a [Symbol],
        references: &'a [kortex_core::Reference],
        imports: &'a [kortex_core::Import],
        import_scopes: &'a [String],
        import_spans: &'a [kortex_core::Span],
        diagnostics: &'a [String],
    }
    serde_json::to_string(&Facts {
        path: &f.path,
        language: &f.language,
        content_hash: &f.content_hash,
        size_bytes: f.size_bytes,
        symbols: &[],
        references: &f.references,
        imports: &f.imports,
        import_scopes: &f.import_scopes,
        import_spans: &f.import_spans,
        diagnostics: &f.diagnostics,
    })
    .map_err(Into::into)
}
fn checkpoint(phase: &str) {
    #[cfg(test)]
    tests::checkpoint(phase);
    #[cfg(not(test))]
    let _ = phase;
}
#[cfg(test)]
#[path = "experimental_tests.rs"]
mod tests;
