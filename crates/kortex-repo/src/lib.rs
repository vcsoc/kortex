//! Confined repository discovery and read-only Git inspection.
mod confined;
pub mod inputs;
pub mod watch;
pub mod workspace;
use kortex_config::Config;
use kortex_core::{Error, Result, hash, public_path};
#[cfg(test)]
use std::fs;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

pub struct SourceFile {
    pub path: String,
    pub source: String,
    pub content_hash: String,
}
pub struct Inventory {
    pub files: Vec<SourceFile>,
    pub diagnostics: Vec<String>,
}
pub fn root(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|e| Error::new("REPO_NOT_FOUND", e.to_string(), 3))?;
    if canonical.to_str().is_none() {
        return Err(Error::new(
            "INVALID_PATH_ENCODING",
            "repository paths must be UTF-8",
            2,
        ));
    }
    if !canonical.is_dir() {
        return Err(Error::new(
            "REPO_NOT_FOUND",
            "repository root must be a directory",
            3,
        ));
    }
    if let Ok(top) = git(&canonical, &["rev-parse", "--show-toplevel"]) {
        let top = Path::new(top.strip_suffix('\n').unwrap_or(&top)).canonicalize()?;
        if canonical.starts_with(&top) {
            // core.worktree is untrusted repository configuration. An ancestor is
            // only a root when its own .git marker resolves to this repository,
            // including linked worktrees and submodule gitfiles.
            let marker_dir = git(&top, &["rev-parse", "--resolve-git-dir", ".git"])
                .ok()
                .and_then(|p| {
                    top.join(p.strip_suffix('\n').unwrap_or(&p))
                        .canonicalize()
                        .ok()
                });
            let actual_dir = git(&canonical, &["rev-parse", "--absolute-git-dir"])
                .ok()
                .and_then(|p| {
                    Path::new(p.strip_suffix('\n').unwrap_or(&p))
                        .canonicalize()
                        .ok()
                });
            if marker_dir.is_none() || marker_dir != actual_dir {
                return Err(Error::new(
                    "PATH_REFUSED",
                    "Git worktree root lacks a matching .git marker; refusing root expansion",
                    8,
                ));
            }
            return Ok(top);
        }
    }
    Ok(canonical)
}
pub fn language(path: &str) -> Option<&'static str> {
    match Path::new(path).extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "cs" => Some("csharp"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "rb" | "rake" | "gemspec" => Some("ruby"),
        "php" | "phtml" => Some("php"),
        _ => None,
    }
}
pub fn read_confined(root: &Path, relative: &str, max: u64) -> Result<String> {
    let input = Path::new(relative);
    if input.is_absolute()
        || input
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::new(
            "PATH_REFUSED",
            "expected normal repository-relative path",
            8,
        ));
    }
    let path = root.join(input).canonicalize()?;
    if !path.starts_with(root) {
        return Err(Error::new("PATH_REFUSED", "symlink escapes repository", 8));
    }
    let file = confined::open(root, &path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::new("PATH_REFUSED", "not a regular file", 8));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(Error::new(
            "FILE_TOO_LARGE",
            "file exceeds configured size limit",
            7,
        ));
    }
    if bytes.contains(&0) {
        return Err(Error::new("BINARY_FILE", "binary file", 7));
    }
    String::from_utf8(bytes).map_err(|_| Error::new("INVALID_ENCODING", "not UTF-8", 7))
}
pub fn discover(root: &Path, config: &Config) -> Result<Inventory> {
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for pattern in &config.repository.include {
        overrides
            .add(pattern)
            .map_err(|e| Error::new("INVALID_GLOB", e.to_string(), 2))?;
    }
    for pattern in &config.repository.exclude {
        overrides
            .add(&format!("!{pattern}"))
            .map_err(|e| Error::new("INVALID_GLOB", e.to_string(), 2))?;
    }
    let overrides = overrides
        .build()
        .map_err(|e| Error::new("INVALID_GLOB", e.to_string(), 2))?;
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(!config.repository.include_hidden)
        .follow_links(config.repository.follow_symlinks)
        .add_custom_ignore_filename(".kortexignore")
        .overrides(overrides)
        .filter_entry(|e| {
            ![
                ".git",
                ".kortex",
                "node_modules",
                "target",
                "dist",
                "build",
                "vendor",
                ".venv",
                "venv",
                "__pycache__",
            ]
            .iter()
            .any(|n| e.file_name() == *n)
        });
    builder.sort_by_file_name(|a, b| a.cmp(b));
    let mut total_bytes = 0u64;
    let mut inventory = Inventory {
        files: vec![],
        diagnostics: vec![],
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut case_paths = std::collections::BTreeMap::<String, String>::new();
    // A case-insensitive checkout cannot hold both spellings, but Git's index can.
    // Inspect names only; ls-files never runs clean filters or repository code.
    if let Ok(tracked) = git(root, &["ls-files", "--cached", "-z"]) {
        for path in tracked.split('\0').filter(|path| !path.is_empty()) {
            if let Some(previous) = case_paths.insert(path.to_lowercase(), path.to_owned())
                && previous != path
            {
                return Err(Error::new(
                    "CASE_COLLISION",
                    format!("tracked case-insensitive path collision: {previous:?} and {path:?}"),
                    8,
                ));
            }
        }
    }
    // Git may retain an old spelling after a legitimate unstaged case-only rename.
    // Detect collisions within each inventory, not between old Git names and current files.
    case_paths.clear();
    let mut considered = 0;
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                inventory.diagnostics.push(e.to_string());
                continue;
            }
        };
        if entry.depth() == 0 || entry.file_type().is_some_and(|t| t.is_dir()) {
            continue;
        }
        if !config.repository.follow_symlinks && entry.file_type().is_some_and(|t| t.is_symlink()) {
            inventory
                .diagnostics
                .push(format!("{}: symlink skipped", entry.path().display()));
            continue;
        }
        considered += 1;
        if considered > config.repository.max_files {
            inventory
                .diagnostics
                .push("max_files reached; inventory truncated".into());
            break;
        }
        let canonical = match entry.path().canonicalize() {
            Ok(p) if p.starts_with(root) => p,
            _ => {
                inventory
                    .diagnostics
                    .push(format!("{}: path refused", entry.path().display()));
                continue;
            }
        };
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| Error::new("PATH_REFUSED", e.to_string(), 8))?;
        if relative.to_str().is_none() {
            inventory.diagnostics.push(format!(
                "{:?}: unsupported filename encoding; skipped",
                relative
            ));
            continue;
        }
        let path = public_path(relative);
        let Some(lang) = language(&path) else {
            inventory
                .diagnostics
                .push(format!("{path}: unsupported language"));
            continue;
        };
        if !config.repository.languages.is_empty()
            && !config.repository.languages.iter().any(|l| l == lang)
        {
            continue;
        }
        // Refuse portable identity collisions rather than silently choosing one spelling.
        // This also prevents a Linux-built inventory becoming lossy on case-insensitive hosts.
        if let Some(previous) = case_paths.insert(path.to_lowercase(), path.clone())
            && previous != path
        {
            return Err(Error::new(
                "CASE_COLLISION",
                format!("case-insensitive path collision: {previous:?} and {path:?}"),
                8,
            ));
        }
        // A skipped alias must not consume the canonical source's dedup slot.
        if !seen.insert(canonical) {
            continue;
        }
        match read_confined(root, &path, config.repository.max_file_size_bytes) {
            Ok(source) => {
                total_bytes += source.len() as u64;
                if total_bytes > config.repository.max_total_bytes {
                    inventory
                        .diagnostics
                        .push("max_total_bytes reached; inventory truncated".into());
                    break;
                }
                let content_hash = hash(&source);
                inventory.files.push(SourceFile {
                    path,
                    source,
                    content_hash,
                });
            }
            Err(e) => inventory.diagnostics.push(format!("{path}: {e}")),
        }
    }
    inventory.files.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.diagnostics.sort();
    Ok(inventory)
}
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    const OUTPUT_LIMIT: u64 = 8 * 1024 * 1024;
    let mut child = Command::new("git")
        .arg("--no-pager")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("diff.external=")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    fn collect(
        reader: impl Read + Send + 'static,
    ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut bytes = vec![];
            reader.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    }
    let stdout = collect(
        child
            .stdout
            .take()
            .ok_or_else(|| Error::new("GIT_ERROR", "missing stdout", 1))?,
    );
    let stderr = collect(
        child
            .stderr
            .take()
            .ok_or_else(|| Error::new("GIT_ERROR", "missing stderr", 1))?,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    let out = stdout
        .join()
        .map_err(|_| Error::new("GIT_ERROR", "stdout reader panicked", 1))??;
    let err = stderr
        .join()
        .map_err(|_| Error::new("GIT_ERROR", "stderr reader panicked", 1))??;
    let status = status.ok_or_else(|| Error::new("GIT_TIMEOUT", "Git exceeded five seconds", 9))?;
    if out.len() as u64 > OUTPUT_LIMIT || err.len() as u64 > OUTPUT_LIMIT {
        return Err(Error::new(
            "GIT_OUTPUT_LIMIT",
            "Git output exceeds 8 MiB",
            7,
        ));
    }
    if !status.success() {
        return Err(Error::new(
            "GIT_ERROR",
            String::from_utf8_lossy(&err).trim(),
            2,
        ));
    }
    String::from_utf8(out).map_err(|_| {
        Error::new(
            "GIT_ENCODING",
            "Git output is not UTF-8; unsupported filename encoding",
            7,
        )
    })
}
pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned())
}

#[cfg(test)]
mod confinement_tests;
#[cfg(test)]
mod filename_tests;
#[cfg(test)]
mod root_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_and_limits() {
        let d = tempfile::tempdir().unwrap();
        let r = root(d.path()).unwrap();
        fs::create_dir(r.join(".git")).unwrap();
        fs::create_dir(r.join("target")).unwrap();
        fs::write(r.join("ok.rs"), "fn ok(){} ").unwrap();
        fs::write(r.join("skip.py"), "x=1").unwrap();
        fs::write(r.join(".gitignore"), "skip.py\n").unwrap();
        fs::write(r.join("target/bad.rs"), "fn bad(){}").unwrap();
        let c = Config::default();
        let inv = discover(&r, &c).unwrap();
        assert_eq!(inv.files.len(), 1);
        assert_eq!(inv.files[0].path, "ok.rs");
        assert!(read_confined(&r, "../x", 100).is_err());
        assert!(read_confined(&r, "ok.rs", 1).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.rs"), "fn secret(){}").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.rs"), d.path().join("link.rs"))
            .unwrap();
        let r = root(d.path()).unwrap();
        assert!(read_confined(&r, "link.rs", 100).is_err());
        let mut c = Config::default();
        c.repository.follow_symlinks = true;
        assert!(discover(&r, &c).unwrap().files.is_empty());
    }
}
