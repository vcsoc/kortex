#[cfg(unix)]
use super::*;
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[cfg(unix)]
#[test]
fn hostile_filenames_round_trip_without_lossy_aliases() {
    let d = tempfile::tempdir().unwrap();
    let root = root(d.path()).unwrap();
    let names = [
        r"literal\name.py",
        "quote\"name.py",
        "control\u{1b}name.py",
        "unicod\u{e9}.py",
        "bad\u{fffd}.py",
    ];
    for (n, name) in names.iter().enumerate() {
        fs::write(root.join(name), format!("VALUE = {n}\n")).unwrap();
    }
    let unsupported_created = match fs::write(
        root.join(OsString::from_vec(b"bad\xff.py".to_vec())),
        "BAD = 1\n",
    ) {
        Ok(()) => true,
        Err(error) => {
            assert!(
                cfg!(target_os = "macos") && error.raw_os_error() == Some(92),
                "{error}"
            );
            false
        }
    };
    let inventory = discover(&root, &Config::default()).unwrap();
    assert_eq!(inventory.files.len(), names.len());
    if unsupported_created {
        let native_name = OsString::from_vec(b"bad\xff.py".to_vec());
        let diagnostic = inventory
            .diagnostics
            .iter()
            .find(|d| d.contains("unsupported filename encoding"))
            .unwrap();
        assert!(diagnostic.contains(&format!("{:?}", Path::new(&native_name))));
        assert!(
            !diagnostic.contains("bad\u{fffd}.py"),
            "invalid name was aliased to a valid replacement-character filename"
        );
    }
    for source in inventory.files {
        assert!(names.contains(&source.path.as_str()));
        assert_eq!(
            source.source,
            fs::read_to_string(root.join(&source.path)).unwrap()
        );
        assert_eq!(source.content_hash, hash(&source.source));
    }
    assert_eq!(
        read_confined(&root, "../escape.py", 20).err().unwrap().code,
        "PATH_REFUSED"
    );
    assert_eq!(
        read_confined(&root, root.join(names[0]).to_str().unwrap(), 20)
            .err()
            .unwrap()
            .code,
        "PATH_REFUSED"
    );
    fs::write(root.join("large.py"), "1234").unwrap();
    assert_eq!(
        read_confined(&root, "large.py", 2).err().unwrap().code,
        "FILE_TOO_LARGE"
    );
    fs::write(root.join("binary.py"), [0]).unwrap();
    assert_eq!(
        read_confined(&root, "binary.py", 20).err().unwrap().code,
        "BINARY_FILE"
    );
    fs::write(root.join("encoding.py"), [0xff]).unwrap();
    assert_eq!(
        read_confined(&root, "encoding.py", 20).err().unwrap().code,
        "INVALID_ENCODING"
    );
    fs::create_dir(root.join("directory")).unwrap();
    assert_eq!(
        read_confined(&root, "directory", 20).err().unwrap().code,
        "PATH_REFUSED"
    );
    assert!(read_confined(&root, "missing.py", 20).is_err());
}

#[cfg(unix)]
#[test]
fn root_preserves_whitespace_and_rejects_unsupported_encoding() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repository \n");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--template="]).unwrap();
    fs::create_dir(repo.join("child")).unwrap();
    assert_eq!(
        root(&repo.join("child")).unwrap(),
        repo.canonicalize().unwrap()
    );
    let invalid = d.path().join(OsString::from_vec(b"root\xff".to_vec()));
    match fs::create_dir(&invalid) {
        Ok(()) => assert_eq!(root(&invalid).err().unwrap().code, "INVALID_PATH_ENCODING"),
        Err(error) => assert!(
            cfg!(target_os = "macos") && error.raw_os_error() == Some(92),
            "{error}"
        ),
    }
    let file = d.path().join("file.py");
    fs::write(&file, "").unwrap();
    assert_eq!(root(&file).err().unwrap().code, "REPO_NOT_FOUND");
    assert_eq!(
        root(&d.path().join("missing")).err().unwrap().code,
        "REPO_NOT_FOUND"
    );
}
