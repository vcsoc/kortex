//! Index placement and fail-closed filesystem checks. No repository-controlled cache paths.
use kortex_core::{Error, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(crate) fn refused(message: impl std::fmt::Display) -> Error {
    Error::new("PATH_REFUSED", message.to_string(), 8)
}
pub fn directory_for(root: &Path, storage: &str) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    if !root.is_dir() || root.to_str().is_none() {
        return Err(refused("repository root must be a UTF-8 directory"));
    }
    match storage {
        "repository" => Ok(root.join(".kortex")),
        "user-cache" => {
            let base = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
                .ok_or_else(|| {
                    Error::new(
                        "CONFIG_HOME",
                        "user-cache requires XDG_CACHE_HOME, LOCALAPPDATA or HOME",
                        2,
                    )
                })?;
            if !base.is_absolute()
                || base
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(refused("cache home must be absolute without traversal"));
            }
            // Canonical root, not basename/remote URL: distinct clones never share a writer.
            let base = canonical_cache_base(&base)?;
            Ok(base
                .join("kortex/indexes")
                .join(kortex_core::hash(root.to_string_lossy().as_bytes())))
        }
        _ => Err(Error::new(
            "INVALID_CONFIG",
            "storage must be repository or user-cache",
            2,
        )),
    }
}
pub(crate) fn canonical_cache_base(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = vec![];
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(refused("cache home cannot be a symlink/non-directory"));
                }
                let mut resolved = ancestor.canonicalize()?;
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| refused("invalid cache home"))?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| refused("cache home needs an absolute parent"))?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
pub(crate) fn check_index_directory(directory: &Path, _storage: &str, create: bool) -> Result<()> {
    // The anchor now checks ALL ancestors, not only three cache parents. Existing
    // non-writable cache-home ancestors may be readable; the artifact leaf is private.
    check_directory(directory, create)
}
pub(crate) fn check_directory(path: &Path, create: bool) -> Result<()> {
    // Full descriptor-walk ancestor policy. This does NOT turn subsequent ambient
    // SQLite/repair paths into handle-bound operations, nor prove ACL privacy.
    kortex_repo::placement::DirectoryAnchor::acquire(path, create).map(|_| ())
}
/// Validate reserved index ownership metadata without ever modifying existing records.
pub(crate) fn identity(directory: &Path, root: &Path, create: bool) -> Result<()> {
    use std::io::{Read, Write};
    let path = directory.join("index-owner.json");
    check_file(&path)?;
    let expected = serde_json::json!({"schema_version": 1, "canonical_root": root.canonicalize()?.to_str(), "identity": kortex_core::hash(root.canonicalize()?.to_string_lossy().as_bytes())});
    if path.exists() {
        let anchor = kortex_repo::placement::DirectoryAnchor::acquire(directory, false)?;
        let mut bytes = vec![];
        anchor
            .open_existing("index-owner.json", false)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        let actual = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
        if bytes.len() > 16 * 1024 || actual.as_ref() != Some(&expected) {
            return Err(Error::new(
                "INDEX_IDENTITY",
                "index ownership metadata differs from this canonical repository root; explicitly preserve/repair and rebuild",
                6,
            ));
        }
    } else if create {
        let mut file = open_private(&path, true)?;
        file.write_all(&serde_json::to_vec(&expected)?)?;
        file.sync_all()?;
    }
    Ok(())
}
#[cfg(unix)]
pub(crate) fn owner(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(refused("index artifacts must be owned by the current user"));
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}
pub(crate) fn check_file(path: &Path) -> Result<()> {
    kortex_repo::placement::require_derived_data_support()?;
    #[cfg(unix)]
    {
        for _ in 0..8 {
            match fs::symlink_metadata(path) {
                Ok(m) => {
                    if !m.is_file() || m.file_type().is_symlink() {
                        return Err(refused(
                            "index artifacts must be regular files, never symlinks",
                        ));
                    }
                    owner(&m)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if m.nlink() == 0 {
                            // An optional SQLite sidecar was unlinked during stat.
                            continue;
                        }
                        if m.nlink() != 1 {
                            return Err(refused("index artifacts cannot be hard links"));
                        }
                        if m.mode() & 0o077 != 0 {
                            return Err(refused("index artifacts must have no group/world access"));
                        }
                    }
                    let parent = path
                        .parent()
                        .ok_or_else(|| refused("artifact needs a parent"))?;
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| refused("artifact name must be UTF-8"))?;
                    let anchor = kortex_repo::placement::DirectoryAnchor::acquire(parent, false)?;
                    match anchor.open_existing(name, false) {
                        Ok(_) => return Ok(()),
                        Err(error) => match fs::symlink_metadata(path) {
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                            Ok(current) => {
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::MetadataExt;
                                    if (m.dev(), m.ino()) != (current.dev(), current.ino()) {
                                        // Revalidate a genuinely replaced journal, never
                                        // downgrade an existing unsafe file to absence.
                                        continue;
                                    }
                                }
                                #[cfg(not(unix))]
                                let _ = current;
                                return Err(error);
                            }
                            Err(_) => return Err(error),
                        },
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
        Err(refused(
            "artifact changed repeatedly during privacy validation",
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        unreachable!("unsupported capability returned before I/O")
    }
}
pub(crate) fn open_private(path: &Path, exclusive: bool) -> Result<File> {
    kortex_repo::placement::require_derived_data_support()?;
    let parent = path
        .parent()
        .ok_or_else(|| refused("artifact needs a parent"))?;
    let anchor = kortex_repo::placement::DirectoryAnchor::acquire(parent, false)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| refused("artifact name must be UTF-8"))?;
    if exclusive || !path.try_exists()? {
        return anchor.create_new(name);
    }
    anchor.open_existing(name, true)
}
