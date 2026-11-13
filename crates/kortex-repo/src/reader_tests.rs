use super::*;
use std::time::{Duration, Instant};

fn control() -> RequestControl {
    RequestControl::new(Duration::from_secs(30)).unwrap()
}

#[test]
fn reused_scratch_never_exposes_prior_file_bytes_after_short_reads_or_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let large = vec![0xa5; 2 * crate::confined::READ_CHUNK_BYTES + 37];
    fs::write(root.join("large"), &large).unwrap();
    fs::write(root.join("short"), b"tiny").unwrap();
    fs::write(root.join("empty"), []).unwrap();
    fs::write(root.join("binary"), [0, 0xff]).unwrap();
    fs::write(root.join("invalid"), [0xff]).unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    for _ in 0..3 {
        assert_eq!(
            reader
                .read_bytes("large", large.len() as u64, &control())
                .unwrap(),
            large
        );
        assert_eq!(reader.read_bytes("short", 4, &control()).unwrap(), b"tiny");
        assert!(
            reader
                .read_bytes("empty", 0, &control())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reader.read_bytes("large", 17, &control()).unwrap_err().code,
            "FILE_TOO_LARGE"
        );
        assert!(
            reader
                .read_bytes("empty", u64::MAX, &control())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reader.read("binary", 2, &control()).unwrap_err().code,
            "BINARY_FILE"
        );
        assert_eq!(
            reader.read("invalid", 1, &control()).unwrap_err().code,
            "INVALID_ENCODING"
        );
        assert_eq!(reader.read("short", u64::MAX, &control()).unwrap(), "tiny");
    }
    assert_eq!(reader.stats().root_opens, 1);
}

#[test]
fn reused_scratch_keeps_real_eof_authoritative_after_growth_and_shrink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    for size in [0, 1, 65535, 65536, 65537, 3, 0] {
        let bytes = vec![b'x'; size];
        fs::write(root.join("changing"), &bytes).unwrap();
        assert_eq!(
            reader
                .read_bytes("changing", size as u64, &control())
                .unwrap(),
            bytes
        );
        if size != 0 {
            assert_eq!(
                reader
                    .read_bytes("changing", size as u64 - 1, &control())
                    .unwrap_err()
                    .code,
                "FILE_TOO_LARGE"
            );
        }
    }
}

#[test]
fn session_bytes_text_bounds_and_errors_match_standalone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("source.rs"), "fn source() {}\n").unwrap();
    fs::write(root.join("binary.rs"), [0, 0xff]).unwrap();
    fs::write(root.join("invalid.rs"), [0xff, 0xfe]).unwrap();
    fs::write(root.join("empty.rs"), []).unwrap();
    fs::write(root.join("utf8.rs"), "\u{e9}").unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    for path in [
        "source.rs",
        "binary.rs",
        "invalid.rs",
        "empty.rs",
        "utf8.rs",
        "directory",
        "missing.rs",
        "../source.rs",
        "",
        "\0",
        "source.rs\0hidden",
    ] {
        for max in [0, 1, 2, 14, 15, u64::MAX] {
            let old =
                read_confined_bytes_with_control(&root, path, max, &control()).map_err(|e| e.code);
            let new = reader.read_bytes(path, max, &control()).map_err(|e| e.code);
            assert_eq!(old, new, "bytes {path:?}, {max}");
            let old = read_confined_with_control(&root, path, max, &control()).map_err(|e| e.code);
            let new = reader.read(path, max, &control()).map_err(|e| e.code);
            assert_eq!(old, new, "text {path:?}, {max}");
        }
    }
    assert_eq!(reader.stats().root_opens, 1);
    for expired in [false, true] {
        let c = if expired {
            RequestControl::from_deadline(Instant::now())
        } else {
            let c = control();
            c.cancel();
            c
        };
        let stats = reader.stats().clone();
        assert_eq!(
            reader.read("source.rs", 100, &c).unwrap_err().code,
            if expired { "TIMEOUT" } else { "CANCELLED" }
        );
        assert_eq!(*reader.stats(), stats, "cancel before any new I/O");
    }
}

fn compare_discovery(root: &Path, config: &Config) {
    let c = control();
    let reference = discover_using(root, config, Some(&c), true, None, |_, path| {
        read_confined_with_control(root, path, config.repository.max_file_size_bytes, &c)
    })
    .unwrap();
    let current = discover_with_control(root, config, &c).unwrap();
    assert_eq!(reference.diagnostics, current.diagnostics);
    assert_eq!(
        reference
            .files
            .iter()
            .map(|f| (&f.path, &f.source, &f.content_hash))
            .collect::<Vec<_>>(),
        current
            .files
            .iter()
            .map(|f| (&f.path, &f.source, &f.content_hash))
            .collect::<Vec<_>>()
    );
    for file in &current.files {
        assert_eq!(file.content_hash, hash(&file.source));
    }
}

#[test]
fn discovery_reference_agrees_through_real_edits_add_delete_ignores_manifests_and_limits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    for path in ["one.rs", "nested/two.py", "nested/three.rs"] {
        fs::write(root.join(path), "# inert source only\n").unwrap();
    }
    fs::write(root.join("invalid.rs"), [0xff]).unwrap();
    fs::write(root.join("nul.rs"), [0]).unwrap();
    let mut config = Config::default();
    compare_discovery(&root, &config);
    // Same-size bytes are always reread, even with restored mtime.
    let file = fs::File::open(root.join("one.rs")).unwrap();
    let modified = file.metadata().unwrap().modified().unwrap();
    fs::write(root.join("one.rs"), "# changed bytes now\n").unwrap();
    file.set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::remove_file(root.join("nested/two.py")).unwrap();
    fs::write(root.join("new.go"), "package inert\n").unwrap();
    compare_discovery(&root, &config);
    let before = inputs::resolution_fingerprint_with_control(&root, &control()).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname='inert'\n").unwrap();
    fs::write(root.join(".kortexignore"), "one.rs\n").unwrap();
    fs::write(root.join("nested/.ignore"), "three.rs\n").unwrap();
    compare_discovery(&root, &config);
    assert_ne!(
        before,
        inputs::resolution_fingerprint_with_control(&root, &control()).unwrap()
    );
    for limit in [0, 1, 10] {
        config.repository.max_files = limit;
        compare_discovery(&root, &config);
    }
    config.repository.max_files = 100;
    config.repository.max_total_bytes = 1;
    compare_discovery(&root, &config);
}

#[test]
fn shared_session_rereads_same_size_restored_mtime_and_atomic_replacements() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("source.rs");
    fs::write(&path, "inert source 000\n").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    let mut previous = String::new();
    for index in 0..64 {
        let source = format!("inert source {index:03}\n");
        if index % 2 == 0 {
            fs::write(&path, &source).unwrap();
        } else {
            fs::write(root.join("replacement.rs"), &source).unwrap();
            fs::rename(root.join("replacement.rs"), &path).unwrap();
        }
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let actual = reader.read("source.rs", 17, &control()).unwrap();
        assert_eq!(actual, source);
        let digest = hash(actual);
        assert_ne!(digest, previous);
        previous = digest;
    }
    assert_eq!(reader.stats().root_opens, 1);
    assert_eq!(reader.stats().file_opens, 64);
    assert_eq!(reader.stats().bytes_read, 64 * 17);
}

#[test]
fn session_cancellation_after_real_chunk_does_not_read_remaining_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("big.rs"), vec![b'a'; 3 * 65536]).unwrap();
    let mut reader = ConfinedReader::new(&root).unwrap();
    let c = control();
    let cancel = c.clone();
    confined::READ_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let result = reader.read("big.rs", u64::MAX, &c);
    confined::READ_HOOK.with(|h| *h.borrow_mut() = None);
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert_eq!(reader.stats().bytes_read, 65536);
    assert_eq!(reader.stats().read_calls, 1);
}

#[test]
fn profiled_discovery_matches_plain_and_counts_all_source_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("one.rs"), "fn one() {}\n").unwrap();
    fs::write(root.join("nested/two.py"), "def two(): pass\n").unwrap();
    let config = Config::default();
    compare_discovery(&root, &config);
    let (inventory, stats) = discover_profiled(&root, &config, &control()).unwrap();
    assert_eq!(inventory.files.len(), 2);
    assert_eq!(stats.reader.bytes_read, 28);
    assert_eq!(stats.reader.file_opens, 2);
    assert_eq!(stats.reader.root_opens, 1);
    assert_eq!(stats.reader.canonicalizations, 1);
    if cfg!(unix) {
        assert_eq!(stats.entry_canonicalizations, 0);
        assert_eq!(stats.descriptor_paths, 2);
        assert_eq!(stats.reader.root_checks, 5);
        assert_eq!(stats.reader.leaf_checks, 2);
    }
    let c = control();
    c.cancel();
    assert_eq!(
        discover_profiled(&root, &config, &c).err().unwrap().code,
        "CANCELLED"
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn descriptor_discovery_keeps_exact_alias_order_filters_utf8_and_limits() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/z.rs"), "fn z() {}\n").unwrap();
        fs::write(root.join("\u{e9}.py"), "# \u{e9}\n").unwrap();
        fs::write(root.join(".hidden.rs"), "fn hidden() {}\n").unwrap();
        fs::write(root.join("nul.rs"), [0]).unwrap();
        fs::write(root.join("invalid.rs"), [0xff]).unwrap();
        symlink("nested/z.rs", root.join("a.rs")).unwrap();
        symlink("nested", root.join("alias-dir")).unwrap();
        symlink(outside.path(), root.join("escape")).unwrap();
        symlink(".", root.join("loop")).unwrap();
        let mut config = Config::default();
        for follow in [false, true] {
            config.repository.follow_symlinks = follow;
            for hidden in [false, true] {
                config.repository.include_hidden = hidden;
                compare_discovery(&root, &config);
                for max in [0, 1, 4, 100] {
                    config.repository.max_files = max;
                    compare_discovery(&root, &config);
                }
                for max in [0, 1, 8, u64::MAX] {
                    config.repository.max_file_size_bytes = max;
                    compare_discovery(&root, &config);
                }
                config.repository.max_file_size_bytes = 2 * 1024 * 1024;
            }
            config.repository.include = vec!["*.rs".into(), "nested/**".into()];
            config.repository.exclude = vec!["invalid.rs".into()];
            compare_discovery(&root, &config);
            config.repository.include.clear();
            config.repository.exclude.clear();
        }
    }

    #[test]
    fn deep_discovery_keeps_two_uncached_descriptor_walks_without_realpath() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = format!("{}source.rs", "d/".repeat(35));
        fs::create_dir_all(root.join(&path).parent().unwrap()).unwrap();
        fs::write(root.join(&path), "safe").unwrap();
        compare_discovery(&root, &Config::default());
        let (inventory, stats) = discover_profiled(&root, &Config::default(), &control()).unwrap();
        assert_eq!(inventory.files.len(), 1);
        assert_eq!(inventory.files[0].path, path);
        assert_eq!(stats.entry_canonicalizations, 0);
        assert_eq!(stats.reader.canonicalizations, 1);
        assert_eq!(stats.reader.parent_opens, 70);
        assert_eq!(stats.reader.file_opens, 2);
        assert_eq!(stats.reader.bytes_read, 4);
    }

    #[test]
    fn discovery_never_treats_a_regular_leaf_as_parent_confinement_proof() {
        for cached in [false, true] {
            for outside_link in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let root = temp.path().canonicalize().unwrap();
                fs::create_dir(root.join("nested")).unwrap();
                fs::write(root.join("nested/source.rs"), "safe").unwrap();
                fs::write(outside.path().join("source.rs"), "secret").unwrap();
                let c = control();
                let config = Config::default();
                let mut reader = ConfinedReader::new(&root).unwrap();
                if cached {
                    reader.read("nested/source.rs", 100, &c).unwrap();
                }
                let inventory =
                    discover_using(&root, &config, Some(&c), false, None, |candidate, _| {
                        fs::rename(root.join("nested"), root.join("saved")).unwrap();
                        symlink(
                            if outside_link {
                                outside.path().to_owned()
                            } else {
                                root.join("saved")
                            },
                            root.join("nested"),
                        )
                        .unwrap();
                        reader
                            .read_resolved(candidate, 100, Some(&c))
                            .and_then(reader::text)
                    })
                    .unwrap();
                assert!(inventory.files.is_empty());
                assert_eq!(inventory.diagnostics.len(), 1);
                assert!(inventory.diagnostics[0].contains("PATH_REFUSED"));
                assert_eq!(reader.stats().bytes_read, if cached { 4 } else { 0 });
            }
        }
    }

    #[test]
    fn descriptor_discovery_refuses_post_read_swaps_and_root_replacement() {
        for kind in ["leaf", "parent", "root"] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("repo");
            fs::create_dir_all(root.join("nested")).unwrap();
            fs::write(root.join("nested/source.rs"), "safe").unwrap();
            let moved = root.clone();
            let mut once = true;
            confined::READ_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |_| {
                    if !once {
                        return;
                    }
                    once = false;
                    match kind {
                        "leaf" => {
                            fs::rename(moved.join("nested/source.rs"), moved.join("saved.rs"))
                                .unwrap();
                            symlink("../saved.rs", moved.join("nested/source.rs")).unwrap();
                        }
                        "parent" => {
                            fs::rename(moved.join("nested"), moved.join("saved")).unwrap();
                            symlink("saved", moved.join("nested")).unwrap();
                        }
                        "root" => {
                            let old = moved.with_extension("old");
                            fs::rename(&moved, &old).unwrap();
                            symlink(old, &moved).unwrap();
                        }
                        _ => unreachable!(),
                    }
                }));
            });
            let result = discover_profiled(&root, &Config::default(), &control());
            confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
            if kind == "root" {
                assert_eq!(result.err().unwrap().code, "PATH_REFUSED");
            } else {
                let (inventory, stats) = result.unwrap();
                assert!(inventory.files.is_empty());
                assert!(
                    inventory
                        .diagnostics
                        .iter()
                        .any(|d| d.contains("PATH_REFUSED"))
                );
                assert_eq!(stats.reader.bytes_read, 4);
            }
        }
    }

    #[test]
    fn parent_reuse_is_real_bounded_and_does_not_impose_private_cache_policy() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/source.rs"), "readable shared source").unwrap();
        let mut reader = ConfinedReader::new(&root).unwrap();
        for _ in 0..100 {
            assert_eq!(
                reader.read("a/b/source.rs", 100, &control()).unwrap(),
                "readable shared source"
            );
        }
        assert_eq!(reader.stats().parent_opens, 2);
        assert_eq!(reader.stats().parent_checks, 398);
        assert_eq!(reader.stats().file_opens, 100);
        assert_eq!(reader.stats().read_calls, 200);
        assert_eq!(reader.stats().bytes_read, 2200);
        let deep = format!("{}source.rs", "d/".repeat(35));
        fs::create_dir_all(root.join(&deep).parent().unwrap()).unwrap();
        fs::write(root.join(&deep), "deep inert source").unwrap();
        assert_eq!(
            reader.read(&deep, 100, &control()).unwrap(),
            read_confined(&root, &deep, 100).unwrap()
        );
        assert_eq!(
            reader.stats().parent_opens,
            72,
            "deep paths use two bounded-FD walks, not 35 cached handles"
        );
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn aliases_fifo_and_resolved_leaf_parent_root_swaps_remain_confined() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(root.join("safe.rs"), "safe").unwrap();
        fs::write(outside.path().join("safe.rs"), "secret").unwrap();
        symlink("safe.rs", root.join("alias.rs")).unwrap();
        symlink(outside.path(), root.join("escape")).unwrap();
        let mut reader = ConfinedReader::new(&root).unwrap();
        assert_eq!(
            reader.read("alias.rs", 100, &control()).unwrap(),
            read_confined(&root, "alias.rs", 100).unwrap()
        );
        assert_eq!(
            reader
                .read("escape/safe.rs", 100, &control())
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
        let canonical = root.join("safe.rs").canonicalize().unwrap();
        fs::rename(&canonical, root.join("old.rs")).unwrap();
        symlink(outside.path().join("safe.rs"), &canonical).unwrap();
        assert_eq!(
            reader
                .read_resolved(&canonical, 100, Some(&control()))
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("pipe.rs"))
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            reader.read("pipe.rs", 100, &control()).unwrap_err().code,
            "PATH_REFUSED"
        );
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/safe.rs"), "old parent").unwrap();
        assert_eq!(
            reader.read("nested/safe.rs", 100, &control()).unwrap(),
            "old parent"
        );
        fs::rename(root.join("nested"), root.join("old-parent")).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        // Even returning the original leaf inode does NOT validate a cached parent.
        fs::rename(root.join("old-parent/safe.rs"), root.join("nested/safe.rs")).unwrap();
        assert_eq!(
            reader
                .read("nested/safe.rs", 100, &control())
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
        assert_eq!(
            reader.read("nested/safe.rs", 100, &control()).unwrap(),
            "old parent"
        );
        fs::rename(root.join("nested"), root.join("another-old-parent")).unwrap();
        symlink(outside.path(), root.join("nested")).unwrap();
        assert_eq!(
            reader
                .read_resolved(&root.join("nested/safe.rs"), 100, Some(&control()))
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }

    #[test]
    fn swaps_after_cached_parent_validation_and_before_open_are_refused() {
        for kind in ["leaf", "parent", "root", "cancel"] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("repo");
            fs::create_dir_all(root.join("nested")).unwrap();
            fs::write(root.join("nested/source.rs"), "safe").unwrap();
            let mut reader = ConfinedReader::new(&root).unwrap();
            reader.read("nested/source.rs", 100, &control()).unwrap();
            let moved = root.clone();
            let c = control();
            let cancel = c.clone();
            reader::BEFORE_OPEN_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || match kind {
                    "leaf" => {
                        fs::rename(moved.join("nested/source.rs"), moved.join("saved.rs")).unwrap();
                        symlink("../saved.rs", moved.join("nested/source.rs")).unwrap();
                    }
                    "parent" => {
                        fs::rename(moved.join("nested"), moved.join("saved")).unwrap();
                        symlink("saved", moved.join("nested")).unwrap();
                    }
                    "root" => {
                        let old = moved.with_extension("old");
                        fs::rename(&moved, &old).unwrap();
                        symlink(old, &moved).unwrap();
                    }
                    "cancel" => cancel.cancel(),
                    _ => unreachable!(),
                }))
            });
            let result = reader.read("nested/source.rs", 100, &c);
            reader::BEFORE_OPEN_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert_eq!(
                result.unwrap_err().code,
                if kind == "cancel" {
                    "CANCELLED"
                } else {
                    "PATH_REFUSED"
                },
                "{kind}"
            );
        }
    }

    #[test]
    fn empty_session_still_binds_the_discovery_root_identity() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("repo");
        fs::create_dir(&root).unwrap();
        let mut reader = ConfinedReader::new(&root).unwrap();
        reader.validate(Some(&control())).unwrap();
        fs::rename(&root, base.join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        assert_eq!(
            reader.validate(Some(&control())).unwrap_err().code,
            "PATH_REFUSED"
        );
        assert_eq!(reader.stats().file_opens, 0);
    }

    #[test]
    fn root_replacement_never_returns_cached_old_or_new_root_bytes() {
        for link in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("repo");
            fs::create_dir(&root).unwrap();
            fs::write(root.join("source.rs"), "old").unwrap();
            let mut reader = ConfinedReader::new(&root).unwrap();
            assert_eq!(reader.read("source.rs", 100, &control()).unwrap(), "old");
            fs::rename(&root, base.join("old")).unwrap();
            if link {
                symlink(base.join("old"), &root).unwrap();
            } else {
                fs::create_dir(&root).unwrap();
                fs::write(root.join("source.rs"), "new").unwrap();
            }
            assert_eq!(
                reader
                    .read_resolved(&root.join("source.rs"), 100, Some(&control()))
                    .unwrap_err()
                    .code,
                "PATH_REFUSED"
            );
        }
    }

    #[test]
    fn real_post_read_leaf_parent_root_swaps_are_refused_before_returning_bytes() {
        for kind in ["leaf", "parent", "root", "deep-parent"] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("repo");
            let relative = if kind == "deep-parent" {
                format!("{}source.rs", "d/".repeat(35))
            } else {
                "nested/source.rs".into()
            };
            fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
            fs::write(root.join(&relative), "safe source").unwrap();
            let mut reader = ConfinedReader::new(&root).unwrap();
            reader.read(&relative, 100, &control()).unwrap();
            let moved_root = root.clone();
            let leaf = root.join(&relative);
            let mut once = true;
            confined::READ_HOOK.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |_| {
                    if !once {
                        return;
                    }
                    once = false;
                    match kind {
                        "leaf" => {
                            fs::rename(&leaf, moved_root.join("saved.rs")).unwrap();
                            symlink("saved.rs", &leaf).unwrap();
                        }
                        "parent" | "deep-parent" => {
                            let parent = leaf.parent().unwrap();
                            fs::rename(parent, moved_root.join("saved-parent")).unwrap();
                            fs::create_dir(parent).unwrap();
                            fs::rename(moved_root.join("saved-parent/source.rs"), &leaf).unwrap();
                        }
                        "root" => {
                            let old = moved_root.with_extension("old");
                            fs::rename(&moved_root, &old).unwrap();
                            symlink(old, &moved_root).unwrap();
                        }
                        _ => unreachable!(),
                    }
                }))
            });
            let result = reader.read(&relative, 100, &control());
            confined::READ_HOOK.with(|h| *h.borrow_mut() = None);
            assert_eq!(result.unwrap_err().code, "PATH_REFUSED", "{kind}");
        }
    }
}
