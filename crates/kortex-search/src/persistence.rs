//! Explicit private SQLite vector cache, independent of graph generations.
//!
//! Stores only versioned hashes and normalized vectors, never source/model text.
//! Full-ancestor/ACL policy precedes I/O. Ambient SQLite paths are not a custom
//! descriptor-bound VFS: current UID, root and mount administrators remain trusted.
use crate::hybrid::VectorPersistence;
use kortex_core::{Error, Result};
use kortex_repo::placement::{DirectoryAnchor, require_derived_data_support};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{fs::File, path::Path, time::Duration};

const DATABASE: &str = "vectors-v1.db";
const JOURNAL: &str = "vectors-v1.db-journal";
const APPLICATION_ID: i64 = 0x4b585631;
const MAX_VECTOR_BYTES: usize = 4096 * 4;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const ROW_BYTES: usize = 216;
const MAX_DATABASE_BYTES: u64 = 128 * 1024 * 1024;
// A journal has page-number/checksum frames and sector-aligned headers, not just
// DB bytes. 4 GiB exceeds 32768 * (4096 + 8 + 65536) + 65536, allowing even one
// maximum-sector header per page during cache spills, plus a trailing sector.
const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn validate_artifact_size(file: &File, name: &str) -> Result<()> {
    let maximum = if name == DATABASE {
        MAX_DATABASE_BYTES
    } else {
        MAX_JOURNAL_BYTES
    };
    if file.metadata()?.len() > maximum {
        return Err(corrupt(if name == DATABASE {
            "cache database exceeds 128 MiB physical bound"
        } else {
            "cache rollback journal exceeds 4 GiB physical bound"
        }));
    }
    Ok(())
}

fn storage(error: rusqlite::Error) -> Error {
    Error::new("VECTOR_CACHE_STORAGE", error.to_string(), 6)
}
fn corrupt(message: &str) -> Error {
    Error::new("VECTOR_CACHE_CORRUPT", message, 6)
}
fn invalid(message: &str) -> Error {
    Error::new("INVALID_VECTOR_CACHE", message, 2)
}
fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn keys(model: &str, content: &str) -> Result<()> {
    if !valid_key(model) || !valid_key(content) {
        return Err(invalid("cache keys must be full lowercase BLAKE3 digests"));
    }
    Ok(())
}
fn vector_valid(vector: &[f32]) -> bool {
    (1..=4096).contains(&vector.len())
        && vector.iter().all(|v| v.is_finite())
        && (vector
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt()
            - 1.0)
            .abs()
            <= 1e-3
}
fn checksum(model: &str, content: &str, dimensions: usize, bytes: &[u8]) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(b"kortex-vector-row-v1\0");
    digest.update(model.as_bytes());
    digest.update(content.as_bytes());
    digest.update(&(dimensions as u64).to_le_bytes());
    digest.update(bytes);
    digest.finalize().to_hex().to_string()
}
fn open_or_create(anchor: &DirectoryAnchor, name: &str) -> Result<File> {
    match anchor.create_new(name) {
        Ok(file) => Ok(file),
        // Concurrent creators may have won; the existing-file API validates the
        // actual descriptor without following, truncating, chmodding or adopting.
        Err(_) => anchor.open_existing(name, true),
    }
}

/// Bounded FIFO cache. Limits count vector bytes + three 64-byte digests +
/// three 8-byte integers per row (216 bytes), not SQLite page overhead. The main
/// DB caps at 128 MiB; preexisting rollback journals cap at 4 GiB including frame,
/// sector and cache-spill overhead. Both are checked on actual opened descriptors
/// BEFORE SQLite can recover a hot journal or create any missing artifact. Runtime
/// placement checks recheck sizes too; no descriptor-bound SQLite VFS is claimed.
/// No WAL/SHM is used.
pub struct SqliteVectorPersistence {
    conn: Connection,
    anchor: DirectoryAnchor,
    _database: File,
    _journal: File,
}
impl SqliteVectorPersistence {
    /// Directory is explicitly trusted configuration, absolute and without aliases.
    /// Unsupported privacy platforms refuse before even creating a directory.
    /// Existing limits may shrink, never silently expand with concurrent clients.
    pub fn open(directory: &Path, capacity: usize, byte_budget: usize) -> Result<Self> {
        require_derived_data_support()?;
        if !(1..=100_000).contains(&capacity) || !(256..=MAX_BYTES).contains(&byte_budget) {
            return Err(invalid("capacity 1..100000 and bytes 256..64 MiB required"));
        }
        let anchor = DirectoryAnchor::acquire(directory, true)?;
        // Validate every reserved artifact before creating any missing file. Unknown
        // SQLite sidecars are refused, not unlinked or silently reused.
        for name in [DATABASE, JOURNAL, "vectors-v1.db-wal", "vectors-v1.db-shm"] {
            match directory.join(name).symlink_metadata() {
                Ok(_) => {
                    let file = anchor.open_existing(name, false)?;
                    if name.ends_with("-wal") || name.ends_with("-shm") {
                        return Err(corrupt("unexpected WAL/SHM in rollback-only vector cache"));
                    }
                    validate_artifact_size(&file, name)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let database = open_or_create(&anchor, DATABASE)?;
        validate_artifact_size(&database, DATABASE)?;
        let mut conn = Connection::open_with_flags(
            directory.join(DATABASE),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let app: i64 = conn
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        let pages: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(storage)?;
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(storage)?;
        if page_size != 4096 || pages > 32768 || database.metadata()?.len() > MAX_DATABASE_BYTES {
            return Err(corrupt("cache exceeds fixed physical page/byte bounds"));
        }
        if (version != 1 || app != APPLICATION_ID) && !(version == 0 && app == 0 && pages == 0) {
            return Err(corrupt(
                "unknown vector cache application/schema; no migration performed",
            ));
        }
        let journal = open_or_create(&anchor, JOURNAL)?;
        anchor.validate()?;
        conn.execute_batch("PRAGMA journal_mode=TRUNCATE; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; PRAGMA secure_delete=ON; PRAGMA cache_size=-2048; PRAGMA max_page_count=32768;").map_err(storage)?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS vector_limits(singleton INTEGER PRIMARY KEY CHECK(singleton=1), entries INTEGER NOT NULL CHECK(entries BETWEEN 1 AND 100000), bytes INTEGER NOT NULL CHECK(bytes BETWEEN 256 AND 67108864));
            CREATE TABLE IF NOT EXISTS vectors(seq INTEGER PRIMARY KEY AUTOINCREMENT, model TEXT NOT NULL, content TEXT NOT NULL, dimensions INTEGER NOT NULL CHECK(dimensions BETWEEN 1 AND 4096), vector BLOB NOT NULL, checksum TEXT NOT NULL, bytes INTEGER NOT NULL CHECK(bytes=216+length(vector)), UNIQUE(model,content));").map_err(storage)?;
        transaction.execute("INSERT INTO vector_limits VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET entries=min(entries,excluded.entries),bytes=min(bytes,excluded.bytes)", params![capacity as i64, byte_budget as i64]).map_err(storage)?;
        transaction
            .execute_batch(&format!(
                "PRAGMA user_version=1; PRAGMA application_id={APPLICATION_ID};"
            ))
            .map_err(storage)?;
        validate_rows(&transaction)?;
        evict(&transaction)?;
        transaction.commit().map_err(storage)?;
        let cache = Self {
            conn,
            anchor,
            _database: database,
            _journal: journal,
        };
        cache.check_placement()?;
        Ok(cache)
    }
    fn check_placement(&self) -> Result<()> {
        require_derived_data_support()?;
        self.anchor.validate()?;
        let database = self.anchor.open_existing(DATABASE, true)?;
        let journal = self.anchor.open_existing(JOURNAL, true)?;
        validate_artifact_size(&database, DATABASE)?;
        validate_artifact_size(&journal, JOURNAL)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            for (held, named) in [(&self._database, &database), (&self._journal, &journal)] {
                let held = held.metadata()?;
                let named = named.metadata()?;
                if held.dev() != named.dev() || held.ino() != named.ino() {
                    return Err(Error::new(
                        "PATH_REFUSED",
                        "vector cache artifact identity changed",
                        8,
                    ));
                }
            }
        }
        Ok(())
    }
    /// Logical bounds and SQLite integrity, useful for explicit diagnostics/tests.
    pub fn usage(&self) -> Result<(usize, usize)> {
        self.check_placement()?;
        let (count, bytes): (i64, i64) = self
            .conn
            .query_row(
                "SELECT count(*),coalesce(sum(bytes),0) FROM vectors",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(storage)?;
        Ok((
            usize::try_from(count).map_err(|_| corrupt("invalid cache entry count"))?,
            usize::try_from(bytes).map_err(|_| corrupt("invalid cache byte count"))?,
        ))
    }
}
fn validate_rows(conn: &Connection) -> Result<()> {
    let physical: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(storage)?;
    if physical != "ok" {
        return Err(corrupt("SQLite cache integrity check failed"));
    }
    let invalid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM vectors WHERE length(model)!=64 OR length(content)!=64 OR length(checksum)!=64 OR dimensions NOT BETWEEN 1 AND 4096 OR length(vector)!=4*dimensions OR bytes!=216+length(vector))", [], |r|r.get(0)).map_err(storage)?;
    if invalid {
        return Err(corrupt("invalid cached row shape"));
    }
    let (count, bytes): (i64, i64) = conn
        .query_row(
            "SELECT count(*),coalesce(sum(bytes),0) FROM vectors",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(storage)?;
    if count > 100_000 || bytes > MAX_BYTES as i64 {
        return Err(corrupt("cache exceeds hard logical bounds"));
    }
    // Stream bounded rows before eviction: reopening with smaller bounds must not
    // erase existing corruption and turn it into an apparent cache miss.
    let mut statement = conn
        .prepare("SELECT model,content,dimensions,vector,checksum FROM vectors")
        .map_err(storage)?;
    let mut rows = statement.query([]).map_err(storage)?;
    while let Some(row) = rows.next().map_err(storage)? {
        let model: String = row.get(0).map_err(storage)?;
        let content: String = row.get(1).map_err(storage)?;
        let dimensions = usize::try_from(row.get::<_, i64>(2).map_err(storage)?)
            .map_err(|_| corrupt("invalid cached dimensions"))?;
        let bytes: Vec<u8> = row.get(3).map_err(storage)?;
        let expected: String = row.get(4).map_err(storage)?;
        decode(&model, &content, dimensions, &bytes, &expected)?;
    }
    Ok(())
}
fn decode(
    model: &str,
    content: &str,
    dimensions: usize,
    bytes: &[u8],
    expected: &str,
) -> Result<Vec<f32>> {
    if !valid_key(model)
        || !valid_key(content)
        || !(1..=4096).contains(&dimensions)
        || bytes.len() != dimensions * 4
        || bytes.len() > MAX_VECTOR_BYTES
        || expected != checksum(model, content, dimensions, bytes)
    {
        return Err(corrupt(
            "cached dimensions, keys, length or checksum mismatch",
        ));
    }
    let vector = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect::<Vec<_>>();
    if !vector_valid(&vector) {
        return Err(corrupt("cached vector is nonfinite or not normalized"));
    }
    Ok(vector)
}
fn evict(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM vectors WHERE seq IN (SELECT seq FROM (SELECT seq,row_number() OVER(ORDER BY seq DESC) AS n,sum(bytes) OVER(ORDER BY seq DESC) AS retained FROM vectors) WHERE n>(SELECT entries FROM vector_limits WHERE singleton=1) OR retained>(SELECT bytes FROM vector_limits WHERE singleton=1))", []).map_err(storage)?;
    Ok(())
}
impl VectorPersistence for SqliteVectorPersistence {
    fn get(
        &mut self,
        model_key: &str,
        content_hash: &str,
        dimensions: usize,
    ) -> Result<Option<Vec<f32>>> {
        keys(model_key, content_hash)?;
        if !(1..=4096).contains(&dimensions) {
            return Err(invalid("dimensions 1..4096 required"));
        }
        self.check_placement()?;
        let row = self.conn.query_row("SELECT dimensions,length(vector),CASE WHEN length(vector)<=16384 THEN vector ELSE NULL END,checksum FROM vectors WHERE model=?1 AND content=?2",params![model_key,content_hash], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<Vec<u8>>>(2)?,r.get::<_,String>(3)?))).optional().map_err(storage)?;
        let Some((stored_dimensions, length, bytes, expected)) = row else {
            return Ok(None);
        };
        let bytes = bytes.ok_or_else(|| corrupt("oversized cached vector"))?;
        if stored_dimensions != dimensions as i64
            || length != (dimensions * 4) as i64
            || bytes.len() > MAX_VECTOR_BYTES
            || expected != checksum(model_key, content_hash, dimensions, &bytes)
        {
            return Err(corrupt("cached dimensions, length or checksum mismatch"));
        }
        Ok(Some(decode(
            model_key,
            content_hash,
            dimensions,
            &bytes,
            &expected,
        )?))
    }
    fn put(&mut self, model_key: &str, content_hash: &str, vector: &[f32]) -> Result<()> {
        keys(model_key, content_hash)?;
        if !vector_valid(vector) {
            return Err(invalid(
                "finite normalized vector with dimensions 1..4096 required",
            ));
        }
        self.check_placement()?;
        let bytes = vector
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let transaction = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let budget: i64 = transaction
            .query_row(
                "SELECT bytes FROM vector_limits WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if bytes.len() + ROW_BYTES > budget as usize {
            return Err(invalid("vector exceeds persistent byte budget"));
        }
        transaction
            .execute(
                "DELETE FROM vectors WHERE model=?1 AND content=?2",
                params![model_key, content_hash],
            )
            .map_err(storage)?;
        transaction.execute("INSERT INTO vectors(model,content,dimensions,vector,checksum,bytes) VALUES(?1,?2,?3,?4,?5,?6)",params![model_key,content_hash,vector.len() as i64,&bytes,checksum(model_key,content_hash,vector.len(),&bytes),(bytes.len()+ROW_BYTES) as i64]).map_err(storage)?;
        evict(&transaction)?;
        transaction.commit().map_err(storage)?;
        self.check_placement()?;
        Ok(())
    }
}

/// Build an explicit cache from trusted configuration, without loading a model.
/// No automatic transport activation or removal of LOCAL_MODEL_GATE occurs here.
pub fn configured_cache(
    config: &kortex_config::Search,
    capacity: usize,
) -> Result<crate::hybrid::EmbeddingCache> {
    if config.cache_path.is_empty() {
        return crate::hybrid::EmbeddingCache::new(capacity);
    }
    let storage =
        SqliteVectorPersistence::open(Path::new(&config.cache_path), capacity, MAX_BYTES)?;
    crate::hybrid::EmbeddingCache::with_persistence(capacity, Box::new(storage))
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
