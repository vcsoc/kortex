//! Git spans are meaningful only in the file content from which they were indexed.
use super::{Service, boolean, commit, string};
use kortex_core::{Error, Result, hash};
use kortex_graph::Direction;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
struct Lines {
    start: usize,
    count: usize,
}
impl Lines {
    fn parse(word: &str, prefix: char) -> Option<Self> {
        let raw = word.strip_prefix(prefix)?;
        let (start, count) = raw.split_once(',').unwrap_or((raw, "1"));
        Some(Self {
            start: start.parse().ok()?,
            count: count.parse().ok()?,
        })
    }
    fn range(self) -> Option<(usize, usize)> {
        // A zero-count hunk describes a gap, not an adjacent changed line.
        (self.count > 0).then(|| (self.start, self.start.saturating_add(self.count - 1)))
    }
}

// Patch headers use Git's C-style path quoting; name-status remains NUL-delimited.
fn patch_path(raw: &str, prefix: &str) -> Result<Option<String>> {
    let invalid = || Error::new("INVALID_DIFF", "invalid Git patch path", 2);
    if raw == "/dev/null" {
        return Ok(None);
    }
    let decoded = if raw.starts_with('"') {
        let inner = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(invalid)?;
        let mut bytes = inner.bytes();
        let mut out = vec![];
        while let Some(byte) = bytes.next() {
            if byte != b'\\' {
                out.push(byte);
                continue;
            }
            let escape = bytes.next().ok_or_else(invalid)?;
            out.push(match escape {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                b'\\' => b'\\',
                b'"' => b'"',
                b'0'..=b'3' => {
                    let second = bytes
                        .next()
                        .filter(|b| matches!(b, b'0'..=b'7'))
                        .ok_or_else(invalid)?;
                    let third = bytes
                        .next()
                        .filter(|b| matches!(b, b'0'..=b'7'))
                        .ok_or_else(invalid)?;
                    (escape - b'0') * 64 + (second - b'0') * 8 + third - b'0'
                }
                _ => return Err(invalid()),
            });
        }
        String::from_utf8(out).map_err(|_| invalid())?
    } else {
        // Git terminates unquoted headers containing spaces with a tab.
        raw.strip_suffix('\t').unwrap_or(raw).to_owned()
    };
    Ok(Some(
        decoded.strip_prefix(prefix).ok_or_else(invalid)?.to_owned(),
    ))
}

impl Service {
    pub(super) fn change_analysis(&self, args: &Value) -> Result<Value> {
        let staged = boolean(args, "staged", false)?;
        let unstaged = boolean(args, "unstaged", false)?;
        let reference = |key| -> Result<Option<String>> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                _ => commit(&self.root, string(args, key)?).map(Some),
            }
        };
        let base = reference("base")?;
        let head = reference("head")?;
        if (staged && unstaged) || (head.is_some() && (base.is_none() || staged || unstaged)) {
            return Err(Error::new(
                "INVALID_DIFF",
                "staged/unstaged are exclusive; head requires base and cannot use either mode",
                2,
            ));
        }
        let mut revisions = vec![];
        if staged {
            revisions.push("--cached".to_owned());
        }
        if let Some(base) = &base {
            revisions.push(base.clone());
        }
        if let Some(head) = &head {
            revisions.push(head.clone());
        }
        // With no explicit base, unstaged compares index -> worktree and staged
        // compares HEAD (or the empty tree for an unborn branch) -> index.
        let old_revision = base.clone().or_else(|| {
            if staged {
                kortex_repo::head(&self.root)
            } else {
                Some(String::new())
            }
        });
        let new_revision = head.clone().or_else(|| staged.then(String::new));
        let old_kind = match old_revision.as_deref() {
            None => "empty_tree",
            Some("") => "index",
            Some(_) => "commit",
        };
        let new_kind = match new_revision.as_deref() {
            None => "worktree",
            Some("") => "index",
            Some(_) => "commit",
        };
        // Even --no-ext-diff/--no-textconv do not disable clean/process
        // filters when Git reads the worktree. Override every configured
        // filter command without evaluating it or trusting attribute names.
        let keys = kortex_repo::git(&self.root, &["config", "--null", "--name-only", "--list"])?;
        let mut filter_overrides = vec![];
        for key in keys
            .split_terminator('\0')
            .filter(|key| key.starts_with("filter."))
        {
            if key.ends_with(".clean") || key.ends_with(".process") || key.ends_with(".required") {
                filter_overrides.push("-c".to_owned());
                filter_overrides.push(format!(
                    "{key}={}",
                    if key.ends_with(".required") {
                        "false"
                    } else {
                        ""
                    }
                ));
            }
        }
        let diff = |extra: &[&str], paths: &[&str]| -> Result<String> {
            let mut command: Vec<&str> = filter_overrides.iter().map(String::as_str).collect();
            command.extend([
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-relative",
                "--find-renames",
                // Recursive dirty checks can execute filters from nested repositories.
                "--ignore-submodules=dirty",
                "--submodule=short",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--line-prefix=",
            ]);
            command.extend_from_slice(extra);
            command.extend(revisions.iter().map(String::as_str));
            command.push("--");
            command.extend_from_slice(paths);
            kortex_repo::git(&self.root, &command)
        };
        let omitted_submodule_worktrees = new_kind == "worktree"
            && kortex_repo::git(&self.root, &["ls-files", "--stage", "-z"])?
                .split_terminator('\0')
                .any(|entry| entry.starts_with("160000 "));
        let raw = diff(&["--name-status", "-z"], &[])?;
        let mut parts = raw.split_terminator('\0');
        let mut changes = vec![];
        let mut changed = BTreeSet::new();
        let mut warnings = BTreeSet::new();
        let mut partial = omitted_submodule_worktrees;
        if omitted_submodule_worktrees {
            warnings.insert("submodule working-tree changes are not inspected: query each submodule separately; recursive Git dirty checks may execute nested filters".to_owned());
        }
        while let Some(status) = parts.next() {
            let old = parts.next().ok_or_else(|| {
                Error::new("INVALID_DIFF", "incomplete Git name-status output", 2)
            })?;
            let path = if status.starts_with(['R', 'C']) {
                parts
                    .next()
                    .ok_or_else(|| Error::new("INVALID_DIFF", "incomplete Git rename output", 2))?
            } else {
                old
            };
            let paths = if old == path {
                vec![old]
            } else {
                vec![old, path]
            };
            let patch = diff(
                &[
                    "--unified=0",
                    "--inter-hunk-context=0",
                    "--diff-algorithm=myers",
                    "--no-indent-heuristic",
                ],
                &paths,
            )?;
            let mut hunks = vec![];
            let mut old_ranges = vec![];
            let mut new_ranges = vec![];
            let mut header_old = None;
            let mut header_new = None;
            let mut in_hunk = false;
            for line in patch.lines() {
                if line.starts_with("diff --git ") {
                    header_old = None;
                    header_new = None;
                    in_hunk = false;
                } else if !in_hunk && let Some(raw) = line.strip_prefix("--- ") {
                    header_old = Some(patch_path(raw, "a/")?);
                } else if !in_hunk && let Some(raw) = line.strip_prefix("+++ ") {
                    header_new = Some(patch_path(raw, "b/")?);
                }
                if !line.starts_with("@@ ") {
                    continue;
                }
                in_hunk = true;
                // Restricting a rename diff to its two paths can still include
                // another addition at the old path. Never mix those hunks.
                let expected_old = (!status.starts_with('A')).then_some(old);
                let expected_new = (!status.starts_with('D')).then_some(path);
                if header_old.as_ref().map(|p| p.as_deref()) != Some(expected_old)
                    || header_new.as_ref().map(|p| p.as_deref()) != Some(expected_new)
                {
                    continue;
                }
                let mut words = line.split_whitespace().skip(1);
                let before = words.next().and_then(|word| Lines::parse(word, '-'));
                let after = words.next().and_then(|word| Lines::parse(word, '+'));
                let (Some(before), Some(after)) = (before, after) else {
                    return Err(Error::new(
                        "INVALID_DIFF",
                        "invalid Git hunk coordinates",
                        2,
                    ));
                };
                old_ranges.extend(before.range());
                new_ranges.extend(after.range());
                hunks.push(json!({"old_start":before.start,"old_count":before.count,"new_start":after.start,"new_count":after.count}));
            }
            let mut side_hash = |revision: Option<&str>, path: &str, absent: bool| {
                if absent {
                    return None;
                }
                let content = match revision {
                    Some(revision) => kortex_repo::git(
                        &self.root,
                        &["cat-file", "blob", &format!("{revision}:{path}")],
                    ),
                    None => {
                        let attributes = kortex_repo::git(
                            &self.root,
                            &["check-attr", "-z", "working-tree-encoding", "--", path],
                        );
                        let encoding_safe = attributes.as_ref().is_ok_and(|raw| {
                            matches!(
                                raw.split_terminator('\0').nth(2),
                                Some("unspecified" | "unset")
                            )
                        });
                        let link = self.root.join(path).symlink_metadata();
                        if !encoding_safe {
                            Err(Error::new(
                                "DIFF_ENCODING",
                                "Git encoding conversion prevents raw-source coordinate verification",
                                7,
                            ))
                        } else if link.is_ok_and(|m| m.file_type().is_symlink()) {
                            Err(Error::new(
                                "PATH_REFUSED",
                                "Git symlink is not source text",
                                8,
                            ))
                        } else {
                            kortex_repo::read_confined(
                                &self.root,
                                path,
                                self.config.repository.max_file_size_bytes,
                            )
                        }
                    }
                };
                match content {
                    Ok(content)
                        if content.len() as u64 <= self.config.repository.max_file_size_bytes =>
                    {
                        Some(hash(content))
                    }
                    Ok(_) => {
                        warnings.insert(format!(
                            "{path}: diff side exceeds source size policy; mapping unavailable"
                        ));
                        None
                    }
                    Err(error) => {
                        warnings
                            .insert(format!("{path}: cannot verify diff side ({})", error.code));
                        None
                    }
                }
            };
            let old_hash = side_hash(
                old_revision.as_deref(),
                old,
                status.starts_with('A') || old_revision.is_none(),
            );
            let new_hash = side_hash(new_revision.as_deref(), path, status.starts_with('D'));
            let mut ids = BTreeSet::new();
            let mut indexed_sides = BTreeSet::new();
            let mut indexed_file_ids = vec![];
            let mut ranges = vec![];
            for file in self
                .snapshot
                .files
                .iter()
                .filter(|f| f.path == old || f.path == path)
            {
                indexed_file_ids.extend(
                    file.symbols
                        .iter()
                        .filter(|s| s.kind == "file")
                        .map(|s| &s.symbol_id),
                );
                let selected = if file.path == path && new_hash.as_ref() == Some(&file.content_hash)
                {
                    indexed_sides.insert("new");
                    Some(&new_ranges)
                } else if file.path == old && old_hash.as_ref() == Some(&file.content_hash) {
                    indexed_sides.insert("old");
                    Some(&old_ranges)
                } else {
                    None
                };
                let Some(selected) = selected else {
                    continue;
                };
                ranges.extend(selected.iter().copied());
                let overlaps: Vec<_> = file
                    .symbols
                    .iter()
                    .filter(|s| {
                        s.kind != "file"
                            && selected.iter().any(|(start, end)| {
                                s.span.start_line <= *end && s.span.end_line >= *start
                            })
                    })
                    .collect();
                if overlaps.is_empty() {
                    ids.extend(
                        file.symbols
                            .iter()
                            .filter(|s| s.kind == "file")
                            .map(|s| s.symbol_id.clone()),
                    );
                } else {
                    ids.extend(overlaps.into_iter().map(|s| s.symbol_id.clone()));
                }
            }
            let mapped = !indexed_sides.is_empty();
            if !mapped {
                partial = true;
                warnings.insert(format!("{path}: indexed content matches neither available diff side; no changed symbols inferred"));
            }
            ranges.sort_unstable();
            ranges.dedup();
            changed.extend(ids.iter().cloned());
            changes.push(json!({"status":status,"old_path":old,"path":path,"hunks":hunks,"old_line_ranges":old_ranges,"new_line_ranges":new_ranges,"line_ranges":ranges,"indexed_sides":indexed_sides,"mapping_status":if mapped {"content_verified"} else {"unavailable"},"partial":!mapped,"indexed_file_ids":indexed_file_ids,"symbol_ids":ids}));
        }
        let limits = self.limits(args)?;
        let mut affected = BTreeSet::new();
        let mut truncated = changed.len() > limits.max_nodes;
        for id in changed.iter().take(limits.max_nodes) {
            let traversal =
                kortex_graph::traverse(&self.snapshot, id, Direction::Upstream, &limits, false)?;
            truncated |= traversal.truncated;
            for item in traversal.items {
                if affected.contains(&item.symbol_id) {
                    continue;
                }
                if affected.len() < limits.max_nodes {
                    affected.insert(item.symbol_id);
                } else {
                    truncated = true;
                }
            }
        }
        let evidenced_tests = super::test_map::evidenced_tests(
            self,
            &affected,
            std::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms),
        )?;
        let likely_tests: Vec<_> = affected
            .iter()
            .filter(|id| {
                evidenced_tests.contains_key(*id)
                    || self.snapshot.symbol(id).is_some_and(|s| s.kind == "test")
            })
            .filter_map(|id| self.snapshot.symbol(id))
            .collect();
        Ok(
            json!({"index_head":self.snapshot.git_head,"diff_base":args.get("base"),"diff_head":args.get("head"),"resolved_diff_base":old_revision,"resolved_diff_head":new_revision,"diff_base_kind":old_kind,"diff_head_kind":new_kind,"staged":staged,"unstaged":unstaged,"changes":changes,"changed_symbols":changed.iter().filter_map(|id|self.snapshot.symbol(id)).collect::<Vec<_>>(),"affected_symbols":affected.iter().filter_map(|id|self.snapshot.symbol(id)).collect::<Vec<_>>(),"likely_tests":likely_tests,"partial":partial,"warnings":warnings,"truncated":truncated}),
        )
    }
}
