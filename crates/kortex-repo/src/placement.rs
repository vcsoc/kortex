//! Descriptor-relative private storage placement, not a SQLite VFS.
//!
//! UID 0, the current effective UID and the filesystem administrator are trusted.
//! Darwin additionally refuses extended ALLOW ACEs, including inherit-only ACEs;
//! deny-only defaults remain usable. No ACLs or existing modes are modified.
//! Unknown Unix ACL models and network filesystems need separate review. Windows
//! derived-data ownership/DACL enforcement is unsupported and fails before I/O.
//! Acquiring this anchor does not make subsequent ambient SQLite opens handle-bound.
use kortex_core::{Error, Result};
use std::{fs::File, path::Path};
#[cfg(target_os = "macos")]
#[path = "placement_acl.rs"]
mod acl;
#[cfg(target_os = "macos")]
use acl::{validate_acl_at, validate_bound_acl};

fn refused(message: impl std::fmt::Display) -> Error {
    Error::new("PATH_REFUSED", message.to_string(), 8)
}

/// Fail closed where a reviewed owner-restricted creation policy is unavailable.
/// Call this BEFORE creating derived-data directories, locks, DBs or registries.
pub fn require_derived_data_support() -> Result<()> {
    #[cfg(unix)]
    {
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(Error::new(
            "PRIVATE_STORAGE_UNSUPPORTED",
            "derived-data owner/DACL privacy is not implemented on this platform; no data was created",
            8,
        ))
    }
}

/// An opened directory identity for descriptor-relative creation, not a path token.
///
/// All ancestors must be owned by the current user or root. Non-sticky group/world
/// writable ancestors are refused even if the final directory is 0700. Sticky
/// ancestors are permitted only with trusted ownership of each child. Final
/// directories must belong to the current user and have no group/world mode bits.
/// Retaining this descriptor does not prevent rename on Unix, but operations on it
/// remain bound to the original directory after an accidental concurrent rename.
#[derive(Debug)]
pub struct DirectoryAnchor {
    #[cfg(unix)]
    directory: File,
}

impl DirectoryAnchor {
    /// Walk from `/` with NOFOLLOW and validate each actual descriptor before use.
    /// Missing components are created 0700 only when `create` is true. Existing
    /// objects are never chmodded, adopted, truncated or followed through symlinks.
    ///
    /// Pass a canonical absolute path; do not canonicalize an attacker-modifiable
    /// alias and assume its original spelling is now safe. Initial root acquisition
    /// and concurrent changes by trusted users remain explicit trust boundaries.
    pub fn acquire(path: &Path, create: bool) -> Result<Self> {
        require_derived_data_support()?;
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
            use std::path::Component;
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
            {
                return Err(refused("storage placement needs a normal absolute path"));
            }
            let mut components = path
                .components()
                .filter_map(|c| match c {
                    Component::Normal(name) => Some(name),
                    _ => None,
                })
                .peekable();
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let mut directory = File::from(open("/", flags, Mode::empty()).map_err(refused)?);
            #[cfg(target_os = "macos")]
            let mut checked_path = std::path::PathBuf::from("/");
            validate_directory(&directory, components.peek().is_none())?;
            #[cfg(target_os = "macos")]
            validate_acl_at(&directory, &checked_path)?;
            while let Some(name) = components.next() {
                let next = match openat(&directory, name, flags, Mode::empty()) {
                    Ok(next) => next,
                    Err(rustix::io::Errno::NOENT) if create => {
                        match mkdirat(&directory, name, Mode::from_raw_mode(0o700)) {
                            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                            Err(error) => return Err(refused(error)),
                        }
                        openat(&directory, name, flags, Mode::empty()).map_err(refused)?
                    }
                    Err(error) => return Err(refused(error)),
                };
                directory = File::from(next);
                validate_directory(&directory, components.peek().is_none())?;
                #[cfg(target_os = "macos")]
                {
                    checked_path.push(name);
                    // The preceding trusted directory was checked before opening
                    // this child. No foreign UID can replace this trusted-owned
                    // name (including beneath a trusted sticky parent).
                    validate_acl_at(&directory, &checked_path)?;
                }
            }
            Ok(Self { directory })
        }
        #[cfg(not(unix))]
        {
            let _ = (path, create);
            unreachable!("unsupported capability returned above")
        }
    }

    /// Create a new owner-restricted regular file relative to the held descriptor.
    /// Refuse existing files (including hard links/symlinks) without changing them.
    /// Validation precedes returning a writable handle: derived bytes cannot be
    /// written by this API before permissions/type/UID/link-count checks succeed.
    pub fn create_new(&self, name: &str) -> Result<File> {
        require_derived_data_support()?;
        validate_name(name)?;
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, openat};
            self.validate()?;
            let file = File::from(
                openat(
                    &self.directory,
                    name,
                    OFlags::RDWR
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                        | OFlags::NONBLOCK,
                    Mode::from_raw_mode(0o600),
                )
                .map_err(refused)?,
            );
            validate_file(&file)?;
            #[cfg(target_os = "macos")]
            validate_bound_acl(&file, false)?;
            Ok(file)
        }
        #[cfg(not(unix))]
        unreachable!("unsupported capability returned above")
    }

    /// Recheck held-directory privacy; does not pin ambient SQLite pathname I/O.
    /// Trusted-user changes can race this check; hostile same-UID isolation is
    /// deliberately not claimed.
    pub fn validate(&self) -> Result<()> {
        require_derived_data_support()?;
        #[cfg(unix)]
        {
            validate_directory(&self.directory, true)?;
            #[cfg(target_os = "macos")]
            validate_bound_acl(&self.directory, true)?;
            Ok(())
        }
        #[cfg(not(unix))]
        unreachable!("unsupported capability returned above")
    }

    /// Open an existing private regular artifact without create or truncation.
    /// `writable` selects read/write rather than read-only access. Actual opened
    /// ownership, link count, mode and Darwin ACL are checked before return.
    pub fn open_existing(&self, name: &str, writable: bool) -> Result<File> {
        require_derived_data_support()?;
        validate_name(name)?;
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, openat};
            self.validate()?;
            let access = if writable {
                OFlags::RDWR
            } else {
                OFlags::RDONLY
            };
            let file = File::from(
                openat(
                    &self.directory,
                    name,
                    access | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(refused)?,
            );
            validate_file(&file)?;
            #[cfg(target_os = "macos")]
            validate_bound_acl(&file, false)?;
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let _ = writable;
            unreachable!("unsupported capability returned above")
        }
    }
}

fn validate_name(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
        || name.contains(['/', '\\', '\0'])
    {
        return Err(refused("expected a single normal artifact name"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_file(file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(refused(
            "artifact must be current-owner, single-link, private regular file",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_directory(directory: &File, final_component: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = directory.metadata()?;
    let uid = rustix::process::geteuid().as_raw();
    if !metadata.is_dir() || (metadata.uid() != uid && metadata.uid() != 0) {
        return Err(refused(
            "storage ancestors must be directories owned by current user or root",
        ));
    }
    if final_component {
        if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(refused(
                "storage directory must be current-owner with no group/world access",
            ));
        }
    } else if metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0 {
        return Err(refused(
            "non-sticky group/world-writable storage ancestor can replace private children; use a secure user-cache location",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
