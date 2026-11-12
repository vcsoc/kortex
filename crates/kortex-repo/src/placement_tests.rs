use super::*;
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

#[cfg(unix)]
fn mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
}

#[cfg(unix)]
#[test]
fn creates_restricted_directory_and_file_before_any_derived_writes() {
    use std::io::Write;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let path = root.join("new/cache/.kortex");
    let anchor = DirectoryAnchor::acquire(&path, true).unwrap();
    for path in [root.join("new"), root.join("new/cache"), path.clone()] {
        let metadata = fs::metadata(path).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
    }
    let mut file = anchor.create_new("index-owner.json").unwrap();
    let metadata = file.metadata().unwrap();
    assert_eq!(metadata.len(), 0);
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
    assert_eq!(metadata.nlink(), 1);
    file.write_all(b"derived data").unwrap();
    assert_eq!(
        fs::read(path.join("index-owner.json")).unwrap(),
        b"derived data"
    );
    assert_eq!(
        anchor.create_new("index-owner.json").unwrap_err().code,
        "PATH_REFUSED"
    );
    assert_eq!(
        fs::read(path.join("index-owner.json")).unwrap(),
        b"derived data"
    );
}

#[cfg(unix)]
#[test]
fn private_child_does_not_hide_nonsticky_writable_ancestor() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let parent = root.join("shared");
    fs::create_dir(&parent).unwrap();
    let child = parent.join("repo/.kortex");
    fs::create_dir_all(&child).unwrap();
    mode(&parent.join("repo"), 0o700);
    mode(&child, 0o700);
    fs::write(child.join("sentinel"), b"preserve").unwrap();
    for bits in [0o770, 0o772, 0o775, 0o777, 0o707, 0o722] {
        mode(&parent, bits);
        let result = DirectoryAnchor::acquire(&child, false);
        mode(&parent, 0o700);
        let error = result.unwrap_err();
        assert_eq!(error.code, "PATH_REFUSED", "{bits:o}");
        assert!(error.message.contains("non-sticky"), "{}", error.message);
        assert_eq!(fs::metadata(&child).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::read(child.join("sentinel")).unwrap(), b"preserve");
    }
    // Refusal precedes creating any descendant under an unsafe ancestor.
    mode(&parent, 0o777);
    let result = DirectoryAnchor::acquire(&parent.join("must-not-exist/cache"), true);
    mode(&parent, 0o700);
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
    assert!(!parent.join("must-not-exist").exists());
}

#[cfg(unix)]
#[test]
fn sticky_intermediate_is_supported_but_not_a_private_final_directory() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let shared = root.join("sticky");
    fs::create_dir(&shared).unwrap();
    mode(&shared, 0o1777);
    let result = DirectoryAnchor::acquire(&shared.join("private/.kortex"), true);
    let public_final = DirectoryAnchor::acquire(&shared, false);
    mode(&shared, 0o700);
    let anchor = result.unwrap();
    assert_eq!(public_final.unwrap_err().code, "PATH_REFUSED");
    anchor.create_new("safe.db").unwrap();
    assert!(shared.join("private/.kortex/safe.db").is_file());
    // Also use the real system temporary directory, including root-owned sticky
    // /tmp where present. Canonical spelling avoids macOS's /tmp -> /private/tmp.
    let system = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let system_path = system.path().canonicalize().unwrap();
    DirectoryAnchor::acquire(&system_path.join(".kortex"), true).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinks_and_permissive_existing_targets_are_never_adopted_or_chmodded() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let public = root.join("public");
    fs::create_dir(&public).unwrap();
    mode(&public, 0o755);
    assert_eq!(
        DirectoryAnchor::acquire(&public, false).unwrap_err().code,
        "PATH_REFUSED"
    );
    assert_eq!(fs::metadata(&public).unwrap().mode() & 0o777, 0o755);
    mode(&public, 0o700);
    symlink(&public, root.join("alias")).unwrap();
    assert_eq!(
        DirectoryAnchor::acquire(&root.join("alias/.kortex"), true)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );
    assert!(!public.join(".kortex").exists());
    let anchor = DirectoryAnchor::acquire(&public, false).unwrap();
    fs::write(public.join("original"), b"preserve").unwrap();
    fs::hard_link(public.join("original"), public.join("hardlink")).unwrap();
    symlink("original", public.join("symlink")).unwrap();
    fs::create_dir(public.join("directory")).unwrap();
    for name in ["original", "hardlink", "symlink", "directory"] {
        assert_eq!(anchor.create_new(name).unwrap_err().code, "PATH_REFUSED");
    }
    assert_eq!(fs::read(public.join("original")).unwrap(), b"preserve");
    for path in ["relative/path", "../escape", "/../escape"] {
        assert_eq!(
            DirectoryAnchor::acquire(Path::new(path), true)
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }
    for name in ["", ".", "..", "a/b", "a\\b", "../escape", "a\0b"] {
        assert_eq!(
            anchor.create_new(name).unwrap_err().code,
            "PATH_REFUSED",
            "{name:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn descriptor_creation_remains_on_original_identity_after_accidental_rename() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let path = root.join(".kortex");
    let anchor = DirectoryAnchor::acquire(&path, true).unwrap();
    fs::rename(&path, root.join("original")).unwrap();
    fs::create_dir(&path).unwrap();
    mode(&path, 0o700);
    fs::write(path.join("sentinel"), b"replacement").unwrap();
    anchor.create_new("pinned.db").unwrap();
    assert!(root.join("original/pinned.db").is_file());
    assert!(!path.join("pinned.db").exists());
    assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"replacement");
    // Detect accidental mode changes before any subsequent file is created.
    mode(&root.join("original"), 0o777);
    let result = anchor.create_new("must-not-exist");
    mode(&root.join("original"), 0o700);
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
    assert!(!root.join("original/must-not-exist").exists());
}

#[cfg(unix)]
#[test]
fn checked_existing_open_never_creates_truncates_or_adopts_insecure_files() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("private");
    let anchor = DirectoryAnchor::acquire(&path, true).unwrap();
    anchor
        .create_new("valid")
        .unwrap()
        .write_all(b"preserve")
        .unwrap();
    let mut reader = anchor.open_existing("valid", false).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"preserve");
    assert!(reader.write_all(b"not writable").is_err());
    let writer = anchor.open_existing("valid", true).unwrap();
    assert_eq!(writer.metadata().unwrap().len(), 8);
    drop(writer);
    assert_eq!(
        anchor.open_existing("missing", true).unwrap_err().code,
        "PATH_REFUSED"
    );
    assert!(!path.join("missing").exists());
    fs::hard_link(path.join("valid"), path.join("hardlink")).unwrap();
    for name in ["hardlink", "valid"] {
        assert_eq!(
            anchor.open_existing(name, false).unwrap_err().code,
            "PATH_REFUSED"
        );
    }
    fs::remove_file(path.join("hardlink")).unwrap();
    mode(&path.join("valid"), 0o644);
    for writable in [false, true] {
        assert_eq!(
            anchor.open_existing("valid", writable).unwrap_err().code,
            "PATH_REFUSED"
        );
    }
    assert_eq!(
        fs::metadata(path.join("valid")).unwrap().mode() & 0o777,
        0o644
    );
    assert_eq!(fs::read(path.join("valid")).unwrap(), b"preserve");
    symlink("valid", path.join("alias")).unwrap();
    fs::create_dir(path.join("directory")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    for name in [
        "alias",
        "directory",
        "fifo",
        "../escape",
        "a/b",
        "a\\b",
        "",
        ".",
        "..",
    ] {
        assert_eq!(
            anchor.open_existing(name, false).unwrap_err().code,
            "PATH_REFUSED",
            "{name:?}"
        );
    }
}

#[cfg(not(unix))]
#[test]
fn unsupported_privacy_fails_before_creation_or_adopting_existing_state() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join("must-not-exist/.kortex");
    assert_eq!(
        require_derived_data_support().unwrap_err().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert_eq!(
        DirectoryAnchor::acquire(&missing, true).unwrap_err().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert!(!d.path().join("must-not-exist").exists());
    fs::write(d.path().join("sentinel"), b"preserve").unwrap();
    assert_eq!(
        DirectoryAnchor::acquire(d.path(), false).unwrap_err().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    assert_eq!(fs::read(d.path().join("sentinel")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
}
