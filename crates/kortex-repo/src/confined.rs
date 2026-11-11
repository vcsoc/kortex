//! Descriptor/handle-relative reads: canonicalization alone is not a confinement boundary.
use kortex_core::{Error, Result};
use std::{fs::File, path::Path};
#[cfg(unix)]
pub(crate) fn open(root: &Path, canonical: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| Error::new("PATH_REFUSED", "path escaped root", 8))?;
    let mut components = relative.components().peekable();
    let mut directory = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    while let Some(component) = components.next() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(Error::new("PATH_REFUSED", "non-normal confined path", 8));
        }
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if components.peek().is_some() {
            flags |= OFlags::DIRECTORY;
        }
        let next =
            openat(&directory, component.as_os_str(), flags, Mode::empty()).map_err(|error| {
                Error::new(
                    "PATH_REFUSED",
                    format!("path changed or became inaccessible during confined read: {error}"),
                    8,
                )
            })?;
        if components.peek().is_none() {
            return Ok(File::from(next));
        }
        directory = next;
    }
    Err(Error::new(
        "PATH_REFUSED",
        "expected a source file below root",
        8,
    ))
}
#[cfg(windows)]
pub(crate) fn open(root: &Path, canonical: &Path) -> Result<File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let refused = |error: std::io::Error| {
        Error::new(
            "PATH_REFUSED",
            format!("path changed or became inaccessible during confined read: {error}"),
            8,
        )
    };
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| Error::new("PATH_REFUSED", "path escaped root", 8))?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            !matches!(component, std::path::Component::Normal(_))
                || component.as_os_str().to_str().is_none_or(|name| {
                    // NTFS streams are not source paths. Reject Win32 aliases whose
                    // normalization could hide a device name or change identity.
                    name.contains(':') || name.ends_with(['.', ' '])
                })
        })
    {
        return Err(Error::new("PATH_REFUSED", "non-normal confined path", 8));
    }

    // Do not follow a root replaced with a junction/symlink. Omitting SHARE_DELETE
    // is required by Dir::from_std_file: the anchor cannot be renamed/deleted
    // while cap-std resolves children. The root and its initial ancestors are
    // trusted input, as on Unix; children are never opened by an ambient path.
    let anchor = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)
        .map_err(refused)?;
    let metadata = anchor.metadata().map_err(refused)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error::new(
            "PATH_REFUSED",
            "root changed to a reparse point",
            8,
        ));
    }
    let directory = Dir::from_std_file(anchor);
    // cap-std walks components with handle-relative NtCreateFile, manually
    // resolves parent reparse points within the anchor, and denies device names.
    // Canonical aliases resolved by the caller still work. A replaced final
    // component must not be followed, even if it happens to point inside root.
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(relative, &options)
        .map_err(refused)?
        .into_std();
    let metadata = file.metadata().map_err(refused)?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error::new("PATH_REFUSED", "not a regular source file", 8));
    }
    Ok(file)
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn open(_root: &Path, _canonical: &Path) -> Result<File> {
    Err(Error::new(
        "PATH_REFUSED",
        "confined reads are not supported on this platform",
        8,
    ))
}

#[cfg(all(test, windows))]
#[path = "confined_windows_tests.rs"]
mod windows_tests;
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn descriptor_walk_refuses_replaced_leaf_parent_and_fifo_without_blocking() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "secret").unwrap();
        let source = root.join("source.rs");
        std::fs::write(&source, "safe").unwrap();
        let canonical = source.canonicalize().unwrap();
        std::fs::remove_file(&source).unwrap();
        symlink(outside.path().join("secret.rs"), &source).unwrap();
        assert_eq!(open(&root, &canonical).unwrap_err().code, "PATH_REFUSED");
        let parent = root.join("nested");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("secret.rs"), "safe").unwrap();
        let canonical = parent.join("secret.rs").canonicalize().unwrap();
        std::fs::rename(&parent, root.join("old")).unwrap();
        symlink(outside.path(), &parent).unwrap();
        assert_eq!(open(&root, &canonical).unwrap_err().code, "PATH_REFUSED");
        let fifo = root.join("pipe.rs");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            crate::read_confined(&root, "pipe.rs", 100)
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }
}
