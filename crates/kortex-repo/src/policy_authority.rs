//! Explicit operator-granted policy files, separate from repository discovery.
//!
//! These types have no serde/config implementation. A trusted caller grants a
//! directory at initial acquisition, then selects exact relative files within it.
//! Repository policy bytes never construct scopes or authorize alias targets.
mod global;
pub use global::{OperatorGlobalAcquisition, OperatorGlobalPlan};

use crate::{ConfinedReadStats, ConfinedReader};
use kortex_core::{Error, RequestControl, Result};
use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_PATH_BYTES: usize = 32_768;
const MAX_DEPTH: usize = 128;
const MAX_POLICY_BYTES: u64 = 1024 * 1024;

fn refused(message: &str) -> Error {
    Error::new("POLICY_AUTHORIZATION_REQUIRED", message, 8)
}

/// A directory explicitly authorized by the operator/trusted SDK caller.
/// This is NOT created implicitly from a selected repository or its config.
/// Parent acquisition is trusted; every actual read retains confined observations.
pub struct OperatorPolicyScope {
    root: PathBuf,
    reader: Mutex<ConfinedReader>,
}

impl OperatorPolicyScope {
    /// The supplied scope must already be canonical. No ambient fallback or
    /// automatic ancestor/home/Git-directed scope expansion is performed.
    pub fn new(canonical_directory: &Path) -> Result<Arc<Self>> {
        if !canonical_directory.is_absolute()
            || canonical_directory.to_str().is_none()
            || canonical_directory.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
            || canonical_directory.components().count() > MAX_DEPTH
            || canonical_directory.components().any(|part| {
                !matches!(
                    part,
                    Component::RootDir | Component::Prefix(_) | Component::Normal(_)
                )
            })
        {
            return Err(refused(
                "operator policy scope must be a bounded canonical directory",
            ));
        }
        Ok(Arc::new(Self {
            root: canonical_directory.to_owned(),
            reader: Mutex::new(ConfinedReader::new(canonical_directory)?),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn reader(&self) -> Result<MutexGuard<'_, ConfinedReader>> {
        self.reader
            .lock()
            .map_err(|_| refused("operator policy reader poisoned"))
    }

    /// Construct an exact file capability, not a wildcard or alias grant.
    /// Selecting a path does not read it or establish presence/absence.
    pub fn file(self: &Arc<Self>, relative: &Path) -> Result<OperatorPolicyFile> {
        if relative.as_os_str().is_empty()
            || relative.to_str().is_none()
            || relative.as_os_str().as_encoded_bytes().contains(&0)
            || relative.components().count() > MAX_DEPTH
            || self
                .root
                .as_os_str()
                .as_encoded_bytes()
                .len()
                .saturating_add(relative.as_os_str().as_encoded_bytes().len())
                > MAX_PATH_BYTES
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(refused(
                "operator policy file requires a bounded normal relative path",
            ));
        }
        Ok(OperatorPolicyFile {
            scope: Arc::clone(self),
            relative: relative.to_owned(),
        })
    }

    pub fn stats(&self) -> Result<ConfinedReadStats> {
        Ok(self.reader()?.stats().clone())
    }

    /// Additional validation, never a replacement for reading effective inputs.
    pub fn validate(&self, control: &RequestControl) -> Result<()> {
        self.reader()?.validate(Some(control))
    }
}

/// Exact path within a separately granted scope. Cloning retains that SAME anchor.
#[derive(Clone)]
pub struct OperatorPolicyFile {
    scope: Arc<OperatorPolicyScope>,
    relative: PathBuf,
}

/// Absence is explicit; neither failure nor an unreadable input becomes absence.
/// Raw bytes are for the trusted caller, not diagnostic logging or model output.
pub struct AcquiredPolicyFile {
    pub origin: PathBuf,
    pub bytes: Option<Vec<u8>>,
    pub digest: Option<String>,
}

impl OperatorPolicyFile {
    pub fn origin(&self) -> PathBuf {
        self.scope.root.join(&self.relative)
    }

    /// Reads and hashes actual bytes on EVERY call; no contents/mtime cache.
    /// All parent and leaf aliases refuse, including aliases within the scope.
    /// Missing optional directories/leaf are typed absence after root checks.
    pub fn acquire(
        &self,
        remaining_bytes: u64,
        control: &RequestControl,
    ) -> Result<AcquiredPolicyFile> {
        self.acquire_inner(remaining_bytes, None, control)
    }

    fn acquire_selector(
        &self,
        remaining_bytes: u64,
        excluded_root: (i128, u128),
        control: &RequestControl,
    ) -> Result<AcquiredPolicyFile> {
        self.acquire_inner(remaining_bytes, Some(excluded_root), control)
    }

    fn acquire_inner(
        &self,
        remaining_bytes: u64,
        excluded_root: Option<(i128, u128)>,
        control: &RequestControl,
    ) -> Result<AcquiredPolicyFile> {
        control.check()?;
        let mut reader = self.scope.reader()?;
        if let Some(excluded) = excluded_root {
            reader.validate(Some(control))?;
            // A scope may itself be inside the source tree. Compare actual
            // directory identities, not filesystem-specific case/Unicode folds.
            for ancestor in self.scope.root.ancestors() {
                if crate::reader::policy_ancestor_identity(ancestor, control)? == excluded {
                    return Err(refused(
                        "repository namespace cannot supply owner policy selectors",
                    ));
                }
            }
            reader.validate(Some(control))?;
        }
        let bytes = reader
            .read_optional_exact_excluding(
                self.relative
                    .to_str()
                    .ok_or_else(|| refused("policy file is not UTF-8"))?,
                remaining_bytes.min(MAX_POLICY_BYTES),
                excluded_root,
                control,
            )
            .map_err(|error| {
                if error.code == "FILE_TOO_LARGE" {
                    Error::new(
                        "RESOURCE_LIMIT",
                        "operator policy input exceeds its remaining byte admission",
                        9,
                    )
                } else {
                    error
                }
            })?;
        let digest = bytes
            .as_deref()
            .map(|bytes| crate::hash_controlled(bytes, Some(control)))
            .transpose()?;
        reader.validate(Some(control))?;
        Ok(AcquiredPolicyFile {
            origin: self.origin(),
            bytes,
            digest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};
    fn control() -> RequestControl {
        RequestControl::new(Duration::from_secs(30)).unwrap()
    }

    #[test]
    fn exact_authority_has_typed_absence_real_reads_hashes_and_downward_admission() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let scope = OperatorPolicyScope::new(&root).unwrap();
        let input = scope.file(Path::new("nested/policy")).unwrap();
        let absent = input.acquire(1024, &control()).unwrap();
        assert!(absent.bytes.is_none() && absent.digest.is_none());
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/policy"), b"one.py\n").unwrap();
        let first = input.acquire(7, &control()).unwrap();
        assert_eq!(first.bytes.as_deref(), Some(b"one.py\n".as_slice()));
        assert_eq!(first.digest, Some(kortex_core::hash(b"one.py\n")));
        fs::write(root.join("nested/policy"), b"two.py\n").unwrap();
        let second = input.acquire(7, &control()).unwrap();
        assert_ne!(first.digest, second.digest);
        assert_eq!(second.bytes.as_deref(), Some(b"two.py\n".as_slice()));
        assert_eq!(
            input.acquire(6, &control()).err().unwrap().code,
            "RESOURCE_LIMIT"
        );
        assert_eq!(input.acquire(7, &control()).unwrap().digest, second.digest);
        fs::write(root.join("nested/policy"), []).unwrap();
        assert_eq!(
            input.acquire(0, &control()).unwrap().bytes,
            Some(Vec::new())
        );
    }

    #[test]
    fn invalid_selection_does_not_read_and_cancel_is_not_absence() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let scope = OperatorPolicyScope::new(&root).unwrap();
        for path in ["", ".", "..", "../outside", "/absolute", "bad\0path"] {
            assert!(scope.file(Path::new(path)).is_err());
        }
        assert_eq!(scope.stats().unwrap().bytes_read, 0);
        let input = scope.file(Path::new("missing")).unwrap();
        let control = control();
        control.cancel();
        assert_eq!(
            input.acquire(1024, &control).err().unwrap().code,
            "CANCELLED"
        );
    }

    #[cfg(unix)]
    #[test]
    fn all_aliases_and_nonregular_files_refuse_without_reading_target_bytes() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scope");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        fs::write(temp.path().join("outside"), b"DUMMY_OUTSIDE_POLICY_TARGET").unwrap();
        fs::write(root.join("inside"), b"DUMMY_INSIDE_POLICY_TARGET").unwrap();
        symlink(temp.path().join("outside"), root.join("external")).unwrap();
        symlink("inside", root.join("internal")).unwrap();
        symlink("nonexistent", root.join("dangling")).unwrap();
        symlink(temp.path(), root.join("parent-alias")).unwrap();
        fs::create_dir(root.join("directory")).unwrap();
        let scope = OperatorPolicyScope::new(&root).unwrap();
        for path in [
            "external",
            "internal",
            "dangling",
            "parent-alias/outside",
            "directory",
        ] {
            let input = scope.file(Path::new(path)).unwrap();
            assert!(input.acquire(1024, &control()).is_err());
            assert_eq!(scope.stats().unwrap().bytes_read, 0);
            assert_eq!(scope.stats().unwrap().file_opens, 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn replacement_after_observation_refuses_before_alias_target_read() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scope");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        fs::write(root.join("policy"), b"initial\n").unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"DUMMY_REPLACEMENT_POLICY").unwrap();
        let scope = OperatorPolicyScope::new(&root).unwrap();
        let input = scope.file(Path::new("policy")).unwrap();
        let leaf = root.join("policy");
        crate::reader::BEFORE_OPEN_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(&leaf).unwrap();
                symlink(outside, leaf).unwrap();
            }))
        });
        assert!(input.acquire(1024, &control()).is_err());
        assert_eq!(scope.stats().unwrap().bytes_read, 0);
    }
}
