//! Request-local source capability. Cached handles are hints, never path authority.
use kortex_core::{Error, RequestControl, Result};
use std::path::{Path, PathBuf};
#[path = "policy_read.rs"]
mod policy_read;
pub(crate) use policy_read::PolicyNode;

#[cfg(all(test, unix))]
type BeforeOpenHook = Option<Box<dyn FnOnce()>>;
#[cfg(all(test, unix))]
std::thread_local! {
    pub(crate) static BEFORE_OPEN_HOOK: std::cell::RefCell<BeforeOpenHook> = std::cell::RefCell::new(None);
    pub(crate) static POLICY_OBSERVATION_HOOK: std::cell::RefCell<BeforeOpenHook> = std::cell::RefCell::new(None);
}

/// Byte-read operations, including failed opens and EOF reads. The additive
/// policy API's metadata-only probes are not counted as parent/file operations;
/// their root validations and all actual policy byte reads ARE counted.
/// `read_calls` counts `Read::read` calls (including `Take`'s limit EOF), not
/// kernel syscalls. Capability-internal Windows traversal is not counted as
/// parent opens/checks. Discovery's external canonicalization is not counted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConfinedReadStats {
    pub root_opens: u64,
    pub root_checks: u64,
    pub canonicalizations: u64,
    pub parent_opens: u64,
    pub parent_checks: u64,
    pub file_opens: u64,
    pub leaf_checks: u64,
    pub read_calls: u64,
    pub bytes_read: u64,
}

/// Reuses a canonical source root for a discovery/request, not source bytes or hashes.
/// Initial acquisition/ancestors are trusted. This is not a filesystem snapshot or
/// the private derived-data policy implemented by `placement::DirectoryAnchor`.
pub struct ConfinedReader {
    root: PathBuf,
    backend: platform::Root,
    // Reuse capacity only; stale contents never substitute for actual returned bytes.
    scratch: Box<crate::confined::ReadScratch>,
    stats: ConfinedReadStats,
}

impl ConfinedReader {
    pub fn new(root: &Path) -> Result<Self> {
        if root.canonicalize()? != root {
            return Err(refused("source root must already be canonical"));
        }
        Ok(Self {
            root: root.to_owned(),
            backend: platform::Root::new(root)?,
            scratch: Box::new([0; crate::confined::READ_CHUNK_BYTES]),
            stats: ConfinedReadStats {
                root_opens: 1,
                canonicalizations: 1,
                ..Default::default()
            },
        })
    }

    pub fn stats(&self) -> &ConfinedReadStats {
        &self.stats
    }

    pub(crate) fn policy_root_identity(
        &mut self,
        control: &RequestControl,
    ) -> Result<(i128, u128)> {
        self.validate(Some(control))?;
        self.backend.policy_identity()
    }

    pub(crate) fn validate(&mut self, control: Option<&RequestControl>) -> Result<()> {
        crate::checkpoint(control)?;
        self.backend.check_root(&self.root, &mut self.stats)
    }

    pub fn read_bytes(
        &mut self,
        relative: &str,
        max: u64,
        control: &RequestControl,
    ) -> Result<Vec<u8>> {
        self.read_impl(relative, max, Some(control))
    }

    pub fn read(&mut self, relative: &str, max: u64, control: &RequestControl) -> Result<String> {
        text(self.read_impl(relative, max, Some(control))?)
    }

    pub(crate) fn read_impl(
        &mut self,
        relative: &str,
        max: u64,
        control: Option<&RequestControl>,
    ) -> Result<Vec<u8>> {
        crate::checkpoint(control)?;
        self.stats.canonicalizations += 1;
        let canonical = crate::resolve_confined(&self.root, relative)?;
        self.read_resolved(&canonical, max, control)
    }

    // Discovery supplies a resolved alias target or a lexical no-link walk
    // candidate. Neither bypasses full descriptor-relative component validation.
    pub(crate) fn read_resolved(
        &mut self,
        canonical: &Path,
        max: u64,
        control: Option<&RequestControl>,
    ) -> Result<Vec<u8>> {
        crate::checkpoint(control)?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| refused("path escaped root"))?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(refused("non-normal confined path"));
        }
        self.backend.read(
            &self.root,
            relative,
            max,
            control,
            &mut self.stats,
            &mut self.scratch,
        )
    }
}

pub(crate) fn text(bytes: Vec<u8>) -> Result<String> {
    if bytes.contains(&0) {
        return Err(Error::new("BINARY_FILE", "binary file", 7));
    }
    String::from_utf8(bytes).map_err(|_| Error::new("INVALID_ENCODING", "not UTF-8", 7))
}

// Metadata-only role checking on explicitly granted canonical scope ancestry.
// Never supplies an opener/anchor for actual file bytes.
pub(crate) fn policy_ancestor_identity(
    path: &Path,
    control: &RequestControl,
) -> Result<(i128, u128)> {
    control.check()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(refused("operator scope ancestry is not a real directory"));
        }
        Ok((metadata.dev().into(), metadata.ino().into()))
    }
    #[cfg(windows)]
    {
        use cap_fs_ext::MetadataExt;
        let directory = crate::confined::windows_root(path)?;
        let metadata = directory.dir_metadata()?;
        Ok((metadata.dev().into(), metadata.ino().into()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(refused("operator ancestry identity unsupported"))
    }
}

fn refused(message: &str) -> Error {
    Error::new("PATH_REFUSED", message, 8)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fs::{AtFlags, Mode, OFlags, Stat, fstat, open, openat, statat};
    use std::{ffi::OsString, fs::File, os::unix::fs::MetadataExt};
    const MAX_PARENTS: usize = 32;

    struct Parent {
        name: OsString,
        file: File,
        identity: Stat,
    }
    pub(super) struct Root {
        file: File,
        identity: std::fs::Metadata,
        parents: Vec<Parent>,
    }

    fn same(a: &Stat, b: &Stat) -> bool {
        a.st_dev == b.st_dev && a.st_ino == b.st_ino && a.st_mode == b.st_mode
    }
    fn io(error: rustix::io::Errno) -> Error {
        refused(&format!(
            "confined path changed or became inaccessible: {error}"
        ))
    }

    impl Root {
        pub(super) fn policy_identity(&self) -> Result<(i128, u128)> {
            Ok((self.identity.dev().into(), self.identity.ino().into()))
        }

        pub(super) fn policy_observe(
            &self,
            relative: &Path,
            required_parents: usize,
            control: &RequestControl,
        ) -> Result<super::policy_read::Observation> {
            super::policy_read::observe_unix(&self.file, relative, required_parents, control)
        }

        pub(super) fn new(root: &Path) -> Result<Self> {
            let file = File::from(
                open(
                    root,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(io)?,
            );
            let identity = file.metadata()?;
            Ok(Self {
                file,
                identity,
                parents: Vec::new(),
            })
        }

        pub(super) fn check_root(&self, root: &Path, stats: &mut ConfinedReadStats) -> Result<()> {
            stats.root_checks += 1;
            let current = std::fs::symlink_metadata(root)?;
            if !current.is_dir()
                || current.dev() != self.identity.dev()
                || current.ino() != self.identity.ino()
            {
                return Err(refused("source root identity changed"));
            }
            Ok(())
        }

        fn check_parents(
            &self,
            stats: &mut ConfinedReadStats,
            control: Option<&RequestControl>,
        ) -> Result<()> {
            for (index, parent) in self.parents.iter().enumerate() {
                crate::checkpoint(control)?;
                let above = if index == 0 {
                    &self.file
                } else {
                    &self.parents[index - 1].file
                };
                stats.parent_checks += 1;
                let current = statat(above, &parent.name, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
                if !same(&current, &parent.identity) {
                    return Err(refused(
                        "cached parent no longer names the current directory",
                    ));
                }
            }
            Ok(())
        }

        fn walk_deep(
            &self,
            relative: &Path,
            control: Option<&RequestControl>,
            stats: &mut ConfinedReadStats,
        ) -> Result<(File, Vec<Stat>)> {
            let mut directory: Option<File> = None;
            let mut identities = Vec::new();
            let mut names = relative.components().peekable();
            while let Some(name) = names.next() {
                crate::checkpoint(control)?;
                let parent = names.peek().is_some();
                let mut flags =
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
                if parent {
                    flags |= OFlags::DIRECTORY;
                    stats.parent_opens += 1;
                } else {
                    stats.file_opens += 1;
                }
                let file = File::from(
                    openat(
                        directory.as_ref().unwrap_or(&self.file),
                        name.as_os_str(),
                        flags,
                        Mode::empty(),
                    )
                    .map_err(io)?,
                );
                identities.push(fstat(&file).map_err(io)?);
                if !parent {
                    return Ok((file, identities));
                }
                directory = Some(file);
            }
            Err(refused("expected source file"))
        }

        pub(super) fn read(
            &mut self,
            root: &Path,
            relative: &Path,
            max: u64,
            control: Option<&RequestControl>,
            stats: &mut ConfinedReadStats,
            scratch: &mut crate::confined::ReadScratch,
        ) -> Result<Vec<u8>> {
            self.check_root(root, stats)?;
            let names: Vec<_> = relative.components().map(|c| c.as_os_str()).collect();
            if names.len() - 1 > MAX_PARENTS {
                // Deep paths retain metadata only; never grow a session FD cache
                // with attacker-controlled depth or silently skip path validation.
                let (file, before) = self.walk_deep(relative, control, stats)?;
                if !file.metadata()?.is_file() {
                    return Err(refused("not a regular file"));
                }
                let bytes = crate::confined::read_opened_bytes_with_scratch(
                    file, max, control, stats, scratch,
                )?;
                crate::checkpoint(control)?;
                let (_after_file, after) = self.walk_deep(relative, control, stats)?;
                if before.iter().zip(&after).any(|(a, b)| !same(a, b)) {
                    return Err(refused("deep source path changed during read"));
                }
                self.check_root(root, stats)?;
                return Ok(bytes);
            }
            let common = self
                .parents
                .iter()
                .zip(&names)
                .take_while(|(p, n)| p.name == **n)
                .count()
                .min(names.len() - 1);
            self.parents.truncate(common);
            if let Err(error) = self.check_parents(stats, control) {
                self.parents.clear();
                return Err(error);
            }
            for name in &names[common..names.len() - 1] {
                crate::checkpoint(control)?;
                let above = self.parents.last().map_or(&self.file, |p| &p.file);
                stats.parent_opens += 1;
                let file = File::from(
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
                    .map_err(io)?,
                );
                let identity = fstat(&file).map_err(io)?;
                self.parents.push(Parent {
                    name: (*name).to_owned(),
                    file,
                    identity,
                });
            }
            #[cfg(test)]
            super::BEFORE_OPEN_HOOK.with(|hook| {
                if let Some(hook) = hook.borrow_mut().take() {
                    hook();
                }
            });
            crate::checkpoint(control)?;
            let above = self.parents.last().map_or(&self.file, |p| &p.file);
            stats.file_opens += 1;
            let file = File::from(
                openat(
                    above,
                    names[names.len() - 1],
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(io)?,
            );
            let identity = fstat(&file).map_err(io)?;
            if !rustix::fs::FileType::from_raw_mode(identity.st_mode).is_file() {
                return Err(refused("not a regular file"));
            }
            let bytes = crate::confined::read_opened_bytes_with_scratch(
                file, max, control, stats, scratch,
            )?;
            crate::checkpoint(control)?;
            self.check_parents(stats, control)?;
            let above = self.parents.last().map_or(&self.file, |p| &p.file);
            stats.leaf_checks += 1;
            let after =
                statat(above, names[names.len() - 1], AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
            // Compare with the descriptor actually read, not a prior pathname stat.
            if !same(&after, &identity) {
                return Err(refused("source leaf identity changed during read"));
            }
            self.check_root(root, stats)?;
            Ok(bytes)
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    pub(super) struct Root(cap_std::fs::Dir);
    impl Root {
        pub(super) fn policy_identity(&self) -> Result<(i128, u128)> {
            use cap_fs_ext::MetadataExt;
            let metadata = self.0.dir_metadata()?;
            Ok((metadata.dev().into(), metadata.ino().into()))
        }

        pub(super) fn policy_observe(
            &self,
            relative: &Path,
            required_parents: usize,
            control: &RequestControl,
        ) -> Result<super::policy_read::Observation> {
            super::policy_read::observe_windows(&self.0, relative, required_parents, control)
        }
        pub(super) fn new(root: &Path) -> Result<Self> {
            Ok(Self(crate::confined::windows_root(root)?))
        }
        pub(super) fn check_root(&self, _: &Path, _: &mut ConfinedReadStats) -> Result<()> {
            // The retained Windows capability omits SHARE_DELETE, preventing
            // root rename/replacement for its lifetime. No path stat is issued.
            Ok(())
        }
        pub(super) fn read(
            &mut self,
            _root: &Path,
            relative: &Path,
            max: u64,
            control: Option<&RequestControl>,
            stats: &mut ConfinedReadStats,
            scratch: &mut crate::confined::ReadScratch,
        ) -> Result<Vec<u8>> {
            crate::checkpoint(control)?;
            stats.file_opens += 1;
            let file = crate::confined::windows_open(&self.0, relative)?;
            crate::confined::read_opened_bytes_with_scratch(file, max, control, stats, scratch)
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;
    pub(super) struct Root;
    impl Root {
        pub(super) fn policy_identity(&self) -> Result<(i128, u128)> {
            Err(refused("policy directory identity unsupported"))
        }

        pub(super) fn policy_observe(
            &self,
            _relative: &Path,
            _required_parents: usize,
            _control: &RequestControl,
        ) -> Result<super::policy_read::Observation> {
            Err(refused("confined policy observation unsupported"))
        }
        pub(super) fn new(_: &Path) -> Result<Self> {
            Err(refused("confined reads unsupported"))
        }
        pub(super) fn check_root(&self, _: &Path, _: &mut ConfinedReadStats) -> Result<()> {
            Err(refused("confined reads unsupported"))
        }
        pub(super) fn read(
            &mut self,
            _: &Path,
            _: &Path,
            _: u64,
            _: Option<&RequestControl>,
            _: &mut ConfinedReadStats,
            _: &mut crate::confined::ReadScratch,
        ) -> Result<Vec<u8>> {
            Err(refused("confined reads unsupported"))
        }
    }
}
