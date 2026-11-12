//! Hash non-source inputs that can change module resolution or index policy.
use std::path::Path;
pub fn resolution_fingerprint(root: &Path) -> String {
    fingerprint(root, None).expect("uncontrolled fingerprint records read errors as before")
}

/// Same resolution-input policy; cancellation and timeout never become digests.
pub fn resolution_fingerprint_with_control(
    root: &Path,
    control: &kortex_core::RequestControl,
) -> kortex_core::Result<String> {
    fingerprint(root, Some(control))
}

fn fingerprint(
    root: &Path,
    control: Option<&kortex_core::RequestControl>,
) -> kortex_core::Result<String> {
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
        super::checkpoint(control)?;
        let read = match control {
            Some(control) => {
                super::read_confined_with_control(root, path, 2 * 1024 * 1024, control)
            }
            None => super::read_confined(root, path, 2 * 1024 * 1024),
        };
        super::checkpoint(control)?;
        let digest = match read {
            Ok(source) => super::hash_controlled(source.as_bytes(), control)?,
            Err(error) => format!("unavailable:{}", error.code),
        };
        records.push(format!("{path}:{digest}"));
    }
    super::checkpoint(control)?;
    Ok(kortex_core::hash(records.join("\n")))
}
