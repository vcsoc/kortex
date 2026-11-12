use super::*;

#[test]
fn atomic_preservation_and_rollback_never_replace_recreated_evidence() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("index.db");
    let backup = d.path().join("backup.db");
    fs::write(&original, "original corruption bytes").unwrap();
    fs::write(&backup, "previous evidence").unwrap();
    assert!(preserve_move(&original, &backup).is_err());
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "original corruption bytes"
    );
    assert_eq!(fs::read_to_string(&backup).unwrap(), "previous evidence");
    fs::remove_file(&backup).unwrap();
    preserve_move(&original, &backup).unwrap();
    assert!(!original.exists());
    fs::write(&original, "new active evidence after concurrent recreation").unwrap();
    // This is the actual primitive used by repair's best-effort rollback.
    // A path appearing after the move must not be overwritten by rollback.
    assert!(preserve_move(&backup, &original).is_err());
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "new active evidence after concurrent recreation"
    );
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        "original corruption bytes"
    );
}

#[test]
fn oversized_sparse_wal_version_inspection_is_bounded_without_mutating_evidence() {
    use std::io::Read;
    let d = tempfile::tempdir().unwrap();
    let directory = d.path().join(".kortex");
    crate::test_support::directory(&directory).unwrap();
    crate::test_support::file(&directory.join("index.db"), b"original corruption bytes").unwrap();
    let mut header = [0u8; 32];
    header[..4].copy_from_slice(&0x377f0682u32.to_be_bytes());
    header[8..12].copy_from_slice(&512u32.to_be_bytes());
    let wal_path = directory.join("index.db-wal");
    let wal = crate::test_support::file(&wal_path, header).unwrap();
    // A sparse actual file, not a giant allocation: inspection must reject the
    // frame-count budget before traversing 1,000,001 attacker-controlled frames.
    let length = 32 + (512 + 24) * 1_000_001u64;
    wal.set_len(length).unwrap();
    drop(wal);
    assert_eq!(
        Store::repair(d.path(), "repository").unwrap_err().code,
        "REPAIR_REFUSED"
    );
    assert_eq!(
        fs::read_to_string(directory.join("index.db")).unwrap(),
        "original corruption bytes"
    );
    assert_eq!(fs::metadata(&wal_path).unwrap().len(), length);
    let mut after = [0u8; 32];
    fs::File::open(&wal_path)
        .unwrap()
        .read_exact(&mut after)
        .unwrap();
    assert_eq!(after, header);
    assert!(!fs::read_dir(&directory).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("repair-")
    }));
}
