//! Bytes-only tracked-name evidence. No Git, configuration, object or filesystem reads.
use kortex_core::{Error, RequestControl, Result};
use sha1::{Digest, Sha1};
use sha2::Sha256;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_PATH: usize = 32_768;
const MAX_NAMES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}
#[derive(Debug)]
pub struct TrackedNames {
    pub object_format: ObjectFormat,
    pub version: u32,
    pub paths: Vec<String>,
}
fn corrupt() -> Error {
    Error::new(
        "GIT_INDEX_CORRUPT",
        "invalid tracked-name index evidence",
        7,
    )
}
fn unsupported() -> Error {
    Error::new(
        "GIT_INDEX_UNSUPPORTED",
        "unsupported Git index version or required extension; complete tracked-name evidence is unavailable",
        7,
    )
}
fn budget() -> Error {
    Error::new(
        "RESOURCE_LIMIT",
        "Git index evidence exceeds its byte, path or entry policy",
        9,
    )
}
fn check(control: Option<&RequestControl>) -> Result<()> {
    if let Some(control) = control {
        control.check()?;
    }
    Ok(())
}
fn number(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset.checked_add(4).ok_or_else(corrupt)?)
        .ok_or_else(corrupt)?;
    Ok(u32::from_be_bytes(value.try_into().map_err(|_| corrupt())?))
}
fn checksum<D: Digest + Default>(
    bytes: &[u8],
    width: usize,
    control: Option<&RequestControl>,
) -> Result<bool> {
    let Some(end) = bytes.len().checked_sub(width) else {
        return Ok(false);
    };
    let mut hash = D::default();
    for chunk in bytes[..end].chunks(64 * 1024) {
        check(control)?;
        hash.update(chunk);
    }
    check(control)?;
    Ok(hash.finalize().as_slice() == &bytes[end..])
}

/// Checks the format checksum, framing and all names before returning any evidence.
/// SHA-1 here is a Git-format corruption checksum, never an authenticity primitive.
/// Split/sparse indexes explicitly refuse rather than silently omitting tracked paths.
pub fn decode(bytes: &[u8], control: Option<&RequestControl>) -> Result<TrackedNames> {
    check(control)?;
    if bytes.len() > MAX_BYTES {
        return Err(budget());
    }
    if bytes.get(..4) != Some(b"DIRC") {
        return Err(corrupt());
    }
    let version = number(bytes, 4)?;
    if !(2..=4).contains(&version) {
        return Err(unsupported());
    }
    let entries = usize::try_from(number(bytes, 8)?).map_err(|_| budget())?;
    if entries > MAX_ENTRIES {
        return Err(budget());
    }
    let (object_format, width) = if checksum::<Sha1>(bytes, 20, control)? {
        (ObjectFormat::Sha1, 20)
    } else if checksum::<Sha256>(bytes, 32, control)? {
        (ObjectFormat::Sha256, 32)
    } else {
        return Err(corrupt());
    };
    let end = bytes.len().checked_sub(width).ok_or_else(corrupt)?;
    let data = bytes.get(..end).ok_or_else(corrupt)?;
    let header = 40 + width + 2;
    if entries > data.len().saturating_sub(12) / (header + 1) {
        return Err(corrupt());
    }
    let mut offset = 12usize;
    let mut paths = Vec::<String>::new();
    let mut names_bytes = 0usize;
    let mut previous = Vec::<u8>::new();
    let mut previous_stage = 0u16;
    let mut empty = false;
    for _ in 0..entries {
        check(control)?;
        let start = offset;
        let fixed = data
            .get(start..start.checked_add(header).ok_or_else(corrupt)?)
            .ok_or_else(corrupt)?;
        let mode = number(fixed, 24)?;
        if mode & 0o170000 == 0o040000 {
            return Err(unsupported());
        }
        if !matches!(mode, 0o100644 | 0o100755 | 0o120000 | 0o160000) {
            return Err(corrupt());
        }
        let flags = u16::from_be_bytes([fixed[header - 2], fixed[header - 1]]);
        let stage = (flags >> 12) & 3;
        offset += header;
        if flags & 0x4000 != 0 {
            if version == 2 {
                return Err(corrupt());
            }
            let extra = data.get(offset..offset + 2).ok_or_else(corrupt)?;
            if u16::from_be_bytes([extra[0], extra[1]]) & !0x6000 != 0 {
                return Err(unsupported());
            }
            offset += 2;
        }
        let keep = if version == 4 {
            let mut strip = 0usize;
            loop {
                let byte = *data.get(offset).ok_or_else(corrupt)?;
                offset += 1;
                strip = strip
                    .checked_mul(128)
                    .and_then(|n| n.checked_add(usize::from(byte & 0x7f)))
                    .ok_or_else(corrupt)?;
                if byte & 0x80 == 0 {
                    break;
                }
                strip = strip.checked_add(1).ok_or_else(corrupt)?;
                if strip > MAX_PATH {
                    return Err(corrupt());
                }
            }
            previous.len().checked_sub(strip).ok_or_else(corrupt)?
        } else {
            0
        };
        let rest = data.get(offset..).ok_or_else(corrupt)?;
        let suffix_len = rest
            .iter()
            .take(MAX_PATH + 1)
            .position(|&byte| byte == 0)
            .ok_or_else(corrupt)?;
        let len = keep.checked_add(suffix_len).ok_or_else(budget)?;
        if len > MAX_PATH {
            return Err(budget());
        }
        if usize::from(flags & 0x0fff) != len.min(0x0fff) {
            return Err(corrupt());
        }
        names_bytes = names_bytes.checked_add(len).ok_or_else(budget)?;
        if names_bytes > MAX_NAMES {
            return Err(budget());
        }
        previous.truncate(keep);
        previous.extend_from_slice(&rest[..suffix_len]);
        offset += suffix_len + 1;
        if version != 4 {
            let padded = (offset - start).checked_add(7).ok_or_else(corrupt)? & !7;
            let next = start.checked_add(padded).ok_or_else(corrupt)?;
            if data
                .get(offset..next)
                .ok_or_else(corrupt)?
                .iter()
                .any(|&byte| byte != 0)
            {
                return Err(corrupt());
            }
            offset = next;
        }
        let name = std::str::from_utf8(&previous)
            .map_err(|_| Error::new("GIT_ENCODING", "tracked name is not UTF-8", 7))?;
        if name.is_empty() {
            empty = true;
        } else if name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
        {
            return Err(corrupt());
        }
        // Pathless split-index replacements must reach mandatory-extension refusal.
        if !empty && let Some(last) = paths.last() {
            match last.as_bytes().cmp(name.as_bytes()) {
                std::cmp::Ordering::Greater => return Err(corrupt()),
                std::cmp::Ordering::Equal if previous_stage >= stage => return Err(corrupt()),
                _ => {}
            }
        }
        paths.push(name.to_owned());
        previous_stage = stage;
    }
    while offset < end {
        check(control)?;
        let signature = data.get(offset..offset + 4).ok_or_else(corrupt)?;
        let size = usize::try_from(number(data, offset + 4)?).map_err(|_| budget())?;
        offset = offset
            .checked_add(8)
            .and_then(|n| n.checked_add(size))
            .ok_or_else(corrupt)?;
        if offset > end {
            return Err(corrupt());
        }
        if !signature[0].is_ascii_uppercase() {
            return Err(unsupported());
        }
    }
    if empty {
        return Err(corrupt());
    }
    check(control)?;
    Ok(TrackedNames {
        object_format,
        version,
        paths,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        path::Path,
        process::{Command, Stdio},
    };
    fn git_command(root: &Path, home: &Path) -> Command {
        let mut command = Command::new("git");
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TEMPLATE_DIR", home)
            .env("LC_ALL", "C")
            .args([
                "--no-pager",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-C",
            ])
            .arg(root);
        command
    }
    fn git(root: &Path, home: &Path, args: &[&str]) -> Vec<u8> {
        let output = git_command(root, home).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    fn git_input(root: &Path, home: &Path, args: &[&str], input: &[u8]) {
        let mut child = git_command(root, home)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn resigned(mut bytes: Vec<u8>, format: ObjectFormat) -> Vec<u8> {
        let width = if format == ObjectFormat::Sha1 { 20 } else { 32 };
        let end = bytes.len() - width;
        let digest = match format {
            ObjectFormat::Sha1 => Sha1::digest(&bytes[..end]).to_vec(),
            ObjectFormat::Sha256 => Sha256::digest(&bytes[..end]).to_vec(),
        };
        bytes[end..].copy_from_slice(&digest);
        bytes
    }
    fn signed(mut bytes: Vec<u8>, extension: Option<&[u8; 4]>) -> Vec<u8> {
        bytes.truncate(bytes.len() - 20);
        if let Some(signature) = extension {
            bytes.extend_from_slice(signature);
            bytes.extend_from_slice(&0u32.to_be_bytes());
        }
        let digest = Sha1::digest(&bytes);
        bytes.extend_from_slice(&digest);
        bytes
    }

    #[test]
    fn actual_git_metadata_rejects_forbidden_components_modes_and_ordering() {
        for format_name in ["sha1", "sha256"] {
            for version in [2, 3, 4] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("repo");
                let home = temp.path().join("home");
                fs::create_dir(&root).unwrap();
                fs::create_dir(&home).unwrap();
                git(
                    &root,
                    &home,
                    &["init", "--quiet", &format!("--object-format={format_name}")],
                );
                fs::create_dir_all(root.join("d/.giZ")).unwrap();
                fs::write(root.join("d/.giZ/x"), b"inert metadata fixture").unwrap();
                fs::write(root.join("z-skip.txt"), b"inert metadata fixture").unwrap();
                git(&root, &home, &["add", "--", "."]);
                if version == 3 {
                    git(
                        &root,
                        &home,
                        &["update-index", "--skip-worktree", "--", "z-skip.txt"],
                    );
                }
                git(
                    &root,
                    &home,
                    &["update-index", &format!("--index-version={version}")],
                );
                let bytes = fs::read(root.join(".git/index")).unwrap();
                let decoded = decode(&bytes, None).unwrap();
                assert_eq!(decoded.version, version);
                let format = decoded.object_format;
                let width = if format == ObjectFormat::Sha1 { 20 } else { 32 };
                let flags_offset = 12 + 40 + width;
                let flags = u16::from_be_bytes([bytes[flags_offset], bytes[flags_offset + 1]]);
                let name_offset = flags_offset
                    + 2
                    + if flags & 0x4000 != 0 { 2 } else { 0 }
                    + if version == 4 { 1 } else { 0 };
                assert_eq!(&bytes[name_offset..name_offset + 8], b"d/.giZ/x");
                let mut bad = bytes.clone();
                bad[name_offset + 5] = b't';
                assert_eq!(
                    decode(&resigned(bad, format), None).unwrap_err().code,
                    "GIT_INDEX_CORRUPT"
                );
                for mode in [0u32, 0o100600, 0x8000_81a4, 0o120644, 0o160755] {
                    let mut bad = bytes.clone();
                    bad[12 + 24..12 + 28].copy_from_slice(&mode.to_be_bytes());
                    assert_eq!(
                        decode(&resigned(bad, format), None).unwrap_err().code,
                        "GIT_INDEX_CORRUPT"
                    );
                }
                for mode in [0o100644u32, 0o100755, 0o120000, 0o160000] {
                    let mut allowed = bytes.clone();
                    allowed[12 + 24..12 + 28].copy_from_slice(&mode.to_be_bytes());
                    assert_eq!(
                        decode(&resigned(allowed, format), None).unwrap().paths,
                        decoded.paths
                    );
                }
                let oid =
                    String::from_utf8(git(&root, &home, &["rev-parse", ":d/.giZ/x"])).unwrap();
                let oid = oid.trim();
                let mut input = format!("0 {}\td/.giZ/x\n", "0".repeat(oid.len()));
                for stage in 1..=3 {
                    input.push_str(&format!("100644 {oid} {stage}\td/.giZ/x\n"));
                }
                git_input(
                    &root,
                    &home,
                    &["update-index", "--index-info"],
                    input.as_bytes(),
                );
                git(
                    &root,
                    &home,
                    &["update-index", &format!("--index-version={version}")],
                );
                let staged = fs::read(root.join(".git/index")).unwrap();
                let decoded = decode(&staged, None).unwrap();
                assert_eq!(decoded.version, version);
                let oracle = git(&root, &home, &["ls-files", "--cached", "-z"]);
                let expected: Vec<_> = oracle
                    .split(|&b| b == 0)
                    .filter(|p| !p.is_empty())
                    .map(|p| std::str::from_utf8(p).unwrap())
                    .collect();
                assert_eq!(decoded.paths, expected);
                assert_eq!(
                    decoded
                        .paths
                        .iter()
                        .filter(|p| p.as_str() == "d/.giZ/x")
                        .count(),
                    3
                );
                for stage in [2u16, 3] {
                    let mut bad = staged.clone();
                    let flags = u16::from_be_bytes([bad[flags_offset], bad[flags_offset + 1]]);
                    bad[flags_offset..flags_offset + 2]
                        .copy_from_slice(&((flags & !0x3000) | stage << 12).to_be_bytes());
                    assert_eq!(
                        decode(&resigned(bad, format), None).unwrap_err().code,
                        "GIT_INDEX_CORRUPT"
                    );
                }
                let mut bad = staged.clone();
                bad[name_offset] = b'~';
                assert_eq!(
                    decode(&resigned(bad, format), None).unwrap_err().code,
                    "GIT_INDEX_CORRUPT"
                );
            }
        }
    }

    #[test]
    fn valid_checksums_do_not_hide_malformed_names_or_required_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let home = temp.path().join("home");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&home).unwrap();
        git(&root, &home, &["init", "--quiet", "--object-format=sha1"]);
        fs::write(root.join("one.txt"), b"inert data").unwrap();
        git(&root, &home, &["add", "--", "one.txt"]);
        git(&root, &home, &["update-index", "--index-version=2"]);
        let bytes = fs::read(root.join(".git/index")).unwrap();
        let expected = decode(&bytes, None).unwrap().paths;
        assert_eq!(
            decode(&signed(bytes.clone(), Some(b"XOWN")), None)
                .unwrap()
                .paths,
            expected
        );
        for extension in [b"link", b"sdir", b"xxxx"] {
            assert_eq!(
                decode(&signed(bytes.clone(), Some(extension)), None)
                    .unwrap_err()
                    .code,
                "GIT_INDEX_UNSUPPORTED"
            );
        }
        for (byte, code) in [(b'/', "GIT_INDEX_CORRUPT"), (0xff, "GIT_ENCODING")] {
            let mut bad = bytes.clone();
            bad[12 + 62] = byte;
            assert_eq!(decode(&signed(bad, None), None).unwrap_err().code, code);
        }
        git(&root, &home, &["update-index", "--split-index"]);
        let split = fs::read(root.join(".git/index")).unwrap();
        assert_eq!(
            decode(&split, None).unwrap_err().code,
            "GIT_INDEX_UNSUPPORTED"
        );
        let control = RequestControl::new(std::time::Duration::from_secs(10)).unwrap();
        control.cancel();
        assert_eq!(
            decode(&bytes, Some(&control)).unwrap_err().code,
            "CANCELLED"
        );
        let oversized = vec![0; MAX_BYTES + 1];
        assert_eq!(decode(&oversized, None).unwrap_err().code, "RESOURCE_LIMIT");
    }

    #[test]
    fn actual_git_versions_formats_unicode_prefixes_and_corruption() {
        for format in ["sha1", "sha256"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("repo");
            let home = temp.path().join("home");
            fs::create_dir(&root).unwrap();
            fs::create_dir(&home).unwrap();
            git(
                &root,
                &home,
                &["init", "--quiet", &format!("--object-format={format}")],
            );
            for path in [
                "a.txt",
                "space name.txt",
                "caf\u{e9}.txt",
                "prefix/long_shared_name_a.txt",
                "prefix/long_shared_name_b.txt",
            ] {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"inert data, never executable\n").unwrap();
            }
            git(&root, &home, &["add", "--", "."]);
            for version in [2, 3, 4] {
                if version == 3 {
                    git(
                        &root,
                        &home,
                        &["update-index", "--skip-worktree", "--", "a.txt"],
                    );
                }
                git(
                    &root,
                    &home,
                    &["update-index", &format!("--index-version={version}")],
                );
                let bytes = fs::read(root.join(".git/index")).unwrap();
                let decoded = decode(&bytes, None).unwrap();
                assert_eq!(decoded.version, version);
                assert_eq!(
                    decoded.object_format,
                    if format == "sha1" {
                        ObjectFormat::Sha1
                    } else {
                        ObjectFormat::Sha256
                    }
                );
                let oracle = git(&root, &home, &["ls-files", "--cached", "-z"]);
                let expected: Vec<_> = oracle
                    .split(|&b| b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| std::str::from_utf8(s).unwrap())
                    .collect();
                assert_eq!(decoded.paths, expected);
                for end in [0, 3, 11, bytes.len() - 1] {
                    assert!(decode(&bytes[..end], None).is_err());
                }
                let mut bad = bytes.clone();
                bad[12] ^= 1;
                assert_eq!(decode(&bad, None).unwrap_err().code, "GIT_INDEX_CORRUPT");
            }
        }
    }
}
