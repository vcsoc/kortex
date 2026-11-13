//! Private typed optional policy reads. Absence is classified before CoreError conversion.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PolicyNode {
    File,
    Directory,
    Alias,
    Other,
}

#[derive(Debug)]
pub(super) struct Observation {
    pub node: Option<PolicyNode>,
    // Full current no-follow ancestry, including the leaf when present.
    pub identities: Vec<(i128, u128, u64)>,
    // Windows leases omit SHARE_DELETE and remain held across the actual read.
    #[cfg(windows)]
    _leases: Vec<std::fs::File>,
}
impl PartialEq for Observation {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node && self.identities == other.identities
    }
}
impl Eq for Observation {}

impl ConfinedReader {
    fn policy_observation(
        &mut self,
        relative: &Path,
        required_parents: usize,
        control: &RequestControl,
    ) -> Result<Observation> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(refused("non-normal policy path"));
        }
        self.validate(Some(control))?;
        let before = self
            .backend
            .policy_observe(relative, required_parents, control)?;
        #[cfg(all(test, unix))]
        super::POLICY_OBSERVATION_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        let after = self
            .backend
            .policy_observe(relative, required_parents, control)?;
        self.validate(Some(control))?;
        if before != after {
            return Err(refused("policy path changed during observation"));
        }
        Ok(after)
    }

    /// Read an optional exact relative file, refusing every parent/leaf alias.
    /// Missing optional parents are absence; inaccessible, changed, dangling or
    /// nonregular paths are errors. Actual bytes retain all confined checks.
    pub fn read_optional_exact(
        &mut self,
        relative: &str,
        max: u64,
        control: &RequestControl,
    ) -> Result<Option<Vec<u8>>> {
        self.read_optional_exact_excluding(relative, max, None, control)
    }

    pub(crate) fn read_optional_exact_excluding(
        &mut self,
        relative: &str,
        max: u64,
        excluded_root: Option<(i128, u128)>,
        control: &RequestControl,
    ) -> Result<Option<Vec<u8>>> {
        let relative = Path::new(relative);
        let before = self.policy_observation(relative, 0, control)?;
        if excluded_root.is_some_and(|root| {
            before
                .identities
                .iter()
                .any(|(dev, ino, _)| (*dev, *ino) == root)
        }) {
            return Err(Error::new(
                "POLICY_AUTHORIZATION_REQUIRED",
                "repository namespace cannot supply owner policy selectors",
                8,
            ));
        }
        match before.node {
            None => return Ok(None),
            Some(PolicyNode::File) => {}
            Some(_) => {
                return Err(refused(
                    "exact file authority does not authorize aliases or nonregular files",
                ));
            }
        }
        let bytes = self.read_resolved(&self.root.join(relative), max, Some(control))?;
        if before != self.policy_observation(relative, 0, control)? {
            return Err(refused("exact file changed during read"));
        }
        Ok(Some(bytes))
    }

    pub(crate) fn policy_node(
        &mut self,
        relative: &Path,
        required_parents: usize,
        control: &RequestControl,
    ) -> Result<Option<PolicyNode>> {
        Ok(self
            .policy_observation(relative, required_parents, control)?
            .node)
    }

    pub(crate) fn policy_bytes(
        &mut self,
        relative: &Path,
        required_parents: usize,
        max: u64,
        control: &RequestControl,
    ) -> Result<Option<Vec<u8>>> {
        let before = self.policy_observation(relative, required_parents, control)?;
        let Some(node) = before.node else {
            return Ok(None);
        };
        if !matches!(node, PolicyNode::File | PolicyNode::Alias) {
            return Err(refused("policy is not a regular file"));
        }
        // Resolve aliases as metadata only. The bytes are ALWAYS read through
        // the retained root's no-follow descriptor reader, never ambient reopen.
        let candidate = if node == PolicyNode::Alias {
            self.stats.canonicalizations += 1;
            let target = self
                .root
                .join(relative)
                .canonicalize()
                .map_err(|_| refused("policy alias is dangling, cyclic or inaccessible"))?;
            if !target.starts_with(&self.root) {
                return Err(refused("policy alias escapes repository"));
            }
            target
        } else {
            self.root.join(relative)
        };
        let bytes = self.read_resolved(&candidate, max, Some(control))?;
        if node == PolicyNode::Alias
            && self
                .root
                .join(relative)
                .canonicalize()
                .map_err(|_| refused("policy alias became inaccessible"))?
                != candidate
        {
            return Err(refused("policy alias changed during read"));
        }
        let after = self.policy_observation(relative, required_parents, control)?;
        if before != after {
            return Err(refused("policy path changed during read"));
        }
        Ok(Some(bytes))
    }
}

#[cfg(unix)]
pub(super) fn observe_unix(
    root: &std::fs::File,
    relative: &Path,
    required_parents: usize,
    control: &RequestControl,
) -> Result<Observation> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};
    let names: Vec<_> = relative.components().map(|c| c.as_os_str()).collect();
    let mut directory: Option<std::fs::File> = None;
    let mut identities = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        control.check()?;
        let above = directory.as_ref().unwrap_or(root);
        let stat = match statat(above, *name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) if index >= required_parents => {
                return Ok(Observation {
                    node: None,
                    identities,
                });
            }
            Err(error) => {
                return Err(refused(&format!(
                    "policy path inaccessible or stale: {error}"
                )));
            }
        };
        identities.push((stat.st_dev.into(), stat.st_ino.into(), stat.st_mode.into()));
        let kind = FileType::from_raw_mode(stat.st_mode);
        if index == names.len() - 1 {
            let node = if kind.is_file() {
                PolicyNode::File
            } else if kind.is_dir() {
                PolicyNode::Directory
            } else if kind.is_symlink() {
                PolicyNode::Alias
            } else {
                PolicyNode::Other
            };
            return Ok(Observation {
                node: Some(node),
                identities,
            });
        }
        let file = std::fs::File::from(
            openat(
                above,
                *name,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| refused(&format!("policy parent inaccessible or changed: {e}")))?,
        );
        let opened =
            fstat(&file).map_err(|e| refused(&format!("policy parent stat failed: {e}")))?;
        if (
            opened.st_dev.into(),
            opened.st_ino.into(),
            opened.st_mode.into(),
        ) != *identities.last().unwrap()
        {
            return Err(refused("policy parent changed during open"));
        }
        directory = Some(file);
    }
    Err(refused("expected policy path"))
}

#[cfg(windows)]
pub(super) fn observe_windows(
    root: &cap_std::fs::Dir,
    relative: &Path,
    required_parents: usize,
    control: &RequestControl,
) -> Result<Observation> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let names: Vec<_> = relative.components().map(|c| c.as_os_str()).collect();
    let mut directories: Vec<Dir> = Vec::new();
    let mut identities = Vec::with_capacity(names.len());
    let mut leaf = None;
    let mut node = None;
    for (index, name) in names.iter().enumerate() {
        control.check()?;
        if name
            .to_str()
            .is_none_or(|n| n.contains(':') || n.ends_with(['.', ' ']))
        {
            return Err(refused("non-normal Windows policy path"));
        }
        let above = directories.last().unwrap_or(root);
        let mut options = OpenOptions::new();
        options
            .read(true)
            .follow(FollowSymlinks::No)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let file = match above.open_with(name, &options) {
            Ok(file) => file.into_std(),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index >= required_parents =>
            {
                break;
            }
            Err(_) => return Err(refused("Windows policy path inaccessible or stale")),
        };
        let metadata = file.metadata()?;
        // Supplemental role identity, NOT a substitute for no-SHARE_DELETE leases.
        let identity = cap_std::fs::Metadata::from_file(&file)?;
        use cap_fs_ext::MetadataExt as CapabilityMetadataExt;
        identities.push((identity.dev().into(), identity.ino().into(), 0));
        let reparse = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if index == names.len() - 1 {
            node = Some(if reparse {
                PolicyNode::Alias
            } else if metadata.is_dir() {
                PolicyNode::Directory
            } else if metadata.is_file() {
                PolicyNode::File
            } else {
                PolicyNode::Other
            });
            leaf = Some(file);
            break;
        }
        if reparse || !metadata.is_dir() {
            return Err(refused("Windows policy parent is not a real directory"));
        }
        directories.push(Dir::from_std_file(file));
    }
    // These handles bind every existing spelling, including absent-leaf parents.
    // No identifier/mtime heuristic substitutes for the no-SHARE_DELETE leases.
    let mut leases: Vec<_> = directories.into_iter().map(Dir::into_std_file).collect();
    leases.extend(leaf);
    Ok(Observation {
        node,
        identities,
        _leases: leases,
    })
}
