//! Exercise real files through the production byte reader, not an ambient test opener.
use super::{confined, read_confined, read_confined_bytes};
use std::fs;

#[test]
fn binary_empty_limits_and_text_compatibility() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    // PNG signature and all byte values include NUL and non-UTF-8 bytes.
    let mut binary = b"\x89PNG\r\n\x1a\n".to_vec();
    binary.extend(0..=255);
    fs::write(root.join("asset.png"), &binary).unwrap();
    for max in [binary.len() as u64, binary.len() as u64 + 1, u64::MAX] {
        assert_eq!(
            read_confined_bytes(&root, "asset.png", max).unwrap(),
            binary
        );
    }
    for max in [0, 1, binary.len() as u64 - 1] {
        assert_eq!(
            read_confined_bytes(&root, "asset.png", max)
                .unwrap_err()
                .code,
            "FILE_TOO_LARGE"
        );
    }
    assert_eq!(
        read_confined(&root, "asset.png", u64::MAX)
            .unwrap_err()
            .code,
        "BINARY_FILE"
    );
    fs::write(root.join("invalid.txt"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        read_confined_bytes(&root, "invalid.txt", 2).unwrap(),
        [0xff, 0xfe]
    );
    assert_eq!(
        read_confined(&root, "invalid.txt", 2).unwrap_err().code,
        "INVALID_ENCODING"
    );
    fs::write(root.join("empty"), []).unwrap();
    assert!(read_confined_bytes(&root, "empty", 0).unwrap().is_empty());
    assert_eq!(read_confined(&root, "empty", 0).unwrap(), "");
    fs::write(root.join("utf8.txt"), "é").unwrap();
    assert_eq!(
        read_confined_bytes(&root, "utf8.txt", 2).unwrap(),
        "é".as_bytes()
    );
    assert_eq!(read_confined(&root, "utf8.txt", 2).unwrap(), "é");
    fs::create_dir(root.join("directory")).unwrap();
    assert_eq!(
        read_confined_bytes(&root, "directory", 100)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );
    for relative in ["../asset.png", "/asset.png", "nested/../asset.png"] {
        assert_eq!(
            read_confined_bytes(&root, relative, 100).unwrap_err().code,
            "PATH_REFUSED"
        );
    }
}

#[test]
fn bounded_generated_binary_and_hostile_path_corpus() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    for length in [0, 1, 2, 31, 1024, 65536] {
        let bytes: Vec<_> = (0..length).map(|i| ((i * 73 + 19) % 256) as u8).collect();
        fs::write(root.join("asset.bin"), &bytes).unwrap();
        assert_eq!(
            read_confined_bytes(&root, "asset.bin", length as u64).unwrap(),
            bytes
        );
        assert_eq!(
            read_confined_bytes(&root, "asset.bin", u64::MAX).unwrap(),
            bytes
        );
        if length > 0 {
            assert_eq!(
                read_confined_bytes(&root, "asset.bin", length as u64 - 1)
                    .unwrap_err()
                    .code,
                "FILE_TOO_LARGE"
            );
        }
    }
    for depth in 1..=256 {
        let path = format!("{}asset.bin", "../".repeat(depth));
        assert_eq!(
            read_confined_bytes(&root, &path, u64::MAX)
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }
    for path in ["", "\0", "asset.bin\0ignored", "./../asset.bin"] {
        assert!(
            read_confined_bytes(&root, path, u64::MAX).is_err(),
            "{path:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn bytes_preserve_alias_policy_and_refuse_leaf_parent_swaps_and_fifo() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    fs::write(root.join("asset.bin"), [0, 0xff]).unwrap();
    fs::write(outside.path().join("asset.bin"), b"secret").unwrap();
    symlink("asset.bin", root.join("alias.bin")).unwrap();
    assert_eq!(
        read_confined_bytes(&root, "alias.bin", 2).unwrap(),
        [0, 0xff]
    );
    symlink(outside.path(), root.join("external")).unwrap();
    assert_eq!(
        read_confined_bytes(&root, "external/asset.bin", 100)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );

    let canonical = root.join("asset.bin").canonicalize().unwrap();
    fs::rename(&canonical, root.join("original.bin")).unwrap();
    symlink(outside.path().join("asset.bin"), &canonical).unwrap();
    assert_eq!(
        confined::read_bytes(&root, &canonical, 100)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );

    let parent = root.join("nested");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("asset.bin"), [0, 0xfe]).unwrap();
    let canonical = parent.join("asset.bin").canonicalize().unwrap();
    fs::rename(&parent, root.join("original-parent")).unwrap();
    symlink(outside.path(), &parent).unwrap();
    assert_eq!(
        confined::read_bytes(&root, &canonical, 100)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );
    let fifo = root.join("pipe.bin");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        read_confined_bytes(&root, "pipe.bin", 100)
            .unwrap_err()
            .code,
        "PATH_REFUSED"
    );
    assert_eq!(
        fs::read(outside.path().join("asset.bin")).unwrap(),
        b"secret"
    );
}

#[cfg(windows)]
#[test]
fn bytes_windows_junction_swaps_streams_and_devices() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let parent = root.join("nested");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("asset.bin"), [0, 0xff]).unwrap();
    fs::write(outside.path().join("asset.bin"), b"secret").unwrap();
    junction::create(&parent, root.join("alias")).unwrap();
    let result = read_confined_bytes(&root, "alias/asset.bin", 2);
    junction::delete(root.join("alias")).unwrap();
    assert_eq!(result.unwrap(), [0, 0xff]);

    let canonical = parent.join("asset.bin").canonicalize().unwrap();
    fs::rename(&parent, root.join("original-parent")).unwrap();
    junction::create(outside.path(), &parent).unwrap();
    let result = confined::read_bytes(&root, &canonical, 100);
    let escape = read_confined_bytes(&root, "nested/asset.bin", 100);
    junction::delete(&parent).unwrap();
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
    assert_eq!(escape.unwrap_err().code, "PATH_REFUSED");

    let leaf = root.join("leaf.bin");
    fs::write(&leaf, [0, 0xff]).unwrap();
    let canonical = leaf.canonicalize().unwrap();
    fs::remove_file(&leaf).unwrap();
    junction::create(outside.path(), &leaf).unwrap();
    let result = confined::read_bytes(&root, &canonical, 100);
    junction::delete(&leaf).unwrap();
    assert_eq!(result.unwrap_err().code, "PATH_REFUSED");
    fs::write(&leaf, [0, 0xff]).unwrap();
    fs::write(root.join("leaf.bin:hidden"), b"stream").unwrap();
    for path in [
        "NUL",
        "CON",
        "COM1",
        "leaf.bin:hidden",
        r"..\leaf.bin",
        r"C:leaf.bin",
        r"\\.\pipe\kortex-must-not-connect",
    ] {
        assert!(read_confined_bytes(&root, path, 100).is_err(), "{path}");
    }
}
