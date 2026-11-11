//! Real Windows filesystem tests. Run on NTFS; junctions need no Developer Mode.
use super::*;
use std::{fs, io::Read};

#[test]
fn junction_parent_replaced_after_resolution_cannot_escape() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let parent = root.join("nested");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source.rs"), "safe").unwrap();
    fs::write(outside.path().join("source.rs"), "secret").unwrap();
    let canonical = parent.join("source.rs").canonicalize().unwrap();
    fs::rename(&parent, root.join("original")).unwrap();
    junction::create(outside.path(), &parent).unwrap();
    let result = open(&root, &canonical);
    junction::delete(&parent).unwrap();
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
    assert_eq!(
        fs::read_to_string(outside.path().join("source.rs")).unwrap(),
        "secret"
    );
}

#[test]
fn canonical_internal_junction_alias_reads_but_external_alias_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("source.rs"), "safe").unwrap();
    fs::write(outside.path().join("source.rs"), "secret").unwrap();
    let internal = root.join("internal");
    let external = root.join("external");
    junction::create(&target, &internal).unwrap();
    junction::create(outside.path(), &external).unwrap();
    let inside = crate::read_confined(&root, "internal/source.rs", 4);
    let escape = crate::read_confined(&root, "external/source.rs", 100);
    junction::delete(&internal).unwrap();
    junction::delete(&external).unwrap();
    assert_eq!(inside.unwrap(), "safe");
    assert_eq!(escape.unwrap_err().code, "PATH_REFUSED");
}

#[test]
fn replaced_leaf_directory_junction_and_root_reparse_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("source.rs");
    fs::write(&source, "safe").unwrap();
    let canonical = source.canonicalize().unwrap();
    fs::remove_file(&source).unwrap();
    junction::create(outside.path(), &source).unwrap();
    let result = open(&root, &canonical);
    junction::delete(&source).unwrap();
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");

    // Use a child root so TempDir's own handle does not obstruct the rename.
    let anchor = root.join("anchor");
    fs::create_dir(&anchor).unwrap();
    fs::write(anchor.join("source.rs"), "safe").unwrap();
    let canonical_root = anchor.canonicalize().unwrap();
    let canonical = canonical_root.join("source.rs");
    fs::rename(&anchor, root.join("old-anchor")).unwrap();
    junction::create(outside.path(), &anchor).unwrap();
    let result = open(&canonical_root, &canonical);
    junction::delete(&anchor).unwrap();
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
}

#[test]
fn devices_streams_prefixes_and_nonregular_handles_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("source.rs"), "safe").unwrap();
    // NTFS alternate streams are real regular handles, but not source identities.
    fs::write(root.join("source.rs:hidden"), "hidden").unwrap();
    for relative in [
        "NUL",
        "CON",
        "COM1",
        "LPT1.txt",
        "source.rs:hidden",
        "source.rs.",
    ] {
        assert_eq!(
            open(&root, &root.join(relative)).unwrap_err().code,
            "PATH_REFUSED",
            "{relative}"
        );
    }
    for relative in [
        r"\\.\pipe\kortex-must-not-connect",
        r"C:\Windows\win.ini",
        r"C:source.rs",
        r"\Windows\win.ini",
        r"..\source.rs",
    ] {
        assert_eq!(
            crate::read_confined(&root, relative, 100).unwrap_err().code,
            "PATH_REFUSED",
            "{relative}"
        );
    }
    assert_eq!(open(&root, &root).unwrap_err().code, "PATH_REFUSED");
    let directory = root.join("directory.rs");
    fs::create_dir(&directory).unwrap();
    assert_eq!(open(&root, &directory).unwrap_err().code, "PATH_REFUSED");
    let mut source = String::new();
    open(&root, &root.join("source.rs"))
        .unwrap()
        .read_to_string(&mut source)
        .unwrap();
    assert_eq!(source, "safe");
}

#[test]
fn symlink_leaf_swap_is_refused_when_host_allows_symlinks() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("source.rs");
    fs::write(&source, "safe").unwrap();
    let canonical = source.canonicalize().unwrap();
    fs::remove_file(&source).unwrap();
    match std::os::windows::fs::symlink_file(outside.path(), &source) {
        Ok(()) => assert_eq!(open(&root, &canonical).unwrap_err().code, "PATH_REFUSED"),
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!(
                "Windows host lacks symlink privilege; junction tests are mandatory, leaf symlink runtime evidence remains missing"
            );
        }
        Err(error) => panic!("cannot create Windows symlink fixture: {error}"),
    }
}
