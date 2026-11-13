//! Repository-controlled configuration must be confined before parsing any bytes.
use crate::{Error, Result, policy::read_opened_config};
#[cfg(unix)]
use std::fs::File;
use std::path::Path;

fn refused() -> Error {
    Error::new(
        "SECURITY_POLICY",
        "repository configuration must be a stable regular file, not a symlink; use explicit --config for trusted aliases",
        8,
    )
}

#[cfg(unix)]
pub(super) fn read(root: &Path) -> Result<Option<String>> {
    use rustix::fs::{Mode, OFlags, open, openat};
    use std::os::unix::fs::MetadataExt;
    let directory = File::from(
        open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| refused())?,
    );
    let anchor = directory.metadata()?;
    let check_root = || -> Result<()> {
        let current = std::fs::symlink_metadata(root).map_err(|_| refused())?;
        if !current.is_dir() || current.dev() != anchor.dev() || current.ino() != anchor.ino() {
            return Err(refused());
        }
        Ok(())
    };
    check_root()?;
    let open_leaf = || {
        openat(
            &directory,
            ".kortex.toml",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
    };
    let file = match open_leaf() {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => {
            check_root()?;
            return Ok(None);
        }
        Err(rustix::io::Errno::LOOP) => return Err(refused()),
        Err(error) => return Err(std::io::Error::from(error).into()),
    };
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(refused());
    }
    let source = read_opened_config(file)?;
    #[cfg(test)]
    after_read();
    let after = open_leaf().map_err(|_| refused())?.metadata()?;
    if !after.is_file() || after.dev() != before.dev() || after.ino() != before.ino() {
        return Err(refused());
    }
    check_root()?;
    Ok(Some(source))
}

#[cfg(windows)]
pub(super) fn read(root: &Path) -> Result<Option<String>> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let anchor = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)?;
    let metadata = anchor.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refused());
    }
    let directory = cap_std::fs::Dir::from_std_file(anchor);
    match directory.symlink_metadata(".kortex.toml") {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(".kortex.toml", &options)
        .map_err(|_| refused())?
        .into_std();
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refused());
    }
    Ok(Some(read_opened_config(file)?))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn read(_root: &Path) -> Result<Option<String>> {
    Err(refused())
}

#[cfg(all(test, unix))]
thread_local! {
    static AFTER_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(all(test, unix))]
fn after_read() {
    AFTER_READ.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn outside_and_inside_aliases_refuse_before_parsing_dummy_contents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        let marker = "OUTSIDE_CONFIG_DUMMY_MARKER";
        let outside = temp.path().join("outside.toml");
        fs::write(&outside, format!("{marker} = [\"unterminated\n")).unwrap();
        let inside = root.join("alias-target.toml");
        fs::write(&inside, fs::read(&outside).unwrap()).unwrap();
        for target in [&outside, &inside] {
            symlink(target, root.join(".kortex.toml")).unwrap();
            let error = crate::Config::load(&root, None).unwrap_err();
            assert_eq!(error.code, "SECURITY_POLICY");
            assert!(!error.message.contains(marker));
            fs::remove_file(root.join(".kortex.toml")).unwrap();
        }
    }

    #[test]
    fn absent_regular_and_explicit_trusted_alias_behaviors_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        assert!(read(&root).unwrap().is_none());
        let source = "[repository]\nmax_files = 123\n";
        fs::write(root.join(".kortex.toml"), source).unwrap();
        assert_eq!(read(&root).unwrap().as_deref(), Some(source));
        assert_eq!(
            crate::Config::load(&root, None)
                .unwrap()
                .repository
                .max_files,
            123
        );
        fs::remove_file(root.join(".kortex.toml")).unwrap();
        let outside = temp.path().join("trusted.toml");
        fs::write(&outside, source).unwrap();
        symlink(&outside, root.join(".kortex.toml")).unwrap();
        let config = crate::Config::load(&root, Some(&root.join(".kortex.toml"))).unwrap();
        assert_eq!(config.repository.max_files, 123);
    }

    #[test]
    fn post_read_leaf_replacement_and_root_substitution_abort_without_contents() {
        for replace_root in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("repo");
            fs::create_dir(&root).unwrap();
            fs::write(root.join(".kortex.toml"), "schema_version = 1\n").unwrap();
            let outside = temp.path().join("outside.toml");
            fs::write(&outside, "OUTSIDE_POST_READ_DUMMY = [\"unterminated\n").unwrap();
            let changed = root.clone();
            AFTER_READ.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    if replace_root {
                        fs::rename(&changed, changed.with_file_name("moved-repo")).unwrap();
                        fs::create_dir(&changed).unwrap();
                    } else {
                        fs::remove_file(changed.join(".kortex.toml")).unwrap();
                        symlink(&outside, changed.join(".kortex.toml")).unwrap();
                    }
                }));
            });
            let error = read(&root).unwrap_err();
            assert_eq!(error.code, "SECURITY_POLICY");
            assert!(!error.message.contains("OUTSIDE_POST_READ_DUMMY"));
        }
    }

    #[test]
    fn repository_fifo_and_directory_refuse_without_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join(".kortex.toml")).unwrap();
        assert_eq!(read(root).unwrap_err().code, "SECURITY_POLICY");
        fs::remove_dir(root.join(".kortex.toml")).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join(".kortex.toml"))
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(read(root).unwrap_err().code, "SECURITY_POLICY");
    }
}
