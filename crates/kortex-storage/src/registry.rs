//! Per-user repository registry backed by SQLite for concurrent updates.
use crate::db_error;
use kortex_core::{Error, Result, Snapshot};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
/// Only repository-selection provenance; never configuration or environment values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySelection {
    pub invocation_root: PathBuf,
    pub explicit_config: Option<PathBuf>,
    pub effective_root: PathBuf,
}
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
        kortex_repo::placement::require_derived_data_support()?;
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
        // An additive side table leaves the legacy registry shape and list contract intact.
        c.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS registry(id TEXT PRIMARY KEY,root TEXT NOT NULL,generation_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS repository_selection(id TEXT PRIMARY KEY REFERENCES registry(id) ON DELETE CASCADE,selection_json TEXT NOT NULL);").map_err(db_error)?;
        Ok(Self(c))
    }
    fn check_placement(&self) -> Result<()> {
        kortex_repo::placement::require_derived_data_support()?;
        let path = self.0.path().map(Path::new).ok_or_else(|| {
            Error::new("PATH_REFUSED", "registry requires an on-disk identity", 8)
        })?;
        crate::location::check_directory(
            path.parent()
                .ok_or_else(|| Error::new("PATH_REFUSED", "registry needs a parent", 8))?,
            false,
        )?;
        crate::location::check_file(path)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            crate::location::check_file(Path::new(&name))?;
        }
        Ok(())
    }
    pub fn register(&self, s: &Snapshot) -> Result<()> {
        self.check_placement()?;
        let tx = self.0.unchecked_transaction().map_err(db_error)?;
        // A legacy generation refresh at the same identity retains known provenance.
        // A changed root must not inherit another selection's configuration path.
        tx.execute("DELETE FROM repository_selection WHERE id=?1 AND EXISTS(SELECT 1 FROM registry WHERE id=?1 AND root<>?2)", params![s.repo_id, s.root]).map_err(db_error)?;
        tx.execute("INSERT INTO registry(id,root,generation_id) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET root=excluded.root,generation_id=excluded.generation_id",params![s.repo_id,s.root,s.generation_id]).map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    pub fn register_with_selection(
        &self,
        s: &Snapshot,
        selection: &RepositorySelection,
    ) -> Result<()> {
        self.check_placement()?;
        if !selection.invocation_root.is_absolute()
            || !selection.effective_root.is_absolute()
            || selection
                .explicit_config
                .as_ref()
                .is_some_and(|p| !p.is_absolute())
            || kortex_core::public_path(Path::new(&s.root))
                != kortex_core::public_path(&selection.effective_root)
        {
            return Err(Error::new(
                "REPO_SELECTION",
                "selection paths must be absolute and effective root must match the snapshot",
                2,
            ));
        }
        let raw = serde_json::to_string(selection)?;
        let tx = self.0.unchecked_transaction().map_err(db_error)?;
        tx.execute("INSERT INTO registry(id,root,generation_id) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET root=excluded.root,generation_id=excluded.generation_id",params![s.repo_id,s.root,s.generation_id]).map_err(db_error)?;
        tx.execute("INSERT INTO repository_selection(id,selection_json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET selection_json=excluded.selection_json", params![s.repo_id, raw]).map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    /// None means a legacy registration with unknown provenance, or an absent ID.
    /// Callers must not guess a configuration path by walking ancestors.
    pub fn selection(&self, id: &str) -> Result<Option<RepositorySelection>> {
        let row: Option<(String, String)> = self.0.query_row("SELECT s.selection_json,r.root FROM repository_selection s JOIN registry r ON r.id=s.id WHERE s.id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_error)?;
        row.map(|(raw, root)| {
            let selection: RepositorySelection = serde_json::from_str(&raw)
                .map_err(|e| Error::new("CORRUPT_REGISTRY", e.to_string(), 6))?;
            if !selection.invocation_root.is_absolute()
                || !selection.effective_root.is_absolute()
                || selection
                    .explicit_config
                    .as_ref()
                    .is_some_and(|p| !p.is_absolute())
                || kortex_core::public_path(Path::new(&root))
                    != kortex_core::public_path(&selection.effective_root)
            {
                return Err(Error::new(
                    "CORRUPT_REGISTRY",
                    "invalid repository selection provenance",
                    6,
                ));
            }
            Ok(selection)
        })
        .transpose()
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
        self.check_placement()?;
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
