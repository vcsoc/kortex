//! Per-user repository registry backed by SQLite for concurrent updates.
use crate::db_error;
use kortex_core::{Error, Result, Snapshot};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
pub struct Registry(Connection);
impl Registry {
    pub fn path() -> Result<PathBuf> {
        if let Some(p) = std::env::var_os("KORTEX_REGISTRY") {
            return Ok(p.into());
        }
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
            .ok_or_else(|| {
                Error::new(
                    "CONFIG_HOME",
                    "set KORTEX_REGISTRY when no home directory exists",
                    2,
                )
            })?;
        Ok(home.join("kortex/registry.db"))
    }
    pub fn open() -> Result<Self> {
        Self::at(&Self::path()?)
    }
    pub fn at(path: &Path) -> Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let parent = crate::location::canonical_cache_base(
            absolute
                .parent()
                .ok_or_else(|| Error::new("PATH_REFUSED", "registry needs a parent", 8))?,
        )?;
        crate::location::check_directory(&parent, true)?;
        let path = parent.join(
            absolute
                .file_name()
                .ok_or_else(|| Error::new("PATH_REFUSED", "registry needs a filename", 8))?,
        );
        crate::location::check_file(&path)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = path.file_name().unwrap().to_os_string();
            name.push(suffix);
            crate::location::check_file(&path.with_file_name(name))?;
        }
        drop(crate::location::open_private(&path, false)?);
        let c = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        c.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS registry(id TEXT PRIMARY KEY,root TEXT NOT NULL,generation_id TEXT NOT NULL);").map_err(db_error)?;
        Ok(Self(c))
    }
    pub fn register(&self, s: &Snapshot) -> Result<()> {
        self.0.execute("INSERT INTO registry VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET root=excluded.root,generation_id=excluded.generation_id",params![s.repo_id,s.root,s.generation_id]).map_err(db_error)?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .0
            .prepare("SELECT id,root,generation_id FROM registry ORDER BY root,id")
            .map_err(db_error)?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        if self
            .0
            .execute("DELETE FROM registry WHERE id=?1", [id])
            .map_err(db_error)?
            == 0
        {
            return Err(Error::new("REPO_NOT_FOUND", id, 3));
        }
        Ok(())
    }
}
