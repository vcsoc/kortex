use super::*;
use crate::read_tests::fixture;

#[test]
fn validation_cache_never_crosses_roots_same_generation_ids_or_writable_transactions() {
    let first = crate::test_support::tempdir().unwrap();
    let second = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(first.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    let mut other = Store::open(second.path(), true).unwrap();
    let other_lock = other.writer_lock().unwrap();
    writer
        .commit(&fixture("identical-generation-id"), &lock)
        .unwrap();
    other
        .commit(&fixture("identical-generation-id"), &other_lock)
        .unwrap();
    let reader = Store::open(first.path(), false).unwrap();
    reader
        .read_session()
        .unwrap()
        .read_projection_source()
        .unwrap();
    assert_eq!(
        reader.projection_source.get(),
        Some(&MetadataSource::Projection)
    );
    other
        .conn
        .execute_batch("DELETE FROM read_symbol_inventory WHERE ordinal=1")
        .unwrap();
    let unrelated = Store::open(second.path(), false).unwrap();
    assert_eq!(
        unrelated
            .read_session()
            .unwrap()
            .read_projection_source()
            .unwrap_err()
            .code,
        "CORRUPT_INDEX"
    );
    assert!(unrelated.projection_source.get().is_none());
    assert!(writer.projection_source.get().is_none());
    assert_eq!(
        writer.read_session().err().unwrap().code,
        "PINNED_READ_REQUIRED"
    );
    // Even an explicit read transaction on a writable connection is NOT immutable.
    writer.conn.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        writer.read_session().err().unwrap().code,
        "PINNED_READ_REQUIRED"
    );
    assert!(writer.projection_source.get().is_none());
    writer.conn.execute_batch("ROLLBACK").unwrap();
    writer.commit(&fixture("replacement"), &lock).unwrap();
    assert!(writer.projection_source.get().is_none());
    assert_eq!(
        reader.read_session().unwrap().metadata().generation_id,
        "identical-generation-id"
    );
    assert_eq!(
        reader
            .read_session()
            .unwrap()
            .symbol_inventory()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn all_order_projections_validate_actual_duplicates_types_gaps_and_orphan_identities() {
    for table in [
        "read_symbol_inventory",
        "read_edge_order",
        "read_file_order",
    ] {
        for ordinals in [
            "0,0,2",
            "0,0.5,2",
            "0,NULL,2",
            "0,1,3",
            "0,'1',2",
            "0,x'31',2",
            "-1,1,2",
        ] {
            let d = crate::test_support::tempdir().unwrap();
            let mut writer = Store::open(d.path(), true).unwrap();
            let lock = writer.writer_lock().unwrap();
            writer.commit(&fixture("old"), &lock).unwrap();
            // Remove constraints only in this malformed fixture: validation must still refuse.
            writer.conn.execute_batch(&format!("CREATE TABLE replacement AS SELECT * FROM {table}; DROP TABLE {table}; ALTER TABLE replacement RENAME TO {table}")).unwrap();
            // CTAS preserves INTEGER affinity, so use a no-affinity ordinal for type fixtures.
            writer.conn.execute_batch(&format!("ALTER TABLE {table} RENAME COLUMN ordinal TO original_ordinal; ALTER TABLE {table} ADD COLUMN ordinal")).unwrap();
            for (ordinal, value) in ordinals.split(',').enumerate() {
                writer
                    .conn
                    .execute_batch(&format!(
                        "UPDATE {table} SET ordinal={value} WHERE original_ordinal={ordinal}"
                    ))
                    .unwrap();
            }
            let reader = Store::open(d.path(), false).unwrap();
            assert_eq!(
                reader
                    .read_session()
                    .unwrap()
                    .read_projection_source()
                    .unwrap_err()
                    .code,
                "CORRUPT_INDEX",
                "{table} {ordinals}"
            );
            assert!(reader.projection_source.get().is_none());
        }
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture("old"), &lock).unwrap();
        writer
            .conn
            .execute_batch(&format!(
                "PRAGMA foreign_keys=OFF; UPDATE {table} SET id='orphan' WHERE ordinal=1"
            ))
            .unwrap();
        let reader = Store::open(d.path(), false).unwrap();
        assert_eq!(
            reader
                .read_session()
                .unwrap()
                .query_candidate_inventory("old", &Default::default())
                .unwrap_err()
                .code,
            "CORRUPT_INDEX",
            "{table}"
        );
        assert!(reader.projection_source.get().is_none());
    }
}

#[test]
fn cached_legacy_absence_is_not_reused_by_a_separate_current_store() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture("legacy"), &lock).unwrap();
    writer.conn.execute_batch("DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_edge_order; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order").unwrap();
    let old = Store::open(d.path(), false).unwrap();
    assert_eq!(
        old.read_session()
            .unwrap()
            .read_projection_source()
            .unwrap(),
        MetadataSource::LegacySnapshot
    );
    assert_eq!(
        old.projection_source.get(),
        Some(&MetadataSource::LegacySnapshot)
    );
    writer.commit(&fixture("current"), &lock).unwrap();
    assert_eq!(
        old.read_session()
            .unwrap()
            .query_candidate_inventory("legacy", &Default::default())
            .unwrap()
            .source,
        MetadataSource::LegacySnapshot
    );
    let current = Store::open(d.path(), false).unwrap();
    assert!(current.projection_source.get().is_none());
    assert_eq!(
        current
            .read_session()
            .unwrap()
            .query_candidate_inventory("current", &Default::default())
            .unwrap()
            .source,
        MetadataSource::Projection
    );
}
