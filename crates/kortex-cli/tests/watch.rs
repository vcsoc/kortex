mod support;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn watcher_reconciles_atomic_saves_and_rename_then_survives_kill() {
    let d = support::tempdir().unwrap();
    let root = d.path();
    std::fs::write(root.join("a.py"), "def original(): pass\n").unwrap();
    let binary = env!("CARGO_BIN_EXE_kortex");
    let mut guard = Guard(
        Command::new(binary)
            .args(["watch", ".", "--debounce-ms", "50", "--json"])
            .current_dir(root)
            .env("KORTEX_REGISTRY", root.join("registry.db"))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let out = guard.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let ready: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
    assert!(ready["watching"].is_string());
    std::fs::write(root.join("save.tmp"), "def changed(): pass\n").unwrap();
    std::fs::rename(root.join("save.tmp"), root.join("renamed.py")).unwrap();
    std::fs::remove_file(root.join("a.py")).unwrap();
    let update: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
    assert!(update["generation_id"].is_string());
    guard.0.kill().unwrap();
    guard.0.wait().unwrap();
    reader.join().unwrap();
    let output = Command::new(binary)
        .args(["context", "changed", "--json"])
        .current_dir(root)
        .env("KORTEX_REGISTRY", root.join("registry.db"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["symbol"]["file_path"], "renamed.py");
}
