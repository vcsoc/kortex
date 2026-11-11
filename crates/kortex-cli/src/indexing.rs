//! CLI indexing orchestration; discovery/storage remain owned by shared crates.
use crate::IndexArgs;
use kortex_config::Config;
use kortex_core::Result;
use kortex_storage::registry::Registry;
use serde_json::{Value, json};
use std::path::Path;
fn one(path: &Path, args: &IndexArgs, config_path: Option<&Path>) -> Result<Value> {
    let base = kortex_repo::root(path)?;
    let mut config = Config::load(&base, config_path)?;
    if args.incremental {
        config.index.mode = "incremental".into();
    }
    if args.include_hidden {
        config.repository.include_hidden = true;
    }
    if args.follow_symlinks {
        config.repository.follow_symlinks = true;
    }
    if let Some(n) = args.max_file_size {
        config.repository.max_file_size_bytes = n;
    }
    if let Some(n) = args.max_files {
        config.repository.max_files = n;
    }
    config.repository.include.extend(args.include.clone());
    config.repository.exclude.extend(args.exclude.clone());
    if !args.languages.is_empty() {
        config.repository.languages = args.languages.clone();
    }
    config.validate()?;
    let root = config.resolved_root(&base)?;
    let r = kortex_indexer::index_at_root(
        &root,
        &config,
        &kortex_indexer::Options {
            full: args.full,
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )?;
    if r.committed {
        Registry::open()?.register(&r.snapshot)?;
    }
    let diagnostics = r.snapshot.diagnostics.len()
        + r.snapshot
            .files
            .iter()
            .map(|f| f.diagnostics.len())
            .sum::<usize>();
    Ok(
        json!({"schema_version":"1.0","repo_id":r.snapshot.repo_id,"generation_id":r.snapshot.generation_id,"committed":r.committed,"files":r.snapshot.files.len(),"symbols":r.snapshot.symbols().count(),"edges":r.snapshot.edges.len(),"parsed":r.parsed,"reused":r.reused,"diagnostic_count":diagnostics,"diagnostics":r.snapshot.diagnostics}),
    )
}
pub(crate) fn run(args: IndexArgs, config_path: Option<&Path>) -> Result<(Value, i32)> {
    let Some(manifest) = args.workspace.as_deref() else {
        return Ok((one(&args.path, &args, config_path)?, 0));
    };
    let roots = kortex_repo::workspace::load(manifest)?;
    let mut items = Vec::new();
    let mut partial = false;
    for root in roots {
        match one(&root, &args, config_path) {
            Ok(report) => items.push(json!({"root":root,"result":report})),
            Err(error) => {
                partial = true;
                items.push(json!({"root":root,"result":kortex_analysis::error_json(&error)}));
            }
        }
    }
    Ok((
        json!({"schema_version":"1.0","workspace":manifest,"partial":partial,"items":items}),
        if partial { 7 } else { 0 },
    ))
}
