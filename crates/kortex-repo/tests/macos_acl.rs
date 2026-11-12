#![cfg(target_os = "macos")]
//! Real Darwin ACEs on temporary fixtures, never system/user directories.

#[test]
fn dev_fd_alias_is_not_evidence_of_the_held_files_acl() {
    use std::os::fd::AsRawFd;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("original");
    fs::write(&path, b"fixture").unwrap();
    ace(&path, "everyone allow read,write");
    let file = fs::File::open(&path).unwrap();
    let alias = format!("/dev/fd/{}", file.as_raw_fd());
    let expected = exacl::getfacl(&path, None).unwrap();
    let alias_acl = exacl::getfacl(&alias, None).unwrap();
    // Native fdescfs returns its own empty ACL, not the target ACL. Production
    // must never turn this tempting /dev/fd shortcut into false privacy evidence.
    assert!(expected.iter().any(|entry| entry.allow));
    assert_ne!(expected, alias_acl);
}

#[test]
fn readable_shared_source_acl_is_not_mistaken_for_private_derived_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("shared-source");
    private_dir(&path);
    fs::write(path.join("source.rs"), "// inert shared source\n").unwrap();
    ace(&path, "everyone allow read,search");
    let before = exacl::getfacl(&path, None).unwrap();
    let mode = fs::metadata(&path).unwrap().mode();
    let control = kortex_core::RequestControl::new(std::time::Duration::from_secs(30)).unwrap();
    let mut reader = kortex_repo::ConfinedReader::new(&path).unwrap();
    assert_eq!(
        reader.read("source.rs", 100, &control).unwrap(),
        "// inert shared source\n"
    );
    assert_eq!(
        kortex_repo::discover_with_control(&path, &kortex_config::Config::default(), &control)
            .unwrap()
            .files
            .len(),
        1
    );
    refused(DirectoryAnchor::acquire(&path, false));
    assert_eq!(before, exacl::getfacl(&path, None).unwrap());
    assert_eq!(mode, fs::metadata(&path).unwrap().mode());
}

use kortex_repo::placement::DirectoryAnchor;
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

fn ace(path: &Path, text: &str) {
    let output = std::process::Command::new("/bin/chmod")
        .args(["+a", text])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !exacl::getfacl(path, None).unwrap().is_empty(),
        "real ACE required"
    );
}
fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn refused<T: std::fmt::Debug>(result: kortex_core::Result<T>) {
    let error = result.unwrap_err();
    assert_eq!(error.code, "PATH_REFUSED", "{error}");
    assert!(error.message.contains("ACL"), "{error}");
}

#[test]
fn explicit_allow_overrides_private_directory_mode_and_is_not_stripped() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let dir = root.join("private");
    private_dir(&dir);
    ace(
        &dir,
        "everyone allow read,write,search,delete_child,writesecurity",
    );
    let before = exacl::getfacl(&dir, None).unwrap();
    assert!(before.iter().any(|entry| entry.allow));
    assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    for create in [false, true] {
        refused(DirectoryAnchor::acquire(&dir, create));
    }
    refused(DirectoryAnchor::acquire(
        &dir.join("must-not-exist/cache"),
        true,
    ));
    assert!(!dir.join("must-not-exist").exists());
    assert_eq!(exacl::getfacl(&dir, None).unwrap(), before);
    assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
}

#[test]
fn inherited_and_inherit_only_allows_fail_before_any_descendant_creation() {
    for only in ["", ",only_inherit"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent = root.join("parent");
        private_dir(&parent);
        ace(
            &parent,
            &format!(
                "everyone allow read,write,search,delete_child,writesecurity,file_inherit,directory_inherit{only}"
            ),
        );
        let child = parent.join("already-inherited");
        private_dir(&child);
        let inherited = exacl::getfacl(&child, None).unwrap();
        assert!(
            inherited
                .iter()
                .any(|e| e.allow && e.flags.contains(exacl::Flag::INHERITED)),
            "actual inherited allow required: {inherited:?}"
        );
        assert_eq!(fs::metadata(&child).unwrap().mode() & 0o777, 0o700);
        refused(DirectoryAnchor::acquire(&child, false));
        refused(DirectoryAnchor::acquire(
            &parent.join("missing/cache"),
            true,
        ));
        assert!(!parent.join("missing").exists());
        assert_eq!(exacl::getfacl(&child, None).unwrap(), inherited);
    }
}

#[test]
fn ancestor_replacement_rights_are_not_hidden_by_a_private_leaf() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let ancestor = root.join("ancestor");
    private_dir(&ancestor);
    let leaf = ancestor.join("private");
    private_dir(&leaf);
    ace(
        &ancestor,
        "everyone allow delete_child,add_subdirectory,search",
    );
    refused(DirectoryAnchor::acquire(&leaf, false));
    assert!(!leaf.join("secret").exists());
}

#[test]
fn post_acquisition_inheritance_is_refused_before_new_secret_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let anchor = DirectoryAnchor::acquire(&path, false).unwrap();
    ace(&path, "everyone allow read,file_inherit,only_inherit");
    refused(anchor.create_new("must-not-exist"));
    assert!(!path.join("must-not-exist").exists());
}

#[test]
fn existing_0600_file_allow_is_refused_without_truncation_or_acl_repair() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let anchor = DirectoryAnchor::acquire(&path, false).unwrap();
    let file_path = path.join("existing");
    anchor
        .create_new("existing")
        .unwrap()
        .write_all(b"preserve secret")
        .unwrap();
    ace(&file_path, "everyone allow read,write,writesecurity");
    let before = exacl::getfacl(&file_path, None).unwrap();
    for writable in [false, true] {
        refused(anchor.open_existing("existing", writable));
    }
    assert_eq!(fs::read(&file_path).unwrap(), b"preserve secret");
    assert_eq!(fs::metadata(&file_path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(exacl::getfacl(&file_path, None).unwrap(), before);
}

#[test]
fn deny_only_defaults_remain_usable_and_inherited_without_stripping() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap();
    ace(&path, "everyone deny delete,file_inherit,directory_inherit");
    let before = exacl::getfacl(&path, None).unwrap();
    let nested = path.join("new/cache");
    let anchor = DirectoryAnchor::acquire(&nested, true).unwrap();
    anchor
        .create_new("secret")
        .unwrap()
        .write_all(b"owner data")
        .unwrap();
    let mut source = String::new();
    anchor
        .open_existing("secret", false)
        .unwrap()
        .read_to_string(&mut source)
        .unwrap();
    assert_eq!(source, "owner data");
    assert_eq!(exacl::getfacl(&path, None).unwrap(), before);
    let entries = exacl::getfacl(nested.join("secret"), None).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| !e.allow && e.flags.contains(exacl::Flag::INHERITED))
    );
}
