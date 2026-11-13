//! Standalone actual-parser proof, compiled explicitly with existing workspace rlibs.
//! Not an auto Cargo test/dependency: parent owns parser/indexer integration.
#[cfg(unix)]
fn main() {
    use kortex_config::Config;
    use kortex_core::{RequestControl, hash};
    use kortex_repo::{
        PolicyInputs, discover_authorized_profiled, discover_authorized_with_control,
    };
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, symlink},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    let base = std::env::temp_dir().join(format!(
        "kortex-authorized-sdk-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
    let root = base.join("repo");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    let source = "def safe():\n    return 1\n";
    fs::write(root.join("safe.py"), source).unwrap();
    let external = base.join("dummy-policy");
    fs::write(&external, "safe.py\n").unwrap();
    let control = RequestControl::new(Duration::from_secs(30)).unwrap();
    let inputs = PolicyInputs::root_only();
    let mut parsed = 0;
    for name in [".kortexignore", ".ignore", ".gitignore"] {
        for follow in [false, true] {
            let mut config = Config::default();
            config.repository.follow_symlinks = follow;
            fs::write(root.join(name), "safe.py\n").unwrap();
            assert!(
                discover_authorized_with_control(&root, &config, &inputs, &control)
                    .unwrap()
                    .files
                    .is_empty()
            );
            fs::remove_file(root.join(name)).unwrap();
            symlink(&external, root.join(name)).unwrap();
            for policy in ["safe.py\n", "unrelated.py\n"] {
                fs::write(&external, policy).unwrap();
                let error = discover_authorized_with_control(&root, &config, &inputs, &control)
                    .err()
                    .expect("outside alias must refuse");
                assert_eq!(error.code, "PATH_REFUSED");
            }
            fs::remove_file(root.join(name)).unwrap();
            let (inventory, stats, provenance) =
                discover_authorized_profiled(&root, &config, &inputs, &control).unwrap();
            assert_eq!(inventory.files.len(), 1);
            let file = &inventory.files[0];
            assert_eq!(file.source, source);
            assert_eq!(file.content_hash, hash(source.as_bytes()));
            assert_eq!(stats.reader.bytes_read, source.len() as u64);
            assert!(!provenance.digest.is_empty());
            // Real Tree-sitter Python parsing; fixture source is NEVER executed.
            let facts =
                kortex_parse::parse("authorized-sdk-proof", &file.path, "python", &file.source)
                    .unwrap();
            assert!(facts.symbols.iter().any(|symbol| symbol.name == "safe"));
            parsed += 1;
        }
    }
    fs::remove_dir_all(&base).unwrap();
    println!(
        "authorized_sdk_actual_parser: 6 regular exclusions; 12 outside-alias refusals; {parsed} real Python parses; source never executed"
    );
}
#[cfg(not(unix))]
fn main() {
    panic!("this standalone parser proof requires Unix symlink fixtures");
}
