//! Index placement and fail-closed filesystem checks. No repository-controlled cache paths.
use kortex_core::{Error, Result};
use std::{
    fs::{self, File, OpenOptions},
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
pub(crate) fn check_index_directory(directory: &Path, storage: &str, create: bool) -> Result<()> {
    if storage == "user-cache" {
        // Validate the cache home, kortex and indexes parents as well as the hash leaf.
        for ancestor in directory
            .ancestors()
            .skip(1)
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            check_directory(ancestor, create)?;
        }
    }
    check_directory(directory, create)
}
pub(crate) fn check_directory(path: &Path, create: bool) -> Result<()> {
    // Roots/cache homes are normalized first. Never follow a planted descendant symlink.
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(refused("index directory ancestors cannot be symlinks"));
        }
    }
    match fs::symlink_metadata(path) {
        Ok(m) => {
            if !m.is_dir() || m.file_type().is_symlink() {
                return Err(refused(
                    "index directory must not be a symlink or non-directory",
                ));
            }
            owner(&m)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o022 != 0 {
                    return Err(refused("index directory must not be group/world writable"));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            let parent = path
                .parent()
                .ok_or_else(|| refused("index directory needs a parent"))?;
            if !parent.exists() {
                check_directory(parent, true)?;
            } else if fs::symlink_metadata(parent)?.file_type().is_symlink() {
                return Err(refused("index parent cannot be a symlink"));
            }
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            check_directory(path, false)?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
/// Validate reserved index ownership metadata without ever modifying existing records.
pub(crate) fn identity(directory: &Path, root: &Path, create: bool) -> Result<()> {
    use std::io::{Read, Write};
    let path = directory.join("index-owner.json");
    check_file(&path)?;
    let expected = serde_json::json!({"schema_version": 1, "canonical_root": root.canonicalize()?.to_str(), "identity": kortex_core::hash(root.canonicalize()?.to_string_lossy().as_bytes())});
    if path.exists() {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
        }
        let mut bytes = vec![];
        options
            .open(&path)?
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
                if m.nlink() != 1 {
                    return Err(refused("index artifacts cannot be hard links"));
                }
                if m.mode() & 0o022 != 0 {
                    return Err(refused("index artifacts must not be group/world writable"));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
pub(crate) fn open_private(path: &Path, exclusive: bool) -> Result<File> {
    check_file(path)?;
    let mut opts = OpenOptions::new();
    opts.read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .create_new(exclusive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let file = opts.open(path)?;
    owner(&file.metadata()?)?;
    check_file(path)?;
    Ok(file)
}
