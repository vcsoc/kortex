use super::*;
use std::{fs, time::Duration};
fn control() -> RequestControl {
    RequestControl::new(Duration::from_secs(60)).unwrap()
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("source.rs"), "fn one() {}\n").unwrap();
    (temp, root)
}
fn stamp(root: &Path) -> SourceStampV1 {
    read_source_stamped_v1(root, "source.rs", 1024, &control())
        .unwrap()
        .into_parts()
        .1
}
#[test]
fn captured_identity_and_hash_belong_to_the_actual_read() {
    use std::os::unix::fs::MetadataExt;
    let (_temp, root) = fixture();
    let read = read_source_stamped_v1(&root, "source.rs", 1024, &control()).unwrap();
    let metadata = fs::metadata(root.join("source.rs")).unwrap();
    assert_eq!(read.stamp().metadata().inode, metadata.ino());
    assert_eq!(read.stamp().metadata().device, metadata.dev());
    assert_eq!(read.stamp().metadata().size, read.source().len() as u64);
    assert_eq!(
        read.stamp().content_hash(),
        kortex_core::hash(read.source())
    );
    assert_eq!(read.stamp().version(), 1);
    assert!(!read.stamp().permits_metadata_reuse());
    let check = verify_source_v1(&root, read.stamp(), 1024, &control()).unwrap();
    assert_eq!(
        check,
        SourceVerificationV1 {
            content_matches: true,
            identity_matches: true,
            metadata_matches: true,
            hash_reads: 1
        }
    );
}
#[test]
fn restored_mtime_same_size_edits_still_rehash_and_do_not_report_equal() {
    let (_temp, root) = fixture();
    let previous = stamp(&root);
    let path = root.join("source.rs");
    let original = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "fn two() {}\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().len(), previous.metadata.size);
    let check = verify_source_v1(&root, &previous, 1024, &control()).unwrap();
    assert!(!check.content_matches);
    assert_eq!(check.hash_reads, 1);
    assert!(check.identity_matches);
}
#[test]
fn rapid_edits_never_reuse_equal_timestamp_guesses() {
    let (_temp, root) = fixture();
    for index in 0..64 {
        let previous = stamp(&root);
        fs::write(root.join("source.rs"), format!("fn x{index:02}() {{}}\n")).unwrap();
        let check = verify_source_v1(&root, &previous, 1024, &control()).unwrap();
        assert!(!check.content_matches);
        assert_eq!(check.hash_reads, 1);
    }
}
#[test]
fn atomic_replacement_with_equal_bytes_changes_identity() {
    let (_temp, root) = fixture();
    let previous = stamp(&root);
    fs::write(root.join("replacement"), "fn one() {}\n").unwrap();
    fs::rename(root.join("replacement"), root.join("source.rs")).unwrap();
    let check = verify_source_v1(&root, &previous, 1024, &control()).unwrap();
    assert!(check.content_matches);
    assert!(!check.identity_matches);
    assert_eq!(check.hash_reads, 1);
}
#[test]
fn parent_swap_is_detected_even_when_the_original_file_is_moved_back() {
    let (_temp, root) = fixture();
    fs::create_dir(root.join("parent")).unwrap();
    fs::rename(root.join("source.rs"), root.join("parent/source.rs")).unwrap();
    let previous = read_source_stamped_v1(&root, "parent/source.rs", 1024, &control())
        .unwrap()
        .into_parts()
        .1;
    fs::rename(root.join("parent"), root.join("old")).unwrap();
    fs::create_dir(root.join("parent")).unwrap();
    fs::rename(root.join("old/source.rs"), root.join("parent/source.rs")).unwrap();
    let check = verify_source_v1(&root, &previous, 1024, &control()).unwrap();
    assert!(check.content_matches);
    assert!(!check.identity_matches);
}
#[test]
fn swaps_and_writes_around_actual_hash_reads_report_source_changed() {
    for mutation in ["write", "leaf", "parent", "root"] {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().canonicalize().unwrap().join("root");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        fs::write(root.join("parent/source.rs"), "fn one() {}\n").unwrap();
        let result = read_source_impl(
            &root,
            "parent/source.rs",
            1024,
            &control(),
            || match mutation {
                "write" => fs::write(root.join("parent/source.rs"), "fn two() {}\n").unwrap(),
                "leaf" => {
                    fs::write(root.join("replacement"), "fn one() {}\n").unwrap();
                    fs::rename(root.join("replacement"), root.join("parent/source.rs")).unwrap();
                }
                "parent" => {
                    fs::rename(root.join("parent"), root.join("old")).unwrap();
                    fs::create_dir(root.join("parent")).unwrap();
                    fs::write(root.join("parent/source.rs"), "fn one() {}\n").unwrap();
                }
                "root" => {
                    fs::rename(&root, outer.path().join("old")).unwrap();
                    fs::create_dir(&root).unwrap();
                }
                _ => unreachable!(),
            },
        );
        assert_eq!(result.unwrap_err().code, "SOURCE_CHANGED", "{mutation}");
    }
}
#[test]
fn repeated_same_size_writes_are_detected_without_timestamp_resolution_assumptions() {
    let (_temp, root) = fixture();
    let source = root.join("source.rs");
    let mut unchanged_metadata = 0;
    for _ in 0..64 {
        fs::write(&source, "fn one() {}\n").unwrap();
        let before = MetadataV1::capture(&source.metadata().unwrap());
        let result = read_source_impl(&root, "source.rs", 1024, &control(), || {
            fs::write(&source, "fn two() {}\n").unwrap();
            if MetadataV1::capture(&source.metadata().unwrap()) == before {
                unchanged_metadata += 1;
            }
        });
        assert_eq!(result.unwrap_err().code, "SOURCE_CHANGED");
        assert_eq!(fs::read_to_string(&source).unwrap(), "fn two() {}\n");
    }
    eprintln!("64 actual same-size mutations; unchanged metadata: {unchanged_metadata}");
}
#[test]
fn stamped_reads_preserve_confinement_limits_and_text_errors() {
    let (_temp, root) = fixture();
    for relative in ["../source.rs", "/source.rs"] {
        assert_eq!(
            read_source_stamped_v1(&root, relative, 1024, &control())
                .unwrap_err()
                .code,
            "PATH_REFUSED"
        );
    }
    assert_eq!(
        read_source_stamped_v1(&root, "source.rs", 1, &control())
            .unwrap_err()
            .code,
        "FILE_TOO_LARGE"
    );
    fs::write(root.join("source.rs"), b"\0").unwrap();
    assert_eq!(
        read_source_stamped_v1(&root, "source.rs", 100, &control())
            .unwrap_err()
            .code,
        "BINARY_FILE"
    );
    fs::write(root.join("source.rs"), b"\xff").unwrap();
    assert_eq!(
        read_source_stamped_v1(&root, "source.rs", 100, &control())
            .unwrap_err()
            .code,
        "INVALID_ENCODING"
    );
}
#[test]
fn inventory_rehash_detects_rapid_edits_new_delete_ignore_and_manifests() {
    for mutation in [
        "edit",
        "new",
        "delete",
        "ignore",
        "manifest",
        "nested-ignore",
        "nested-manifest",
    ] {
        let (_temp, root) = fixture();
        fs::create_dir(root.join("nested")).unwrap();
        let config = Config::default();
        let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
        assert_eq!(
            verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
            InventoryFreshnessV1::UnchangedByHash
        );
        match mutation {
            "edit" => fs::write(root.join("source.rs"), "fn two() {}\n").unwrap(),
            "new" => fs::write(root.join("new.rs"), "fn new() {}\n").unwrap(),
            "delete" => fs::remove_file(root.join("source.rs")).unwrap(),
            "ignore" => fs::write(root.join(".kortexignore"), "source.rs\n").unwrap(),
            "manifest" => fs::write(root.join("Cargo.toml"), "[package]\nname='new'\n").unwrap(),
            "nested-ignore" => fs::write(root.join("nested/.ignore"), "anything\n").unwrap(),
            "nested-manifest" => fs::write(root.join("nested/package.json"), "{}\n").unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
            InventoryFreshnessV1::Changed,
            "{mutation}"
        );
    }
}
#[test]
fn truncated_or_failed_inventories_are_uncertain_not_fresh() {
    let (_temp, root) = fixture();
    fs::write(root.join("another.rs"), "fn another() {}\n").unwrap();
    let mut config = Config::default();
    config.repository.max_files = 1;
    let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
    assert_eq!(
        verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
        InventoryFreshnessV1::Uncertain
    );
}
#[test]
fn root_replacement_is_detected_for_nonempty_and_empty_inventories() {
    for populated in [false, true] {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().canonicalize().unwrap().join("root");
        fs::create_dir(&root).unwrap();
        if populated {
            fs::write(root.join("source.rs"), "fn one() {}\n").unwrap();
        }
        let config = Config::default();
        let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
        fs::rename(&root, outer.path().join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        if populated {
            fs::write(root.join("source.rs"), "fn one() {}\n").unwrap();
        }
        assert_eq!(
            verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
            InventoryFreshnessV1::Changed
        );
    }
}

#[test]
fn supplied_effective_config_and_restored_mtime_manifest_edits_are_hashed() {
    let (_temp, root) = fixture();
    let manifest = root.join("Cargo.toml");
    fs::write(&manifest, "[package]\nname='one'\n").unwrap();
    let config = Config::default();
    let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
    let original = fs::metadata(&manifest).unwrap().modified().unwrap();
    fs::write(&manifest, "[package]\nname='two'\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&manifest)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original))
        .unwrap();
    assert_eq!(
        verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
        InventoryFreshnessV1::Changed
    );
    let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
    let mut different = config.clone();
    different.repository.include_hidden = !config.repository.include_hidden;
    assert_eq!(
        verify_inventory_v1(&root, &different, &baseline, &control()).unwrap(),
        InventoryFreshnessV1::Changed
    );
}

#[test]
fn unknown_timestamp_semantics_always_hash_without_an_fs_allowlist() {
    let (_temp, root) = fixture();
    let previous = stamp(&root);
    // The real filesystem is not reclassified as coarse/network by this test.
    // There is no metadata-reuse branch for ANY filesystem, even our native one.
    for _ in 0..3 {
        assert!(!previous.permits_metadata_reuse());
        assert_eq!(
            verify_source_v1(&root, &previous, 1024, &control())
                .unwrap()
                .hash_reads,
            1
        );
    }
}

#[test]
fn ancillary_inputs_preserve_literal_backslashes_without_aliasing() {
    let (_temp, root) = fixture();
    fs::create_dir(root.join(r"literal\name")).unwrap();
    fs::create_dir_all(root.join("literal/name")).unwrap();
    fs::write(
        root.join(r"literal\name/Cargo.toml"),
        "[package]\nname='one'\n",
    )
    .unwrap();
    fs::write(
        root.join("literal/name/Cargo.toml"),
        "[package]\nname='two'\n",
    )
    .unwrap();
    let config = Config::default();
    let baseline = capture_inventory_v1(&root, &config, &control()).unwrap();
    assert_eq!(baseline.nested_inputs.len(), 2);
    assert!(
        baseline
            .nested_inputs
            .contains_key(r"literal\name/Cargo.toml")
    );
    assert!(
        baseline
            .nested_inputs
            .contains_key("literal/name/Cargo.toml")
    );
    fs::write(
        root.join(r"literal\name/Cargo.toml"),
        "[package]\nname='new'\n",
    )
    .unwrap();
    assert_eq!(
        verify_inventory_v1(&root, &config, &baseline, &control()).unwrap(),
        InventoryFreshnessV1::Changed
    );
}

#[test]
fn unproved_directory_alias_input_coverage_reports_uncertainty() {
    let (_temp, root) = fixture();
    let mut config = Config::default();
    config.repository.follow_symlinks = true;
    assert_eq!(
        capture_inventory_v1(&root, &config, &control())
            .unwrap_err()
            .code,
        "SOURCE_STAMP_UNCERTAIN"
    );
    // Existing default/control-aware discovery support remains intact.
    assert_eq!(
        crate::discover_with_control(&root, &config, &control())
            .unwrap()
            .files
            .len(),
        1
    );
}

#[test]
fn source_stamp_checks_cancellation_after_an_actual_hash_read() {
    let (_temp, root) = fixture();
    let control = control();
    let result = read_source_impl(&root, "source.rs", 1024, &control, || control.cancel());
    assert_eq!(result.unwrap_err().code, "CANCELLED");
}
