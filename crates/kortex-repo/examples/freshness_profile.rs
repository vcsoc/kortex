//! Diagnostic only: inert generated source is read/hashed, never executed.
//! Run under /usr/bin/time -l on Darwin for whole-process peak RSS.
use kortex_core::RequestControl;
use kortex_repo::{ConfinedReadStats, ConfinedReader};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn add(total: &mut ConfinedReadStats, part: &ConfinedReadStats) {
    total.root_opens += part.root_opens;
    total.root_checks += part.root_checks;
    total.canonicalizations += part.canonicalizations;
    total.parent_opens += part.parent_opens;
    total.parent_checks += part.parent_checks;
    total.file_opens += part.file_opens;
    total.leaf_checks += part.leaf_checks;
    total.read_calls += part.read_calls;
    total.bytes_read += part.bytes_read;
}

fn read_phase(
    root: &Path,
    expected: &BTreeMap<String, String>,
    shared: bool,
    control: &RequestControl,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut stats = ConfinedReadStats::default();
    let mut reader = if shared {
        Some(ConfinedReader::new(root)?)
    } else {
        None
    };
    for (path, digest) in expected {
        control.check()?;
        if !shared {
            reader = Some(ConfinedReader::new(root)?);
        }
        let current = reader.as_mut().expect("a source reader was acquired");
        let source = current.read(path, 2 * 1024 * 1024, control)?;
        assert_eq!(&kortex_core::hash(&source), digest);
        if !shared {
            add(&mut stats, current.stats());
        }
    }
    if shared {
        add(&mut stats, reader.as_ref().unwrap().stats());
    }
    println!(
        "phase={} elapsed_ms={:.3} stats={stats:?}",
        if shared {
            "shared_session_read_hash"
        } else {
            "reacquired_session_read_hash"
        },
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let count: usize = args.get(1).map_or("10000", String::as_str).parse()?;
    let nested = args.get(2).is_some_and(|a| a == "nested");
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("repo");
    fs::create_dir(&root)?;
    let root = root.canonicalize()?;
    let mut fixture = blake3::Hasher::new();
    let mut fixture_bytes = Vec::new();
    let mut expected = BTreeMap::new();
    let mut source_bytes = 0usize;
    for index in 0..count {
        let name = if nested {
            format!("group_{:03}/source_{index:05}.rs", index / 100)
        } else {
            format!("source_{index:05}.rs")
        };
        let source =
            format!("pub fn source_{index:05}(value: usize) -> usize {{ value + {index} }}\n");
        fixture.update(name.as_bytes());
        fixture.update(source.as_bytes());
        fixture_bytes.extend_from_slice(name.as_bytes());
        fixture_bytes.extend_from_slice(source.as_bytes());
        source_bytes += source.len();
        fs::create_dir_all(root.join(&name).parent().unwrap())?;
        fs::write(root.join(&name), &source)?;
        expected.insert(name, kortex_core::hash(&source));
    }
    // Optional independent SHA-256 tool hashes exactly the same name+source byte
    // stream as BLAKE3. It does NOT execute any fixture source or repository tool.
    let digest_input = temp.path().join("fixture-bytes");
    fs::write(&digest_input, fixture_bytes)?;
    let sha256 = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(&digest_input)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "unavailable".into());
    println!(
        "DIAGNOSTIC_NOT_ACCEPTANCE files={count} layout={} source_bytes={source_bytes} fixture_blake3={} fixture_sha256={sha256} hash_framing=concatenated_name_then_source_in_generation_order",
        if nested { "nested100" } else { "flat" },
        fixture.finalize().to_hex()
    );
    let config = kortex_config::Config::default();
    for run in 0..5 {
        let control = RequestControl::new(Duration::from_secs(120))?;
        println!("run={run}");
        let start = Instant::now();
        let mut paths = Vec::new();
        for entry in ignore::WalkBuilder::new(&root)
            .sort_by_file_name(|a, b| a.cmp(b))
            .build()
        {
            control.check()?;
            let entry = entry?;
            if entry.file_type().is_some_and(|t| t.is_file()) {
                paths.push(entry.into_path());
            }
        }
        assert_eq!(paths.len(), count);
        println!(
            "phase=walk_fixture_only entries={} elapsed_ms={:.3}",
            paths.len(),
            start.elapsed().as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        for path in paths {
            control.check()?;
            assert!(path.canonicalize()?.starts_with(&root));
        }
        println!(
            "phase=canonicalize calls={count} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        let mut bytes = 0;
        for (path, digest) in &expected {
            let source =
                kortex_repo::read_confined_with_control(&root, path, 2 * 1024 * 1024, &control)?;
            bytes += source.len();
            assert_eq!(&kortex_core::hash(source), digest);
        }
        println!(
            "phase=legacy_standalone_read_hash successful_api_calls={count} bytes={bytes} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        // Same controls/postchecks in both variants, not a security-bypass benchmark.
        for shared in if run % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            read_phase(&root, &expected, shared, &control)?;
        }
        let start = Instant::now();
        let inventory = kortex_repo::discover_with_control(&root, &config, &control)?;
        let elapsed = start.elapsed();
        assert_eq!(inventory.files.len(), count);
        assert!(inventory.diagnostics.is_empty());
        for file in &inventory.files {
            assert_eq!(expected.get(&file.path), Some(&file.content_hash));
        }
        println!(
            "phase=production_discovery_read_hash files={count} bytes={source_bytes} elapsed_ms={:.3}",
            elapsed.as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        for file in &inventory.files {
            control.check()?;
            assert_eq!(kortex_core::hash(&file.source), file.content_hash);
        }
        println!(
            "phase=resident_rehash bytes={source_bytes} hash_calls={count} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
