//! Hash non-source inputs that can change module resolution or index policy.
use std::path::Path;
pub fn resolution_fingerprint(root: &Path) -> String {
    let mut records = vec![];
    for path in [
        "go.mod",
        "Cargo.toml",
        "pyproject.toml",
        "tsconfig.json",
        "jsconfig.json",
        "package.json",
        ".gitignore",
        ".ignore",
        ".kortexignore",
    ] {
        let digest = super::read_confined(root, path, 2 * 1024 * 1024)
            .map(kortex_core::hash)
            .unwrap_or_else(|e| format!("unavailable:{}", e.code));
        records.push(format!("{path}:{digest}"));
    }
    kortex_core::hash(records.join("\n"))
}
