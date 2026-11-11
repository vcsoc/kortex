//! Explicit, versioned multi-repository manifests; no glob expansion or command execution.
use kortex_core::{Error, Result};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    repositories: Vec<String>,
}
/// Read `schema_version = 1; repositories = ["relative/root", "/absolute/root"]` TOML.
/// Relative paths are anchored at the manifest, not the process working directory.
/// Git subdirectories normalize to their guarded worktree roots; duplicates collapse.
pub fn load(path: &Path) -> Result<Vec<PathBuf>> {
    let manifest_path = path.canonicalize()?;
    let base = manifest_path
        .parent()
        .ok_or_else(|| Error::new("INVALID_WORKSPACE", "manifest has no parent", 2))?;
    // Reuse source confinement rather than opening an ambient path after a
    // metadata check: this also opens Unix FIFOs nonblocking before refusal.
    let file = crate::confined::open(base, &manifest_path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::new(
            "INVALID_WORKSPACE",
            "workspace manifest must be a regular file",
            2,
        ));
    }
    let mut text = String::new();
    file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        return Err(Error::new(
            "INVALID_WORKSPACE",
            "workspace manifest exceeds 1 MiB",
            2,
        ));
    }
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| Error::new("INVALID_WORKSPACE", e.to_string(), 2))?;
    if manifest.schema_version != 1
        || manifest.repositories.is_empty()
        || manifest.repositories.len() > 1024
    {
        return Err(Error::new(
            "INVALID_WORKSPACE",
            "expected schema_version=1 and 1..1024 repositories",
            2,
        ));
    }
    let mut roots = BTreeSet::new();
    for entry in manifest.repositories {
        if entry.is_empty() || entry.contains('\0') {
            return Err(Error::new(
                "INVALID_WORKSPACE",
                "repository entries must be nonempty paths",
                2,
            ));
        }
        roots.insert(crate::root(&base.join(entry))?);
    }
    Ok(roots.into_iter().collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_manifest_relative_absolute_restart_duplicates_and_errors() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("one")).unwrap();
        std::fs::create_dir(d.path().join("two")).unwrap();
        let path = d.path().join("kortex.workspace.toml");
        std::fs::write(
            &path,
            format!(
                "schema_version=1\nrepositories=['one', 'one/.', {:?}]\n",
                d.path().join("two").to_str().unwrap()
            ),
        )
        .unwrap();
        let first = load(&path).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(load(&path).unwrap(), first);
        for contents in [
            "schema_version=2\nrepositories=['one']",
            "schema_version=1\nrepositories=[]",
            "schema_version=1\nrepositories=['missing']",
            "schema_version=1\nrepositories=['one']\ncommand='touch owned'",
        ] {
            std::fs::write(&path, contents).unwrap();
            assert!(load(&path).is_err(), "accepted {contents}");
        }
        assert!(!d.path().join("owned").exists());
    }
    #[test]
    fn tracked_case_collisions_are_detected_even_on_case_insensitive_filesystems() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        crate::git(&root, &["init", "-q"]).unwrap();
        crate::git(&root, &["config", "core.ignorecase", "false"]).unwrap();
        std::fs::write(root.join("Name.rs"), "fn source(){}").unwrap();
        let object = crate::git(&root, &["hash-object", "-w", "Name.rs"]).unwrap();
        for path in ["Name.rs", "name.rs"] {
            crate::git(
                &root,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{},{}", object.trim(), path),
                ],
            )
            .unwrap();
        }
        assert_eq!(
            crate::git(&root, &["ls-files", "-z"])
                .unwrap()
                .split('\0')
                .filter(|s| !s.is_empty())
                .count(),
            2
        );
        assert_eq!(
            crate::discover(&root, &kortex_config::Config::default())
                .err()
                .unwrap()
                .code,
            "CASE_COLLISION"
        );
        crate::git(&root, &["update-index", "--force-remove", "name.rs"]).unwrap();
        std::fs::rename(root.join("Name.rs"), root.join("name.rs")).unwrap();
        let renamed = crate::discover(&root, &kortex_config::Config::default()).unwrap();
        assert_eq!(renamed.files.len(), 1);
        assert_eq!(renamed.files[0].path, "name.rs");
    }
    #[test]
    fn case_collisions_are_not_silently_published() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::write(root.join("Name.rs"), "fn upper(){}").unwrap();
        std::fs::write(root.join("name.rs"), "fn lower(){}").unwrap();
        let distinct = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .count()
            == 2;
        let result = crate::discover(&root, &kortex_config::Config::default());
        if distinct {
            assert_eq!(result.err().unwrap().code, "CASE_COLLISION");
        } else {
            assert_eq!(result.unwrap().files.len(), 1);
        }
    }
}
