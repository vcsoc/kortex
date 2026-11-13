//! Full/incremental indexing with cached facts and atomic generation publication.
mod resources;
use kortex_config::Config;
use kortex_core::{Error, PARSER_VERSION, Result, Snapshot, id, public_path};
use kortex_storage::Store;
use rayon::prelude::*;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct Options {
    pub full: bool,
    pub dry_run: bool,
    pub verify: bool,
}
pub struct Report {
    pub snapshot: Snapshot,
    pub parsed: usize,
    pub reused: usize,
    pub committed: bool,
}
pub fn index(path: &Path, config: &Config, options: &Options) -> Result<Report> {
    config.validate()?;
    let base = kortex_repo::root(path)?;
    let root = config.resolved_root(&base)?;
    index_at_root(&root, config, options)
}
/// Index an already selected effective root. Does not expand to a Git ancestor or relocate twice.
pub fn index_at_root(path: &Path, config: &Config, options: &Options) -> Result<Report> {
    config.validate()?;
    let root = path.canonicalize()?;
    if !root.is_dir() || root.to_str().is_none() {
        return Err(Error::new(
            "PATH_REFUSED",
            "effective root must be a UTF-8 directory",
            8,
        ));
    }
    let directory = Store::directory_for(&root, &config.index.storage)?;
    let mut store = if !options.dry_run || directory.join("index.db").exists() {
        Some(Store::open_with_storage(
            &root,
            &config.index.storage,
            !options.dry_run,
        )?)
    } else {
        None
    };
    let lock = if options.dry_run {
        None
    } else {
        Some(
            store
                .as_ref()
                .ok_or_else(|| Error::new("STORAGE_ERROR", "missing store", 6))?
                .writer_lock()?,
        )
    };
    let old = store.as_ref().and_then(|s| s.load().ok());
    let config_hash = config.fingerprint()?;
    let repo_id = id(&["repository", &public_path(&root)]);
    let inventory = kortex_repo::discover(&root, config)?;
    let cached: std::collections::BTreeMap<_, _> = old
        .as_ref()
        .filter(|o| {
            !options.full
                && config.index.mode != "full"
                && o.config_hash == config_hash
                && o.grammar_hash == PARSER_VERSION
                && o.repo_id == repo_id
        })
        .into_iter()
        .flat_map(|o| o.files.iter().map(|f| (f.path.as_str(), f)))
        .collect();
    let cpus = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8);
    let workers = if config.index.workers == 0 {
        cpus
    } else {
        config.index.workers.min(cpus).max(1)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        // Recursive emitters run only after AST depth admission, on a declared stack.
        .stack_size(8 * 1024 * 1024)
        .num_threads(workers)
        .build()
        .map_err(|e| Error::new("WORKER_POOL", e.to_string(), 1))?;
    let admission = resources::Admission::new(&config.index.resources);
    let output_limits = admission.output_limits();
    let parse_limits = kortex_parse::ParseLimits {
        max_ast_nodes: config.index.resources.max_file_ast_nodes,
        max_ast_depth: config.index.resources.max_file_ast_depth,
    };
    let facts: Vec<_> = pool.install(|| {
        inventory
            .files
            .par_iter()
            .map(|file| -> Result<_> {
                if let Some(f) = cached
                    .get(file.path.as_str())
                    .filter(|f| f.content_hash == file.content_hash)
                {
                    admission.admit(f)?;
                    return Ok(((*f).clone(), true));
                }
                let parsed = kortex_parse::parse_bounded_with_output_limits(
                    &repo_id,
                    &file.path,
                    kortex_repo::language(&file.path)
                        .ok_or_else(|| Error::new("UNSUPPORTED_LANGUAGE", &file.path, 7))?,
                    &file.source,
                    &parse_limits,
                    &output_limits,
                )?;
                admission.admit(&parsed)?;
                Ok((parsed, false))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    let reused = facts.iter().filter(|(_, reused)| *reused).count();
    let parsed = facts.len() - reused;
    let files = facts.into_iter().map(|(f, _)| f).collect();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::new("CLOCK_ERROR", e.to_string(), 1))?
        .as_nanos()
        .to_string();
    let mut snapshot = Snapshot {
        repo_id,
        root: public_path(&root),
        generation_id: id(&["generation", &timestamp, &std::process::id().to_string()]),
        git_head: kortex_repo::head(&root),
        config_hash,
        grammar_hash: PARSER_VERSION.into(),
        resolution_hash: kortex_repo::inputs::resolution_fingerprint(&root),
        files,
        edges: vec![],
        diagnostics: inventory.diagnostics,
    };
    let go_manifest =
        kortex_repo::read_confined(&root, "go.mod", config.repository.max_file_size_bytes)
            .unwrap_or_default();
    let go_module = go_manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("module "))
        .map(|m| m.trim().trim_matches('"'));
    let mut context = kortex_languages::ResolutionContext {
        go_module: go_module.map(str::to_owned),
        ..Default::default()
    };
    for name in ["tsconfig.json", "jsconfig.json"] {
        if let Ok(source) =
            kortex_repo::read_confined(&root, name, config.repository.max_file_size_bytes)
        {
            match context.clone().with_tsconfig(&source) {
                Ok(parsed) => context = parsed,
                Err(error) => snapshot
                    .diagnostics
                    .push(format!("{name}: alias configuration ignored: {error}")),
            }
            break;
        }
    }
    // Safe fallback: cached immutable parse facts are reused, but ALL resolution is rebuilt.
    // No dependent graph is retained, so manifest/import/export edits cannot leave stale edges.
    let limits = &config.index.resources;
    snapshot.edges = kortex_languages::resolve_bounded_with_context(
        &snapshot,
        &context,
        &kortex_languages::ResolutionLimits {
            max_edges: limits.max_edges,
            max_candidate_memberships: limits.max_candidate_memberships,
            max_edge_bytes: limits.max_edge_bytes,
            max_imports: limits.max_imports,
            max_fact_bytes: limits.max_parsed_bytes,
        },
    )?;
    if options.verify {
        let fresh = kortex_repo::discover(&root, config)?;
        let before: Vec<_> = snapshot
            .files
            .iter()
            .map(|f| (&f.path, &f.content_hash))
            .collect();
        let after: Vec<_> = fresh
            .files
            .iter()
            .map(|f| (&f.path, &f.content_hash))
            .collect();
        if before != after {
            return Err(Error::new(
                "FILES_CHANGED",
                "repository changed during indexing; retry",
                7,
            ));
        }
    } else {
        // Always revalidate source hashes before publishing, even when inventory verification is off.
        for file in &snapshot.files {
            let source = kortex_repo::read_confined(
                &root,
                &file.path,
                config.repository.max_file_size_bytes,
            )?;
            if kortex_core::hash(source) != file.content_hash {
                return Err(Error::new(
                    "FILES_CHANGED",
                    format!("{} changed during indexing", file.path),
                    7,
                ));
            }
        }
    }
    if snapshot.resolution_hash != kortex_repo::inputs::resolution_fingerprint(&root) {
        return Err(Error::new(
            "FILES_CHANGED",
            "resolution inputs changed during indexing; retry",
            7,
        ));
    }
    if let (Some(store), Some(lock)) = (&mut store, &lock) {
        store.commit(&snapshot, lock)?;
        if let Some(origin) = &config.origin {
            kortex_storage::registry::Registry::open()?.register_with_selection(
                &snapshot,
                &kortex_storage::registry::RepositorySelection {
                    invocation_root: origin.invocation_root.clone(),
                    explicit_config: origin.explicit_config.clone(),
                    effective_root: root.clone(),
                },
            )?;
        }
    }
    Ok(Report {
        snapshot,
        parsed,
        reused,
        committed: !options.dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn persistence_resolution_incremental_and_full_equivalence() {
        let d = tempfile::tempdir().unwrap();
        let c = Config::default();
        fs::create_dir(d.path().join("src")).unwrap();
        fs::write(
            d.path().join("src/lib.rs"),
            "mod auth; use crate::auth::check; fn entry() { check(); missing(); }",
        )
        .unwrap();
        fs::write(d.path().join("src/auth.rs"), "pub fn check() {}").unwrap();
        let first = index(d.path(), &c, &Options::default()).unwrap();
        assert_eq!(first.parsed, 2);
        let calls: Vec<_> = first
            .snapshot
            .edges
            .iter()
            .filter(|e| e.kind == "CALLS")
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls.iter().filter(|e| e.resolution == "exact").count(), 1);
        assert_eq!(
            calls
                .iter()
                .filter(|e| e.resolution == "unresolved")
                .count(),
            1
        );
        let second = index(d.path(), &c, &Options::default()).unwrap();
        assert_eq!(second.reused, 2);
        fs::write(d.path().join("src/auth.rs"), "pub fn check() { check(); }").unwrap();
        let incremental = index(
            d.path(),
            &c,
            &Options {
                verify: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(incremental.parsed, 1);
        let full = index(
            d.path(),
            &c,
            &Options {
                full: true,
                verify: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(incremental.snapshot.files, full.snapshot.files);
        assert_eq!(incremental.snapshot.edges, full.snapshot.edges);
        let s = Store::open(d.path(), false).unwrap();
        assert_eq!(s.load().unwrap().generation_id, full.snapshot.generation_id);
        assert!(
            !s.keyword(&full.snapshot.generation_id, "check", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            s.keyword(&first.snapshot.generation_id, "check", 10)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn ambiguity_and_shadowing_are_not_guessed() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.py"),"def check(): pass\ndef check(): pass\ndef entry(): check()\ndef shadow(check): check()\n").unwrap();
        fs::write(
            d.path().join("other.py"),
            "def missing(): pass\ndef other(): unrelated()\n",
        )
        .unwrap();
        let r = index(d.path(), &Config::default(), &Options::default()).unwrap();
        assert_eq!(
            r.snapshot
                .edges
                .iter()
                .filter(|e| e.resolution == "ambiguous")
                .count(),
            1
        );
        assert_eq!(
            r.snapshot
                .edges
                .iter()
                .filter(|e| e.resolution == "unresolved")
                .count(),
            2
        );
        assert!(
            r.snapshot
                .edges
                .iter()
                .filter(|e| e.resolution == "ambiguous")
                .all(|e| e.target_id.is_none() && e.candidates.len() == 2)
        );
    }
    #[test]
    fn dry_run_does_not_create_index() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.rs"), "fn x(){}").unwrap();
        let r = index(
            d.path(),
            &Config::default(),
            &Options {
                dry_run: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(!r.committed);
        assert!(!d.path().join(".kortex").exists());
    }
    #[test]
    fn cross_file_four_languages() {
        for (main, source, dependency, body) in [
            (
                "main.py",
                "from auth import check\ndef entry(): check()\n",
                "auth.py",
                "def check(): pass\n",
            ),
            (
                "main.ts",
                "import {check} from './auth'; function entry(){check();}",
                "auth.ts",
                "export function check(){}",
            ),
            (
                "main.js",
                "import {check} from './auth.js'; function entry(){check();}",
                "auth.js",
                "export function check(){}",
            ),
            (
                "main.go",
                "package main\nimport \"app/auth\"\nfunc entry(){auth.Check()}",
                "auth/auth.go",
                "package auth\nfunc Check(){}",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            if let Some(p) = Path::new(dependency).parent() {
                fs::create_dir_all(d.path().join(p)).unwrap();
            }
            fs::write(d.path().join(main), source).unwrap();
            fs::write(d.path().join(dependency), body).unwrap();
            fs::write(d.path().join("go.mod"), "module app\n").unwrap();
            let r = index(d.path(), &Config::default(), &Options::default()).unwrap();
            assert_eq!(
                r.snapshot
                    .edges
                    .iter()
                    .filter(|e| e.kind == "CALLS" && e.resolution == "exact")
                    .count(),
                1,
                "{main}: {:?}",
                r.snapshot.edges
            );
        }
    }
}
