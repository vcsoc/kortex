use kortex_core::{Error, Result};
use std::{
    fs::{self, OpenOptions},
    io::{IsTerminal, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const START: &str = "<!-- kortex:instructions:start";
const END: &str = "<!-- kortex:instructions:end -->";
const CURRENT: u32 = 1;
const SECTION: &str = include_str!("agent_instructions.md");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn invalid(message: &str) -> Error {
    Error::new("AGENT_INSTRUCTIONS_INVALID", message, 2)
}

fn updated(
    existing: &str,
    confirm: impl FnOnce(u32) -> Result<bool>,
) -> Result<(String, &'static str)> {
    let starts: Vec<_> = existing.match_indices(START).collect();
    let ends: Vec<_> = existing.match_indices(END).collect();
    if starts.is_empty() && ends.is_empty() {
        let separator = if existing.is_empty() || existing.ends_with("\n\n") {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        return Ok((format!("{existing}{separator}{SECTION}"), "added"));
    }
    if starts.len() != 1 || ends.len() != 1 {
        return Err(invalid(
            "AGENTS.md has malformed or duplicate Kortex markers",
        ));
    }
    let start = starts[0].0;
    let end = ends[0].0;
    if end < start || (start != 0 && existing.as_bytes()[start - 1] != b'\n') {
        return Err(invalid("AGENTS.md has misplaced Kortex markers"));
    }
    let header_end = existing[start..]
        .find(" -->")
        .map(|n| start + n + 4)
        .ok_or_else(|| invalid("AGENTS.md has an invalid Kortex version header"))?;
    let version = existing[start..header_end]
        .strip_prefix("<!-- kortex:instructions:start version=")
        .and_then(|s| s.strip_suffix(" -->"))
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| invalid("AGENTS.md has an invalid Kortex version header"))?;
    if header_end > end || (end != 0 && existing.as_bytes()[end - 1] != b'\n') {
        return Err(invalid("AGENTS.md has misplaced Kortex markers"));
    }
    if version >= CURRENT {
        return Ok((
            existing.to_owned(),
            if version == CURRENT {
                "current"
            } else {
                "newer"
            },
        ));
    }
    if !confirm(version)? {
        return Ok((existing.to_owned(), "update_declined"));
    }
    let end = end + END.len();
    let section = SECTION.trim_end_matches('\n');
    Ok((
        format!("{}{section}{}", &existing[..start], &existing[end..]),
        "updated",
    ))
}

pub(crate) fn install(root: &Path, json: bool) -> Result<&'static str> {
    let path = root.join("AGENTS.md");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(SECTION.as_bytes())?;
            return Ok("created");
        }
        Err(e) => return Err(e.into()),
    };
    if !metadata.file_type().is_file() {
        return Err(Error::new(
            "PATH_REFUSED",
            "AGENTS.md must be a regular file, not a symlink",
            8,
        ));
    }
    let original = String::from_utf8(kortex_repo::read_confined_bytes(
        root,
        "AGENTS.md",
        1024 * 1024,
    )?)
    .map_err(|_| invalid("AGENTS.md must be UTF-8"))?;
    let (replacement, status) = updated(&original, |version| {
        if json || !std::io::stdin().is_terminal() {
            return Ok(false);
        }
        eprint!(
            "Kortex instructions version {version} can be updated to {CURRENT}. Update? [y/N] "
        );
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    })?;
    if replacement == original {
        return Ok(status);
    }
    // Write a sibling and rename it, rather than truncating a possibly linked file.
    let temp = root.join(format!(
        ".kortex-agents-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.set_permissions(metadata.permissions())?;
        file.write_all(replacement.as_bytes())?;
        drop(file);
        if fs::symlink_metadata(&path)?.file_type().is_symlink()
            || kortex_repo::read_confined_bytes(root, "AGENTS.md", 1024 * 1024)?
                != original.as_bytes()
        {
            return Err(invalid("AGENTS.md changed during initialization; retry"));
        }
        fs::rename(&temp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_and_updates_only_with_consent() {
        let old = "before\n<!-- kortex:instructions:start version=0 -->\nold\n<!-- kortex:instructions:end -->\nafter\n";
        let (declined, status) = updated(old, |_| Ok(false)).unwrap();
        assert_eq!(status, "update_declined");
        assert_eq!(declined, old);
        let (accepted, status) = updated(old, |_| Ok(true)).unwrap();
        assert_eq!(status, "updated");
        assert_eq!(accepted, format!("before\n{}after\n", SECTION));
        assert_eq!(
            updated(&accepted, |_| panic!("unexpected prompt"))
                .unwrap()
                .1,
            "current"
        );
    }

    #[test]
    fn rejects_ambiguous_markers_and_preserves_newer_versions() {
        for input in [
            START,
            END,
            "<!-- kortex:instructions:start version=x -->\n<!-- kortex:instructions:end -->",
        ] {
            assert!(updated(input, |_| Ok(true)).is_err());
        }
        let newer = "<!-- kortex:instructions:start version=999 -->\ntext\n<!-- kortex:instructions:end -->";
        assert_eq!(
            updated(newer, |_| panic!("unexpected prompt")).unwrap().1,
            "newer"
        );
    }
}
