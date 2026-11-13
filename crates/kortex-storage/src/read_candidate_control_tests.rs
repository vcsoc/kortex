use super::*;
use std::{cell::RefCell, time::Duration};

struct ScanHook {
    seen: usize,
    cancel_at: usize,
    control: RequestControl,
}
thread_local! {
    static SCAN: RefCell<Option<ScanHook>> = const { RefCell::new(None) };
}
pub(super) fn row_checkpoint() {
    SCAN.with_borrow_mut(|hook| {
        if let Some(hook) = hook {
            hook.seen += 1;
            assert!(
                hook.seen <= hook.cancel_at,
                "scan processed rows after cancellation"
            );
            if hook.seen == hook.cancel_at {
                hook.control.cancel();
            }
        }
    });
}
struct ResetHook;
impl Drop for ResetHook {
    fn drop(&mut self) {
        SCAN.set(None);
    }
}
fn fixture() -> Snapshot {
    let mut snapshot = crate::read_tests::fixture("controlled");
    let template = snapshot.files[0].symbols[0].clone();
    snapshot.files[0].symbols = (0..256)
        .map(|i| {
            let mut symbol = template.clone();
            symbol.symbol_id = format!("controlled{i:04}");
            symbol.name = format!("item{i:04}");
            symbol.qualified_name = format!("module.{}", symbol.name);
            symbol
        })
        .collect();
    snapshot.edges.clear();
    snapshot
}

#[test]
fn cancellation_during_actual_projected_and_legacy_scans_stops_before_remaining_rows() {
    for legacy in [false, true] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture(), &lock).unwrap();
        if legacy {
            writer.conn.execute_batch("DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_edge_order; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order").unwrap();
        } else {
            // This later row would fail decoding if the scan continued past cancellation.
            writer
                .conn
                .execute_batch(
                    "UPDATE nodes SET name=CAST(x'ff' AS TEXT) WHERE id='controlled0030'",
                )
                .unwrap();
        }
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        let control = RequestControl::new(Duration::from_secs(60)).unwrap();
        SCAN.set(Some(ScanHook {
            seen: 0,
            cancel_at: 17,
            control: control.clone(),
        }));
        let reset = ResetHook;
        let result = session
            .query_candidate_inventory_with_control("item", &Default::default(), &control)
            .unwrap_err();
        assert_eq!((result.code, result.exit), ("CANCELLED", 9));
        SCAN.with_borrow(|hook| assert_eq!(hook.as_ref().unwrap().seen, 17));
        drop(reset);
        assert!(control.is_cancelled());
        let independent = RequestControl::new(Duration::from_secs(60)).unwrap();
        let resumed = session.query_candidate_inventory_with_control(
            "item",
            &Default::default(),
            &independent,
        );
        let uncontrolled = session.query_candidate_inventory("item", &Default::default());
        if legacy {
            let actual = resumed.unwrap();
            assert_eq!(actual.items.len(), 256);
            assert_eq!(actual.items, uncontrolled.unwrap().items);
            assert_eq!(actual.source, MetadataSource::LegacySnapshot);
        } else {
            assert_eq!(resumed.unwrap_err().code, "CORRUPT_INDEX");
            assert_eq!(uncontrolled.unwrap_err().code, "CORRUPT_INDEX");
        }
    }
}

#[test]
fn caller_cancel_deadline_and_uncontrolled_compatibility_are_preserved() {
    let d = crate::test_support::tempdir().unwrap();
    let mut writer = Store::open(d.path(), true).unwrap();
    let lock = writer.writer_lock().unwrap();
    writer.commit(&fixture(), &lock).unwrap();
    let reader = Store::open(d.path(), false).unwrap();
    let session = reader.read_session().unwrap();
    let cancelled = RequestControl::new(Duration::from_secs(60)).unwrap();
    cancelled.cancel();
    assert_eq!(
        session
            .query_candidate_inventory_with_control("item", &Default::default(), &cancelled)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert!(reader.projection_source.get().is_none());
    let expired = RequestControl::from_deadline(std::time::Instant::now());
    assert_eq!(
        session
            .query_candidate_inventory_with_control("item", &Default::default(), &expired)
            .unwrap_err()
            .code,
        "TIMEOUT"
    );
    assert!(reader.projection_source.get().is_none());
    let control = RequestControl::new(Duration::from_secs(60)).unwrap();
    let controlled = session
        .query_candidate_inventory_with_control("item", &Default::default(), &control)
        .unwrap();
    let uncontrolled = session
        .query_candidate_inventory("item", &Default::default())
        .unwrap();
    assert_eq!(controlled.items, uncontrolled.items);
    assert_eq!(controlled.fts, uncontrolled.fts);
    assert_eq!(controlled.source, uncontrolled.source);
}

#[test]
fn query_scope_cancellation_stops_at_row17_for_fast_old_writer_and_legacy() {
    for mode in ["fast", "old-writer", "legacy"] {
        let d = crate::test_support::tempdir().unwrap();
        let mut writer = Store::open(d.path(), true).unwrap();
        let lock = writer.writer_lock().unwrap();
        writer.commit(&fixture(), &lock).unwrap();
        if mode != "fast" {
            writer
                .conn
                .execute_batch(
                    "DROP TABLE query_symbol_inventory; DROP TABLE generation_query_manifest",
                )
                .unwrap();
        }
        if mode == "legacy" {
            writer.conn.execute_batch("DROP TABLE generation_read_manifest; DROP TABLE read_symbol_inventory; DROP TABLE read_edge_order; DROP TABLE read_file_diagnostics; DROP TABLE read_file_order").unwrap();
        } else {
            let table = if mode == "fast" {
                "query_symbol_inventory"
            } else {
                "nodes"
            };
            writer
                .conn
                .execute_batch(&format!(
                    "UPDATE {table} SET name=CAST(x'ff' AS TEXT) WHERE id='controlled0030'"
                ))
                .unwrap();
        }
        let reader = Store::open(d.path(), false).unwrap();
        let session = reader.read_session().unwrap();
        let cancelled = RequestControl::new(Duration::from_secs(60)).unwrap();
        cancelled.cancel();
        assert_eq!(
            session
                .query_scoped_candidate_inventory_with_control(
                    "item",
                    &Default::default(),
                    &cancelled
                )
                .unwrap_err()
                .code,
            "CANCELLED"
        );
        assert!(reader.query_projection.get().is_none());
        let expired = RequestControl::from_deadline(std::time::Instant::now());
        assert_eq!(
            session
                .query_scoped_candidate_inventory_with_control(
                    "item",
                    &Default::default(),
                    &expired
                )
                .unwrap_err()
                .code,
            "TIMEOUT"
        );
        assert!(reader.query_projection.get().is_none());
        let control = RequestControl::new(Duration::from_secs(60)).unwrap();
        SCAN.set(Some(ScanHook {
            seen: 0,
            cancel_at: 17,
            control: control.clone(),
        }));
        let reset = ResetHook;
        let error = session
            .query_scoped_candidate_inventory_with_control("item", &Default::default(), &control)
            .unwrap_err();
        assert_eq!((error.code, error.exit), ("CANCELLED", 9));
        SCAN.with_borrow(|hook| assert_eq!(hook.as_ref().unwrap().seen, 17));
        drop(reset);
        let independent = RequestControl::new(Duration::from_secs(60)).unwrap();
        let controlled = session.query_scoped_candidate_inventory_with_control(
            "item",
            &Default::default(),
            &independent,
        );
        let uncontrolled = session.query_scoped_candidate_inventory("item", &Default::default());
        if mode == "legacy" {
            let actual = controlled.unwrap();
            assert_eq!(actual.items.len(), 256);
            assert_eq!(actual.items, uncontrolled.unwrap().items);
            assert_eq!(actual.source, MetadataSource::LegacySnapshot);
        } else {
            assert_eq!(controlled.unwrap_err().code, "CORRUPT_INDEX");
            assert_eq!(uncontrolled.unwrap_err().code, "CORRUPT_INDEX");
        }
        assert!(reader.projection_source.get().is_none());
    }
}
