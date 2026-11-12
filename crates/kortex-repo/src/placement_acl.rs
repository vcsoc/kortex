//! Darwin path ACL inspection with a root-down name-stability precondition.
//!
//! exacl exposes no descriptor ACL reader. Never inspect an arbitrary descendant
//! path: first establish trusted ownership/mode and deny-only ACLs for EVERY
//! preceding directory. Sticky ancestors protect trusted-owned child names (see
//! Darwin unlink(2), EPERM); foreign-owned children fail before ACL inspection.
//! Current/root UID changes and mount-administrator changes are trusted, not races
//! this policy claims to prevent. F_GETPATH alone is NOT a security proof.
use super::*;
use std::{
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::PathBuf,
};

pub(super) fn validate_acl_at(file: &File, path: &Path) -> Result<()> {
    let before = file.metadata()?;
    let named = path.symlink_metadata().map_err(refused)?;
    if before.dev() != named.dev() || before.ino() != named.ino() || named.file_type().is_symlink()
    {
        return Err(refused("ACL path no longer names the opened object"));
    }
    let entries = exacl::getfacl(path, exacl::AclOption::SYMLINK_ACL).map_err(refused)?;
    // Even inherit-only or current-owner ALLOWs are conservatively refused:
    // no principal/group resolution or ACL ordering is trusted as an exclusion.
    if entries
        .iter()
        .any(|entry| entry.allow || entry.kind == exacl::AclEntryKind::Unknown)
    {
        return Err(refused(
            "extended ALLOW or unknown ACL entry is unsupported for private storage",
        ));
    }
    let after = path.symlink_metadata().map_err(refused)?;
    if after.dev() != before.dev() || after.ino() != before.ino() || after.file_type().is_symlink()
    {
        return Err(refused("ACL object changed during inspection"));
    }
    Ok(())
}

pub(super) fn validate_bound_acl(file: &File, directory_leaf: bool) -> Result<()> {
    use rustix::fs::{Mode, OFlags, open, openat};
    // Recover the current name even after an accidental trusted-user rename,
    // then prove the complete ancestor chain before querying any child ACL.
    let name = rustix::fs::getpath(file).map_err(refused)?;
    let path = Path::new(std::ffi::OsStr::from_bytes(name.as_bytes()));
    if !path.is_absolute()
        || path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err(refused("descriptor has no normal absolute ACL path"));
    }
    let mut parts = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n),
            _ => None,
        })
        .peekable();
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let mut opened =
        File::from(open("/", flags | OFlags::DIRECTORY, Mode::empty()).map_err(refused)?);
    let mut checked = PathBuf::from("/");
    validate_directory(&opened, directory_leaf && parts.peek().is_none())?;
    validate_acl_at(&opened, &checked)?;
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        let extra = if !last || directory_leaf {
            OFlags::DIRECTORY
        } else {
            OFlags::empty()
        };
        opened = File::from(openat(&opened, part, flags | extra, Mode::empty()).map_err(refused)?);
        if !last || directory_leaf {
            validate_directory(&opened, last)?;
        } else {
            validate_file(&opened)?;
        }
        checked.push(part);
        validate_acl_at(&opened, &checked)?;
    }
    let expected = file.metadata()?;
    let actual = opened.metadata()?;
    if expected.dev() != actual.dev() || expected.ino() != actual.ino() {
        return Err(refused(
            "revalidated ACL chain does not name held descriptor",
        ));
    }
    Ok(())
}
