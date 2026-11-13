//! Descriptor/handle-relative reads: canonicalization alone is not a confinement boundary.
use kortex_core::{Error, Result};
use std::{fs::File, io::Read, path::Path};

#[cfg(test)]
type ReadHook = Option<Box<dyn FnMut(usize)>>;
#[cfg(test)]
std::thread_local! {
    // Inject cancellation only AFTER a real native read, on the calling test's
    // thread. No hook exists in production builds or other concurrently run tests.
    pub(crate) static READ_HOOK: std::cell::RefCell<ReadHook> = std::cell::RefCell::new(None);
}

/// Shared byte-read path, also exercised after deterministic resolution/open swaps.
pub(crate) fn read_bytes(root: &Path, canonical: &Path, max: u64) -> Result<Vec<u8>> {
    read_bytes_with_control(root, canonical, max, None)
}

pub(crate) fn read_bytes_with_control(
    root: &Path,
    canonical: &Path,
    max: u64,
    control: Option<&kortex_core::RequestControl>,
) -> Result<Vec<u8>> {
    crate::checkpoint(control)?;
    let file = open(root, canonical)?;
    if !file.metadata()?.is_file() {
        return Err(Error::new("PATH_REFUSED", "not a regular file", 8));
    }
    read_opened_bytes(file, max, control)
}

pub(crate) fn read_opened_bytes(
    file: File,
    max: u64,
    control: Option<&kortex_core::RequestControl>,
) -> Result<Vec<u8>> {
    read_opened_bytes_counted(file, max, control, &mut crate::ConfinedReadStats::default())
}

pub(crate) const READ_CHUNK_BYTES: usize = 64 * 1024;
pub(crate) type ReadScratch = [u8; READ_CHUNK_BYTES];

pub(crate) fn read_opened_bytes_counted(
    file: File,
    max: u64,
    control: Option<&kortex_core::RequestControl>,
    stats: &mut crate::ConfinedReadStats,
) -> Result<Vec<u8>> {
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    read_opened_bytes_with_scratch(file, max, control, stats, &mut chunk)
}

pub(crate) fn read_opened_bytes_with_scratch(
    file: File,
    max: u64,
    control: Option<&kortex_core::RequestControl>,
    stats: &mut crate::ConfinedReadStats,
    chunk: &mut ReadScratch,
) -> Result<Vec<u8>> {
    let mut reader = file.take(max.saturating_add(1));
    let mut bytes = Vec::new();
    loop {
        crate::checkpoint(control)?;
        stats.read_calls += 1;
        let count = match reader.read(chunk) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        stats.bytes_read += count as u64;
        crate::checkpoint(control)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        #[cfg(test)]
        READ_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(count);
            }
        });
    }
    if bytes.len() as u64 > max {
        return Err(Error::new(
            "FILE_TOO_LARGE",
            "file exceeds configured size limit",
            7,
        ));
    }
    Ok(bytes)
}
#[cfg(unix)]
pub(crate) fn open(root: &Path, canonical: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, open};
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| Error::new("PATH_REFUSED", "path escaped root", 8))?;
    let directory = File::from(
        open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    open_anchored(&directory, relative)
}

#[cfg(unix)]
pub(crate) fn open_anchored(root: &File, relative: &Path) -> Result<File> {
    open_anchored_impl(root, relative, None)
}

#[cfg(unix)]
pub(crate) fn open_anchored_with_parents(
    root: &File,
    relative: &Path,
) -> Result<(File, Vec<std::fs::Metadata>)> {
    let mut parents = Vec::new();
    let file = open_anchored_impl(root, relative, Some(&mut parents))?;
    Ok((file, parents))
}

#[cfg(unix)]
fn open_anchored_impl(
    root: &File,
    relative: &Path,
    mut parents: Option<&mut Vec<std::fs::Metadata>>,
) -> Result<File> {
    use rustix::fs::{Mode, OFlags, openat};
    let mut components = relative.components().peekable();
    let mut directory: Option<File> = None;
    while let Some(component) = components.next() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(Error::new("PATH_REFUSED", "non-normal confined path", 8));
        }
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if components.peek().is_some() {
            flags |= OFlags::DIRECTORY;
        }
        let next = openat(
            directory.as_ref().unwrap_or(root),
            component.as_os_str(),
            flags,
            Mode::empty(),
        )
        .map_err(|error| {
            Error::new(
                "PATH_REFUSED",
                format!("path changed or became inaccessible during confined read: {error}"),
                8,
            )
        })?;
        if components.peek().is_none() {
            return Ok(File::from(next));
        }
        let next = File::from(next);
        if let Some(parents) = parents.as_mut() {
            parents.push(next.metadata()?);
        }
        directory = Some(next);
    }
    Err(Error::new(
        "PATH_REFUSED",
        "expected a source file below root",
        8,
    ))
}
#[cfg(windows)]
pub(crate) fn open(root: &Path, canonical: &Path) -> Result<File> {
    let directory = windows_root(root)?;
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| Error::new("PATH_REFUSED", "path escaped root", 8))?;
    windows_open(&directory, relative)
}

#[cfg(windows)]
pub(crate) fn windows_root(root: &Path) -> Result<cap_std::fs::Dir> {
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
    // No SHARE_DELETE: cap-std's source anchor cannot be renamed/deleted while
    // resolving children. Initial ancestors are trusted, as on Unix. This is
    // source confinement, NOT private derived-data ACL enforcement.
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
    Ok(cap_std::fs::Dir::from_std_file(anchor))
}

#[cfg(windows)]
pub(crate) fn windows_open(directory: &cap_std::fs::Dir, relative: &Path) -> Result<File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::OpenOptions;
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    let refused = |error: std::io::Error| {
        Error::new("PATH_REFUSED", format!("confined open failed: {error}"), 8)
    };
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
