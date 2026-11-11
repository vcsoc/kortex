//! Explicit migration and evidence-preserving repair. Never deletes source/config/old indexes.
use crate::{SCHEMA, Store, acquire_writer, db_error, location};
use kortex_core::{Error, Result};
use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
#[path = "preservation_tests.rs"]
mod preservation_tests;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn unique_path(directory: &Path, prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    directory.join(format!(
        "{prefix}-{nanos}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}
pub(crate) fn migrate(conn: &Connection, directory: &Path, version: i64) -> Result<()> {
    if version == 1 {
        let recorded: i64 = conn
            .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .map_err(db_error)?;
        if recorded != version {
            return Err(Error::new(
                "INDEX_VERSION",
                "schema version and migration records disagree",
                6,
            ));
        }
        return Ok(());
    }
    if version != 0 {
        return Err(Error::new(
            "INDEX_VERSION",
            "unsupported migration source",
            6,
        ));
    }
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if tables > 0 {
        // Only the known pre-versioned Kortex shape is migratable. Do not adopt arbitrary DBs.
        for query in [
            "SELECT snapshot FROM generations LIMIT 0",
            "SELECT facts_json FROM files LIMIT 0",
            "SELECT data_json FROM nodes LIMIT 0",
            "SELECT data_json FROM edges LIMIT 0",
            "SELECT root FROM repositories LIMIT 0",
            "SELECT active_generation_id FROM index_state LIMIT 0",
            "SELECT symbol_id,generation_id FROM symbol_fts LIMIT 0",
        ] {
            conn.prepare(query).map_err(|_| {
                Error::new(
                    "INDEX_VERSION",
                    "unrecognized legacy schema; explicit repair/rebuild required",
                    6,
                )
            })?;
        }
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if integrity != "ok" {
            return Err(Error::new("CORRUPT_INDEX", integrity, 6));
        }
        // A consistent SQLite backup includes committed WAL pages, unlike copying just index.db.
        // Always back up recognized legacy DBs, even though this migration is additive.
        let backup = unique_path(directory, "migration-0-to-1.db");
        drop(location::open_private(&backup, true)?);
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])
            .map_err(db_error)?;
        fs::File::open(&backup)?.sync_all()?;
    }
    conn.execute_batch("BEGIN IMMEDIATE").map_err(db_error)?;
    let result = conn.execute_batch(SCHEMA).and_then(|()| conn.execute_batch("CREATE TABLE IF NOT EXISTS storage_versions(key TEXT PRIMARY KEY, value TEXT NOT NULL);")).and_then(|()| {
        conn.execute("INSERT OR REPLACE INTO storage_versions VALUES('application',?1),('parser',?2),('schema','1')", [env!("CARGO_PKG_VERSION"), kortex_core::PARSER_VERSION]).map(|_| ())
    }).and_then(|()| conn.execute_batch("COMMIT"));
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result.map_err(db_error)
}

/// Never overwrite evidence that appeared after an earlier existence check.
/// This is atomic on supported Unix targets; it is not ancestor confinement.
fn preserve_move(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(any(
        target_vendor = "apple",
        target_os = "linux",
        target_os = "android",
        target_os = "redox"
    ))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)
    }
    #[cfg(windows)]
    {
        fn path(path: &Path) -> std::io::Result<&str> {
            path.to_str()
                .filter(|path| !path.contains('\0'))
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "repair path must be UTF-8 without NUL",
                    )
                })
        }
        // Unlike std::fs::rename (MoveFileExW + REPLACE_EXISTING), MoveFileW
        // refuses an existing destination. No first-party unsafe/FFI is used.
        winsafe::MoveFile(path(from)?, path(to)?)
            .map_err(|error| std::io::Error::from_raw_os_error(error.raw() as i32))
    }
    #[cfg(not(any(
        windows,
        target_vendor = "apple",
        target_os = "linux",
        target_os = "android",
        target_os = "redox"
    )))]
    {
        // No safe atomic no-replace primitive is wired on these targets yet.
        // Fail closed rather than use rename, which can erase raced evidence.
        let _ = (from, to);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic evidence-preserving repair moves are unsupported on this platform",
        ))
    }
}

fn has_future_version(directory: &Path) -> Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    fn future(header: &[u8]) -> bool {
        header.len() >= 64
            && &header[..16] == b"SQLite format 3\0"
            && u32::from_be_bytes(header[60..64].try_into().unwrap()) > 1
    }
    let mut header = [0u8; 100];
    let mut file = fs::File::open(directory.join("index.db"))?;
    let length = file.read(&mut header)?;
    if future(&header[..length]) {
        return Ok(true);
    }
    let wal_path = directory.join("index.db-wal");
    let mut wal = match fs::File::open(&wal_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut wal_header = [0u8; 32];
    if wal.read(&mut wal_header)? != 32 {
        return Ok(false);
    }
    let magic = u32::from_be_bytes(wal_header[..4].try_into().unwrap());
    let page_size = u32::from_be_bytes(wal_header[8..12].try_into().unwrap()) as u64;
    if ![0x377f0682, 0x377f0683].contains(&magic)
        || !(512..=65536).contains(&page_size)
        || !page_size.is_power_of_two()
    {
        return Ok(false);
    }
    let length = wal.metadata()?.len();
    let frame_size = page_size + 24;
    let frames = length.saturating_sub(32) / frame_size;
    if frames > 1_000_000 {
        return Err(Error::new(
            "REPAIR_REFUSED",
            "WAL too large for bounded version inspection; preserve and inspect manually",
            6,
        ));
    }
    for frame in 0..frames {
        wal.seek(SeekFrom::Start(32 + frame * frame_size))?;
        let mut frame_header = [0u8; 24];
        wal.read_exact(&mut frame_header)?;
        if frame_header[8..16] != wal_header[16..24] {
            continue;
        }
        if u32::from_be_bytes(frame_header[..4].try_into().unwrap()) == 1 {
            wal.read_exact(&mut header)?;
            // Conservatively refuse even an uncommitted/newer page-one frame. No future data loss.
            if future(&header) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[derive(Debug, Clone)]
pub struct RepairReport {
    /// Original files were moved, not rewritten: bytes, Unix owner, permissions and inode survive.
    pub backup_directory: PathBuf,
    pub preserved_files: Vec<String>,
}
impl Store {
    /// Explicitly quarantine an index before rebuilding. Returns no synthetic rebuilt generation.
    /// Caller must close all Store handles first, then reindex; failure leaves this backup intact.
    /// The writer lock, source files, .kortex.toml and unknown files are never moved/deleted.
    pub fn repair(root: &Path, storage: &str) -> Result<RepairReport> {
        let directory = Self::directory_for(root, storage)?;
        location::check_index_directory(&directory, storage, false)?;
        let _lock = acquire_writer(&directory)?;
        let names = [
            "index.db",
            "index.db-wal",
            "index.db-shm",
            "index.db-journal",
            "index-owner.json",
        ];
        for name in names {
            location::check_file(&directory.join(name))?;
        }
        if !directory.join("index.db").is_file() {
            return Err(Error::new("INDEX_NOT_FOUND", "no database to preserve", 3));
        }
        // Inspect headers only, including page-one WAL frames; never let SQLite checkpoint,
        // delete sidecars or rewrite potentially corrupt evidence while deciding to repair.
        if has_future_version(&directory)? {
            return Err(Error::new(
                "INDEX_VERSION",
                "refusing repair of a newer index",
                6,
            ));
        }
        let backup = unique_path(&directory, "repair");
        // Atomic mkdir preserves prior evidence; Unix permissions are private at creation.
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&backup)?;
        // Durable recovery inventory exists before any rename, including if interrupted midway.
        use std::io::Write;
        let mut manifest = location::open_private(&backup.join("preservation.json"), true)?;
        manifest.write_all(serde_json::to_vec_pretty(&serde_json::json!({"schema_version": 1, "source_directory": directory, "artifacts": names, "action": "preserve-before-rebuild"}))?.as_slice())?;
        manifest.sync_all()?;
        #[cfg(unix)]
        {
            fs::File::open(&backup)?.sync_all()?;
            fs::File::open(&directory)?.sync_all()?;
        }
        let mut moved: Vec<String> = vec![];
        for name in names {
            let from = directory.join(name);
            if !from.exists() {
                continue;
            }
            if let Err(error) = preserve_move(&from, &backup.join(name)) {
                // Best-effort rollback; never delete either copy if a platform denies the move.
                let mut rollback_failed = false;
                for previous in moved.iter().rev() {
                    if preserve_move(&backup.join(previous), &directory.join(previous)).is_err() {
                        rollback_failed = true;
                    }
                }
                return Err(Error::new(
                    "REPAIR_FAILED",
                    format!(
                        "preserved evidence at {}: {error}; rollback_failed={rollback_failed}",
                        backup.display()
                    ),
                    6,
                ));
            }
            moved.push(name.to_owned());
        }
        // Persist directory entries before reporting successful preservation.
        #[cfg(unix)]
        {
            for path in [&backup, &directory] {
                fs::File::open(path)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| {
                        Error::new(
                            "REPAIR_FAILED",
                            format!(
                                "evidence preserved at {}; directory sync failed: {error}",
                                backup.display()
                            ),
                            6,
                        )
                    })?;
            }
        }
        Ok(RepairReport {
            backup_directory: backup,
            preserved_files: moved,
        })
    }
}
