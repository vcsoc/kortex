use super::*;
#[cfg(unix)]
#[test]
fn native_watch_does_not_follow_file_or_directory_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = crate::root(directory.path()).unwrap();
    let secret = outside.path().join("secret.py");
    std::fs::write(&secret, "SECRET = 1\n").unwrap();
    std::os::unix::fs::symlink(&secret, root.join("alias.py")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("aliasdir")).unwrap();
    let inside = root.join("inside.py");
    std::fs::write(&inside, "VALUE = 1\n").unwrap();
    let events = Events::new(&root).unwrap();
    std::fs::write(&inside, "VALUE = 2\n").unwrap();
    events
        .receiver
        .recv_timeout(Duration::from_secs(8))
        .expect("native in-root event missing");
    let mut quiet = false;
    for _ in 0..64 {
        if events
            .receiver
            .recv_timeout(Duration::from_millis(100))
            .is_err()
        {
            quiet = true;
            break;
        }
    }
    assert!(quiet, "native event queue never became quiet");
    std::fs::write(&secret, "SECRET = 2\n").unwrap();
    assert!(
        matches!(
            events.receiver.recv_timeout(Duration::from_millis(500)),
            Err(RecvTimeoutError::Timeout)
        ),
        "watch followed an outside symlink"
    );
    std::fs::write(&inside, "VALUE = 3\n").unwrap();
    events
        .receiver
        .recv_timeout(Duration::from_secs(8))
        .expect("confined watcher stopped");
}
#[test]
fn real_native_events_wake_and_reconcile() {
    let d = tempfile::tempdir().unwrap();
    let root = crate::root(d.path()).unwrap();
    let events = Events::new(&root).unwrap();
    std::fs::write(d.path().join("first.py"), "def first(): pass\n").unwrap();
    events
        .receiver
        .recv_timeout(Duration::from_secs(8))
        .expect("native filesystem event missing");
    std::fs::write(d.path().join("second.py"), "def second(): pass\n").unwrap();
    events.wait(Duration::from_millis(20)).unwrap();
    let inventory = crate::discover(&root, &kortex_config::Config::default()).unwrap();
    assert_eq!(inventory.files.len(), 2);
    std::fs::remove_file(d.path().join("first.py")).unwrap();
    events.wait(Duration::from_millis(20)).unwrap();
    assert_eq!(
        crate::discover(&root, &kortex_config::Config::default())
            .unwrap()
            .files
            .len(),
        1
    );
}
