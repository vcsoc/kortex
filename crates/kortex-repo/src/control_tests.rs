use super::*;
use std::time::{Duration, Instant};

#[test]
fn old_and_controlled_discovery_and_resolution_inputs_agree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("one.rs"), "fn one() {}\n").unwrap();
    fs::write(root.join("two.py"), "def two(): pass\n").unwrap();
    let control = RequestControl::new(Duration::from_secs(30)).unwrap();
    let old = discover(&root, &Config::default()).unwrap();
    let controlled = discover_with_control(&root, &Config::default(), &control).unwrap();
    assert_eq!(old.diagnostics, controlled.diagnostics);
    assert_eq!(
        old.files
            .iter()
            .map(|f| (&f.path, &f.source, &f.content_hash))
            .collect::<Vec<_>>(),
        controlled
            .files
            .iter()
            .map(|f| (&f.path, &f.source, &f.content_hash))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        inputs::resolution_fingerprint(&root),
        inputs::resolution_fingerprint_with_control(&root, &control).unwrap()
    );
}

#[test]
fn cancellation_and_deadline_propagate_instead_of_becoming_file_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("one.rs"), "fn one() {}\n").unwrap();
    for cancelled in [false, true] {
        let control = RequestControl::from_deadline(Instant::now());
        if cancelled {
            control.cancel();
        }
        let expected = if cancelled { "CANCELLED" } else { "TIMEOUT" };
        assert_eq!(
            discover_with_control(&root, &Config::default(), &control)
                .err()
                .unwrap()
                .code,
            expected
        );
        assert_eq!(
            read_confined_with_control(&root, "one.rs", 1024, &control)
                .unwrap_err()
                .code,
            expected
        );
        assert_eq!(
            inputs::resolution_fingerprint_with_control(&root, &control)
                .unwrap_err()
                .code,
            expected
        );
        assert_eq!(
            git_with_control(&root, &["status"], &control)
                .unwrap_err()
                .code,
            expected
        );
    }
}

#[test]
fn cancellation_after_a_real_read_chunk_stops_before_next_chunk_and_aborts_discovery() {
    use std::{cell::Cell, rc::Rc};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("one.rs"), vec![b'a'; 3 * 64 * 1024]).unwrap();
    for inventory in [false, true] {
        let control = RequestControl::new(Duration::from_secs(30)).unwrap();
        let cancellation = control.clone();
        let count = Rc::new(Cell::new(0usize));
        let observed = count.clone();
        confined::READ_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                observed.set(observed.get() + bytes);
                cancellation.cancel();
            }))
        });
        let result = if inventory {
            discover_with_control(&root, &Config::default(), &control).map(|_| ())
        } else {
            read_confined_bytes_with_control(&root, "one.rs", 1024 * 1024, &control).map(|_| ())
        };
        confined::READ_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert_eq!(
            count.get(),
            64 * 1024,
            "only the first real native chunk was read"
        );
    }
}

#[test]
fn actual_large_discovery_observes_a_short_shared_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for index in 0..2000 {
        fs::write(
            root.join(format!("source_{index}.rs")),
            "fn actual_source() {}\n",
        )
        .unwrap();
    }
    let control = RequestControl::new(Duration::from_millis(5)).unwrap();
    let result = discover_with_control(&root, &Config::default(), &control);
    assert_eq!(
        result
            .err()
            .expect("2000 real file opens must honor the 5ms request budget")
            .code,
        "TIMEOUT"
    );
}
