//! Additive SDK authority boundary; legacy discovery deliberately remains separate.
use crate::{ConfinedReader, DiscoveryStats, DiscoveryWalk, Inventory, discover_prepared};
use ignore::{
    gitignore::{Gitignore, GitignoreBuilder},
    overrides::{Override, OverrideBuilder},
};
use kortex_config::Config;
use kortex_core::{Error, RequestControl, Result};
use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_POLICY_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINES: usize = 32768;
const MAX_LINE_BYTES: usize = 4096;
const MAX_DEPTH: usize = 128;
const MAX_DIRECTORIES: usize = 16384;
const MAX_COMPILATIONS: usize = 2048;
const MAX_EXTERNAL: usize = 1024;
const MAX_PATH_BYTES: usize = 32768;
const MAX_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRACKED_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRACKED_NAMES: usize = 1_000_000;

/// Precedence is category-first, then nearest matching base, then last line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PolicyCategory {
    Custom,
    Ignore,
    GitIgnore,
    GitExclude,
    Global,
}
impl PolicyCategory {
    fn slot(self) -> Option<usize> {
        match self {
            Self::Custom => Some(0),
            Self::Ignore => Some(1),
            Self::GitIgnore => Some(2),
            Self::GitExclude => Some(3),
            Self::Global => None,
        }
    }
}

/// Explicit bytes acquired by a TRUSTED SDK caller, not a path grant. No serde.
#[derive(Clone, Debug)]
pub struct AuthorizedPolicy {
    pub origin: PathBuf,
    pub category: PolicyCategory,
    pub matching_base: PathBuf,
    pub bytes: Vec<u8>,
}

/// Absolute ancestor metadata in farthest-to-nearest order, including empty frames.
#[derive(Clone, Debug)]
pub struct PolicyAncestor {
    pub directory: PathBuf,
    pub repository_boundary: bool,
}

/// Caller-authorized worktree metadata. Gitfile/commondir bytes are never followed.
#[derive(Clone, Debug)]
pub struct PolicyRepository {
    pub directory: PathBuf,
    pub external_git_metadata: bool,
    /// Required BLAKE3 of the confined .git file or .git/commondir when present.
    pub marker_digest: Option<String>,
}

/// A caller attestation of COMPLETE inherited/global/worktree policy context.
/// Empty vectors mean explicitly absent inputs, not ambient discovery grants.
/// `standard_local` separately grants demand-driven ordinary in-root Git indexes.
#[derive(Clone, Debug, Default)]
pub struct PolicyContext {
    pub complete: bool,
    pub ancestors: Vec<PolicyAncestor>,
    pub policies: Vec<AuthorizedPolicy>,
    pub repositories: Vec<PolicyRepository>,
    /// Complete Git tracked names, repository-root-relative (including nested repos).
    /// For `standard_local`, these attest inherited names only; selected ordinary
    /// in-root repositories contribute their names through confined index reads.
    pub tracked_paths: Vec<String>,
}

#[derive(Clone, Debug)]
enum Mode {
    RootOnly,
    Standard(Option<PolicyContext>),
    StandardLocal(Option<PolicyContext>),
}
/// Authority is supplied in process, never synthesized from Config or repository data.
#[derive(Clone, Debug)]
pub struct PolicyInputs {
    mode: Mode,
}
impl PolicyInputs {
    pub fn root_only() -> Self {
        Self {
            mode: Mode::RootOnly,
        }
    }
    pub fn standard(context: Option<PolicyContext>) -> Self {
        Self {
            mode: Mode::Standard(context),
        }
    }

    /// Attests complete caller-authorized ancestor/global/inherited tracked context.
    /// `complete` is required and `repositories` must be empty: ordinary in-root
    /// Git indexes are acquired only after each directory is selected by its parent.
    /// Gitfiles, commondir and aliased metadata are not authority grants.
    pub fn standard_local(context: Option<PolicyContext>) -> Self {
        Self {
            mode: Mode::StandardLocal(context),
        }
    }
}

/// Evidence includes absent attempted policies; paths are metadata, never reopened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyEvidence {
    pub origin: PathBuf,
    pub matching_base: PathBuf,
    pub category: Option<PolicyCategory>,
    pub digest: Option<String>,
    pub caller_authorized: bool,
}
/// Observed policy evidence, NOT a complete repository/config/source freshness token.
#[derive(Clone, Debug)]
pub struct PolicyProvenance {
    /// Legacy modes use 1; demand-driven `standard_local` uses 2.
    pub version: u32,
    pub root_only: bool,
    pub digest: String,
    pub evidence: Vec<PolicyEvidence>,
    pub peak_frames: usize,
    pub compiled_policies: usize,
    pub policy_bytes: usize,
}

/// Safe SDK foundation. Standard mode without complete context fails closed.
/// Existing discover()/CLI defaults are NOT changed by this additive API.
pub fn discover_authorized_with_control(
    root: &Path,
    config: &Config,
    inputs: &PolicyInputs,
    control: &RequestControl,
) -> Result<Inventory> {
    authorized(root, config, inputs, control, None).map(|(inventory, _)| inventory)
}

pub fn discover_authorized_profiled(
    root: &Path,
    config: &Config,
    inputs: &PolicyInputs,
    control: &RequestControl,
) -> Result<(Inventory, DiscoveryStats, PolicyProvenance)> {
    let start = std::time::Instant::now();
    let mut stats = DiscoveryStats::default();
    let (inventory, provenance) = authorized(root, config, inputs, control, Some(&mut stats))?;
    stats.total = start.elapsed();
    Ok((inventory, stats, provenance))
}

fn auth(message: &str) -> Error {
    Error::new("POLICY_AUTHORIZATION_REQUIRED", message, 8)
}
fn invalid(message: &str) -> Error {
    Error::new("INVALID_IGNORE_POLICY", message, 8)
}
fn budget() -> Error {
    Error::new(
        "POLICY_BUDGET_EXCEEDED",
        "ignore policy resource bound exceeded",
        8,
    )
}
fn hash_field(digest: &mut blake3::Hasher, bytes: &[u8]) {
    digest.update(&(bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}
fn normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().as_encoded_bytes().len() <= MAX_PATH_BYTES
        && !path.as_os_str().as_encoded_bytes().contains(&0)
        && path.to_str().is_some()
        && path.components().all(|c| {
            matches!(
                c,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        })
}
fn lock(state: &Mutex<State>) -> Result<MutexGuard<'_, State>> {
    state
        .lock()
        .map_err(|_| invalid("ignore policy state poisoned"))
}

fn authorized(
    root: &Path,
    config: &Config,
    inputs: &PolicyInputs,
    control: &RequestControl,
    mut stats: Option<&mut DiscoveryStats>,
) -> Result<(Inventory, PolicyProvenance)> {
    control.check()?;
    let setup = stats.as_ref().map(|_| std::time::Instant::now());
    let context = match &inputs.mode {
        Mode::RootOnly => None,
        Mode::Standard(Some(context)) | Mode::StandardLocal(Some(context)) if context.complete => {
            Some(context)
        }
        Mode::Standard(_) | Mode::StandardLocal(_) => {
            return Err(auth(
                "complete caller-authorized policy context is required",
            ));
        }
    };
    let mut state = if matches!(inputs.mode, Mode::StandardLocal(_)) {
        State::with_local(root, config, context, control, true)?
    } else {
        State::new(root, config, context, control)?
    };
    // The locked walker does not invoke filter_entry at depth zero.
    state.push_directory(root, control)?;
    let state = Arc::new(Mutex::new(state));
    let mut builder = ignore::WalkBuilder::new(root);
    // No path-opening ignore API is used on this path, including parent/global/Git metadata.
    builder
        .hidden(false)
        .ignore(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .follow_links(config.repository.follow_symlinks);
    let callback_state = Arc::clone(&state);
    let callback_control = control.clone();
    builder.filter_entry(move |entry| {
        let Ok(mut state) = callback_state.lock() else {
            return false;
        };
        if state.pending.is_some() {
            return false;
        }
        match state.select(entry, &callback_control) {
            Ok(selected) => selected,
            Err(error) => {
                state.pending = Some(error);
                false
            }
        }
    });
    builder.sort_by_file_name(|a, b| a.cmp(b));
    if let Some(stats) = stats.as_deref_mut() {
        stats.setup = setup.unwrap().elapsed();
    }
    let inventory = discover_prepared(
        root,
        config,
        Some(control),
        stats.as_deref_mut(),
        DiscoveryWalk {
            builder,
            canonicalize_entries: false,
            inspect_git: false,
        },
        |canonical, _| {
            lock(&state)?
                .reader
                .read_resolved(
                    canonical,
                    config.repository.max_file_size_bytes,
                    Some(control),
                )
                .and_then(crate::reader::text)
        },
        || {
            let mut state = lock(&state)?;
            match state.pending.take() {
                Some(error) => Err(error),
                None => control.check(),
            }
        },
    )?;
    let mut state = lock(&state)?;
    state.reader.validate(Some(control))?;
    if let Some(stats) = stats {
        stats.reader = state.reader.stats().clone();
    }
    let provenance = state.finish(control)?;
    Ok((inventory, provenance))
}

struct CompiledExternal {
    base: PathBuf,
    category: PolicyCategory,
    matcher: Gitignore,
}
struct Frame {
    directory: PathBuf,
    has_git: bool,
    rules: [Option<Gitignore>; 4],
    external: [Option<usize>; 4],
}
struct State {
    root: PathBuf,
    reader: ConfinedReader,
    frames: Vec<Frame>,
    external: Vec<CompiledExternal>,
    repositories: Vec<PolicyRepository>,
    root_only: bool,
    local_tracked: bool,
    tracked_cases: std::collections::BTreeMap<String, String>,
    index_bytes: usize,
    tracked_bytes: usize,
    tracked_names: usize,
    include_hidden: bool,
    follow: bool,
    overrides: Override,
    pending: Option<Error>,
    evidence: Vec<PolicyEvidence>,
    evidence_bytes: usize,
    bytes: usize,
    lines: usize,
    compilations: usize,
    directories: usize,
    peak_frames: usize,
    context_digest: String,
}
impl State {
    fn new(
        root: &Path,
        config: &Config,
        context: Option<&PolicyContext>,
        control: &RequestControl,
    ) -> Result<Self> {
        Self::with_local(root, config, context, control, false)
    }

    fn with_local(
        root: &Path,
        config: &Config,
        context: Option<&PolicyContext>,
        control: &RequestControl,
        local_tracked: bool,
    ) -> Result<Self> {
        if local_tracked && context.is_none_or(|c| !c.complete || !c.repositories.is_empty()) {
            return Err(auth(
                "local tracked acquisition requires complete inherited context and no repository grants",
            ));
        }
        if !normal_absolute(root) {
            return Err(invalid("root must be an absolute bounded UTF-8 path"));
        }
        let mut overrides = OverrideBuilder::new(root);
        let mut override_bytes = 0usize;
        let override_count = config
            .repository
            .include
            .len()
            .saturating_add(config.repository.exclude.len());
        if override_count > MAX_LINES {
            return Err(budget());
        }
        for (patterns, exclude) in [
            (&config.repository.include, false),
            (&config.repository.exclude, true),
        ] {
            for pattern in patterns {
                control.check()?;
                override_bytes = override_bytes.saturating_add(pattern.len());
                if pattern.len() > MAX_LINE_BYTES || override_bytes > MAX_TOTAL_BYTES {
                    return Err(budget());
                }
                let pattern = if exclude {
                    format!("!{pattern}")
                } else {
                    pattern.clone()
                };
                overrides
                    .add(&pattern)
                    .map_err(|_| invalid("invalid configured override glob"))?;
            }
        }
        let mut state = Self {
            root: root.to_owned(),
            reader: ConfinedReader::new(root)?,
            frames: vec![],
            external: vec![],
            repositories: vec![],
            root_only: context.is_none(),
            local_tracked,
            tracked_cases: Default::default(),
            index_bytes: 0,
            tracked_bytes: 0,
            tracked_names: 0,
            include_hidden: config.repository.include_hidden,
            follow: config.repository.follow_symlinks,
            overrides: overrides
                .build()
                .map_err(|_| invalid("override compilation failed"))?,
            pending: None,
            evidence: vec![],
            evidence_bytes: 0,
            bytes: override_bytes,
            lines: override_count,
            compilations: 0,
            directories: 0,
            peak_frames: 0,
            context_digest: String::new(),
        };
        let mut digest = blake3::Hasher::new();
        digest.update(if local_tracked {
            b"kortex-authorized-policy-standard-local-v2"
        } else {
            b"kortex-authorized-policy-v1"
        });
        hash_field(&mut digest, root.as_os_str().as_encoded_bytes());
        digest.update(&[
            u8::from(state.root_only),
            u8::from(state.include_hidden),
            u8::from(state.follow),
        ]);
        for patterns in [&config.repository.include, &config.repository.exclude] {
            digest.update(&(patterns.len() as u64).to_le_bytes());
            for pattern in patterns {
                hash_field(&mut digest, pattern.as_bytes());
            }
        }
        if let Some(context) = context {
            state.context(context, control, &mut digest)?;
        }
        state.context_digest = digest.finalize().to_hex().to_string();
        Ok(state)
    }

    fn context(
        &mut self,
        context: &PolicyContext,
        control: &RequestControl,
        digest: &mut blake3::Hasher,
    ) -> Result<()> {
        if context.policies.len() > MAX_EXTERNAL
            || context.ancestors.len() >= MAX_DEPTH
            || context.repositories.len() > MAX_EXTERNAL
            || context.tracked_paths.len() > 1_000_000
        {
            return Err(budget());
        }
        for count in [
            context.ancestors.len(),
            context.repositories.len(),
            context.policies.len(),
            context.tracked_paths.len(),
        ] {
            digest.update(&(count as u64).to_le_bytes());
        }
        let mut previous: Option<&Path> = None;
        for ancestor in &context.ancestors {
            control.check()?;
            if !normal_absolute(&ancestor.directory)
                || ancestor.directory == self.root
                || !self.root.starts_with(&ancestor.directory)
                || previous
                    .is_some_and(|p| p == ancestor.directory || !ancestor.directory.starts_with(p))
            {
                return Err(auth("invalid or unordered ancestor matching context"));
            }
            previous = Some(&ancestor.directory);
            hash_field(digest, ancestor.directory.as_os_str().as_encoded_bytes());
            digest.update(&[u8::from(ancestor.repository_boundary)]);
            self.frames.push(Frame {
                directory: ancestor.directory.clone(),
                has_git: ancestor.repository_boundary,
                rules: Default::default(),
                external: [None; 4],
            });
        }
        for repository in &context.repositories {
            control.check()?;
            if !normal_absolute(&repository.directory)
                || !repository.directory.starts_with(&self.root)
                || self
                    .repositories
                    .iter()
                    .any(|r| r.directory == repository.directory)
            {
                return Err(auth("invalid or duplicate repository boundary context"));
            }
            hash_field(digest, repository.directory.as_os_str().as_encoded_bytes());
            digest.update(&[u8::from(repository.external_git_metadata)]);
            hash_field(
                digest,
                repository.marker_digest.as_deref().unwrap_or("").as_bytes(),
            );
            self.repositories.push(repository.clone());
        }
        let mut cases = std::collections::BTreeMap::new();
        let mut tracked_bytes = 0usize;
        for path in &context.tracked_paths {
            control.check()?;
            tracked_bytes = tracked_bytes.saturating_add(path.len());
            if tracked_bytes > 64 * 1024 * 1024 {
                return Err(budget());
            }
            if Path::new(path).is_absolute()
                || path.is_empty()
                || path.len() > MAX_PATH_BYTES
                || path.as_bytes().contains(&0)
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(auth("invalid authorized tracked path"));
            }
            if let Some(old) = cases.insert(path.to_lowercase(), path)
                && old != path
            {
                return Err(Error::new(
                    "CASE_COLLISION",
                    "authorized Git tracked names collide",
                    8,
                ));
            }
            hash_field(digest, path.as_bytes());
            if self.local_tracked {
                self.track(path.clone(), control)?;
            }
        }
        for policy in &context.policies {
            control.check()?;
            if !normal_absolute(&policy.origin) || !normal_absolute(&policy.matching_base) {
                return Err(auth(
                    "external origins and matching bases must be absolute UTF-8 paths",
                ));
            }
            let ancestor = self
                .frames
                .iter()
                .position(|f| f.directory == policy.matching_base);
            let worktree_exclude = policy.category == PolicyCategory::GitExclude
                && self
                    .repositories
                    .iter()
                    .any(|r| r.external_git_metadata && r.directory == policy.matching_base);
            if policy.category != PolicyCategory::Global && ancestor.is_none() && !worktree_exclude
            {
                return Err(auth(
                    "external policy lacks an authorized ancestor or worktree matching base",
                ));
            }
            if self
                .external
                .iter()
                .any(|p| p.base == policy.matching_base && p.category == policy.category)
            {
                return Err(auth("duplicate policy category at one matching base"));
            }
            let matcher = self.compile(
                &policy.origin,
                &policy.matching_base,
                policy.category,
                &policy.bytes,
                true,
                control,
            )?;
            let index = self.external.len();
            self.external.push(CompiledExternal {
                base: policy.matching_base.clone(),
                category: policy.category,
                matcher,
            });
            if let (Some(frame), Some(slot)) = (ancestor, policy.category.slot()) {
                self.frames[frame].external[slot] = Some(index);
            }
        }
        Ok(())
    }

    fn track(&mut self, path: String, control: &RequestControl) -> Result<()> {
        control.check()?;
        self.tracked_bytes = self.tracked_bytes.saturating_add(path.len());
        self.tracked_names = self.tracked_names.saturating_add(1);
        if self.tracked_bytes > MAX_TRACKED_BYTES || self.tracked_names > MAX_TRACKED_NAMES {
            return Err(budget());
        }
        if Path::new(&path).is_absolute()
            || path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path.as_bytes().contains(&0)
            || Path::new(&path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(auth("invalid authorized tracked path"));
        }
        // Kept separate from current-file collision checks: unstaged case-only
        // renames are valid, and identical names in conflict stages are repeated.
        match self.tracked_cases.entry(path.to_lowercase()) {
            std::collections::btree_map::Entry::Occupied(old) if old.get() != &path => {
                return Err(Error::new("CASE_COLLISION", "Git tracked names collide", 8));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(path);
            }
        }
        Ok(())
    }

    fn tracked_index(
        &mut self,
        directory: &Path,
        git_relative: &Path,
        depth: usize,
        control: &RequestControl,
    ) -> Result<()> {
        let relative = git_relative.join("index");
        let before = self.metadata(&relative, depth + 1, control)?;
        if before == Some(crate::reader::PolicyNode::Alias) {
            return Err(Error::new(
                "PATH_REFUSED",
                "aliased Git index is unsupported",
                8,
            ));
        }
        // The read's own root/parent/leaf checks are mandatory; the type probe
        // is not a preflight permission to reopen through the ambient filesystem.
        let remaining = MAX_TRACKED_BYTES.saturating_sub(self.index_bytes);
        let bytes = self
            .reader
            .policy_bytes(&relative, depth + 1, remaining as u64, control)?;
        let digest = if let Some(bytes) = bytes {
            self.index_bytes += bytes.len();
            let decoded = crate::git_index::decode(&bytes, Some(control))?;
            let base = directory
                .strip_prefix(&self.root)
                .map_err(|_| auth("tracked directory escaped lexical root"))?;
            let prefix = kortex_core::public_path(base);
            for path in decoded.paths {
                control.check()?;
                let len = path
                    .len()
                    .saturating_add(prefix.len())
                    .saturating_add(usize::from(!prefix.is_empty()));
                if len > MAX_PATH_BYTES
                    || len > MAX_TRACKED_BYTES.saturating_sub(self.tracked_bytes)
                    || self.tracked_names >= MAX_TRACKED_NAMES
                {
                    return Err(budget());
                }
                let rebased = if prefix.is_empty() {
                    path
                } else {
                    format!("{prefix}/{path}")
                };
                self.track(rebased, control)?;
            }
            Some(crate::hash_controlled(&bytes, Some(control))?)
        } else {
            None
        };
        if self.metadata(&relative, depth + 1, control)? != before
            || self.reader.policy_node(git_relative, depth, control)?
                != Some(crate::reader::PolicyNode::Directory)
        {
            return Err(Error::new(
                "PATH_REFUSED",
                "Git metadata changed during loading",
                8,
            ));
        }
        self.record(PolicyEvidence {
            origin: self.root.join(relative),
            matching_base: directory.to_owned(),
            category: None,
            digest,
            caller_authorized: false,
        })
    }

    fn record(&mut self, evidence: PolicyEvidence) -> Result<()> {
        self.evidence_bytes = self.evidence_bytes.saturating_add(
            evidence.origin.as_os_str().as_encoded_bytes().len()
                + evidence.matching_base.as_os_str().as_encoded_bytes().len()
                + 128,
        );
        if self.evidence_bytes > MAX_EVIDENCE_BYTES {
            return Err(budget());
        }
        self.evidence.push(evidence);
        Ok(())
    }

    fn marker_bytes(
        &mut self,
        relative: &Path,
        required: usize,
        control: &RequestControl,
    ) -> Result<Vec<u8>> {
        let bytes = self
            .reader
            .policy_bytes(relative, required, MAX_POLICY_BYTES, control)?
            .ok_or_else(|| invalid("repository metadata disappeared"))?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > MAX_TOTAL_BYTES {
            return Err(budget());
        }
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return Err(invalid("repository metadata must be NUL-free UTF-8"));
        }
        Ok(bytes)
    }

    fn compile(
        &mut self,
        origin: &Path,
        base: &Path,
        category: PolicyCategory,
        bytes: &[u8],
        external: bool,
        control: &RequestControl,
    ) -> Result<Gitignore> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        self.compilations += 1;
        if bytes.len() as u64 > MAX_POLICY_BYTES
            || self.bytes > MAX_TOTAL_BYTES
            || self.compilations > MAX_COMPILATIONS
        {
            return Err(budget());
        }
        if bytes.contains(&0) {
            return Err(invalid(&format!(
                "{category:?} {}: NUL in policy",
                origin.display()
            )));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| {
            invalid(&format!(
                "{category:?} {}: policy is not UTF-8",
                origin.display()
            ))
        })?;
        let mut builder = GitignoreBuilder::new(base);
        for (index, line) in text.lines().enumerate() {
            control.check()?;
            self.lines += 1;
            if line.len() > MAX_LINE_BYTES || self.lines > MAX_LINES {
                return Err(budget());
            }
            let line = if index == 0 {
                line.trim_start_matches('\u{feff}')
            } else {
                line
            };
            builder
                .add_line(Some(origin.to_owned()), line)
                .map_err(|_| {
                    invalid(&format!(
                        "{category:?} {}:{}: invalid policy glob",
                        origin.display(),
                        index + 1
                    ))
                })?;
        }
        control.check()?;
        let matcher = builder.build().map_err(|_| {
            invalid(&format!(
                "{category:?} {}: policy compilation failed",
                origin.display()
            ))
        })?;
        control.check()?;
        self.record(PolicyEvidence {
            origin: origin.to_owned(),
            matching_base: base.to_owned(),
            category: Some(category),
            digest: Some(crate::hash_controlled(bytes, Some(control))?),
            caller_authorized: external,
        })?;
        Ok(matcher)
    }

    fn local(
        &mut self,
        origin: &Path,
        base: &Path,
        category: PolicyCategory,
        required: usize,
        control: &RequestControl,
    ) -> Result<Option<Gitignore>> {
        let relative = origin
            .strip_prefix(&self.root)
            .map_err(|_| auth("local policy escaped root"))?;
        match self
            .reader
            .policy_bytes(relative, required, MAX_POLICY_BYTES, control)
            .map_err(|error| {
                Error::new(
                    error.code,
                    format!("{category:?} {}: {}", origin.display(), error.message),
                    error.exit,
                )
            })? {
            Some(bytes) => self
                .compile(origin, base, category, &bytes, false, control)
                .map(Some),
            None => {
                self.record(PolicyEvidence {
                    origin: origin.to_owned(),
                    matching_base: base.to_owned(),
                    category: Some(category),
                    digest: None,
                    caller_authorized: false,
                })?;
                Ok(None)
            }
        }
    }

    fn metadata(
        &mut self,
        relative: &Path,
        required: usize,
        control: &RequestControl,
    ) -> Result<Option<crate::reader::PolicyNode>> {
        let node = self.reader.policy_node(relative, required, control)?;
        // Presence/type is evidence too; no metadata content is interpreted as a grant.
        self.record(PolicyEvidence {
            origin: self.root.join(relative),
            matching_base: self.root.clone(),
            category: None,
            digest: node
                .map(|n| crate::hash_controlled(format!("{n:?}").as_bytes(), Some(control)))
                .transpose()?,
            caller_authorized: false,
        })?;
        Ok(node)
    }

    fn push_directory(&mut self, directory: &Path, control: &RequestControl) -> Result<()> {
        control.check()?;
        self.directories += 1;
        if self.directories > MAX_DIRECTORIES || self.frames.len() >= MAX_DEPTH {
            return Err(budget());
        }
        let canonical = directory.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(Error::new(
                "PATH_REFUSED",
                "directory alias escapes repository",
                8,
            ));
        }
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| invalid("invalid directory base"))?;
        let depth = relative.components().count();
        let git_relative = relative.join(".git");
        let marker = self.metadata(&git_relative, depth, control)?;
        let jj = self.metadata(&relative.join(".jj"), depth, control)?;
        if matches!(
            marker,
            Some(crate::reader::PolicyNode::Alias | crate::reader::PolicyNode::Other)
        ) || matches!(
            jj,
            Some(crate::reader::PolicyNode::Alias | crate::reader::PolicyNode::Other)
        ) {
            return Err(auth(
                "aliased or nonregular repository markers require explicit resolution support",
            ));
        }
        let mut frame = Frame {
            directory: directory.to_owned(),
            has_git: marker.is_some() || jj.is_some(),
            rules: Default::default(),
            external: [None; 4],
        };
        let external_repository = self
            .repositories
            .iter()
            .find(|r| r.directory == directory)
            .cloned();
        if marker == Some(crate::reader::PolicyNode::File) {
            if self.local_tracked {
                return Err(auth("gitfile is unsupported by local tracked acquisition"));
            }
            let bytes = self.marker_bytes(&git_relative, depth, control)?;
            let digest = crate::hash_controlled(&bytes, Some(control))?;
            self.record(PolicyEvidence {
                origin: canonical.join(".git"),
                matching_base: directory.to_owned(),
                category: None,
                digest: Some(digest.clone()),
                caller_authorized: false,
            })?;
            if !external_repository.as_ref().is_some_and(|r| {
                r.external_git_metadata && r.marker_digest.as_deref() == Some(&digest)
            }) {
                return Err(auth(
                    "gitfile requires matching authorized external repository policy context",
                ));
            }
        }
        if marker == Some(crate::reader::PolicyNode::Directory) {
            if !self.local_tracked
                && self
                    .metadata(&git_relative.join("index"), depth + 1, control)?
                    .is_some()
                && self.root_only
            {
                return Err(auth(
                    "Git index case checks require complete caller-authorized tracked names",
                ));
            }
            if self
                .metadata(&git_relative.join("commondir"), depth + 1, control)?
                .is_some()
            {
                if self.local_tracked {
                    return Err(auth(
                        "commondir is unsupported by local tracked acquisition",
                    ));
                }
                let bytes =
                    self.marker_bytes(&git_relative.join("commondir"), depth + 1, control)?;
                let digest = crate::hash_controlled(&bytes, Some(control))?;
                self.record(PolicyEvidence {
                    origin: canonical.join(".git/commondir"),
                    matching_base: directory.to_owned(),
                    category: None,
                    digest: Some(digest.clone()),
                    caller_authorized: false,
                })?;
                if !external_repository.as_ref().is_some_and(|r| {
                    r.external_git_metadata && r.marker_digest.as_deref() == Some(&digest)
                }) {
                    return Err(auth(
                        "commondir requires matching authorized external repository policy context",
                    ));
                }
            } else if external_repository
                .as_ref()
                .is_some_and(|r| r.external_git_metadata)
            {
                return Err(auth(
                    "external repository context does not match an ordinary Git directory",
                ));
            }
            if self.local_tracked {
                self.tracked_index(directory, &git_relative, depth, control)?;
            }
            if !external_repository
                .as_ref()
                .is_some_and(|r| r.external_git_metadata)
            {
                frame.rules[3] = self.local(
                    &canonical.join(".git/info/exclude"),
                    directory,
                    PolicyCategory::GitExclude,
                    depth + 1,
                    control,
                )?;
            }
        }
        if external_repository.is_some() && marker.is_none() && jj.is_none() {
            return Err(auth("authorized repository boundary has no current marker"));
        }
        for (slot, (name, category)) in [
            (".kortexignore", PolicyCategory::Custom),
            (".ignore", PolicyCategory::Ignore),
            (".gitignore", PolicyCategory::GitIgnore),
        ]
        .into_iter()
        .enumerate()
        {
            frame.rules[slot] =
                self.local(&canonical.join(name), directory, category, depth, control)?;
        }
        for (index, external) in self.external.iter().enumerate() {
            if external.base == directory
                && let Some(slot) = external.category.slot()
            {
                frame.external[slot] = Some(index);
            }
        }
        // Original alias spellings remain observations, not newly granted roots.
        if directory.canonicalize()? != canonical {
            return Err(invalid("directory alias changed during policy loading"));
        }
        self.frames.push(frame);
        self.peak_frames = self.peak_frames.max(self.frames.len());
        Ok(())
    }

    fn select(&mut self, entry: &ignore::DirEntry, control: &RequestControl) -> Result<bool> {
        control.check()?;
        let parent = entry
            .path()
            .parent()
            .ok_or_else(|| invalid("walk entry lacks parent"))?;
        while self
            .frames
            .last()
            .is_some_and(|f| f.directory != parent && f.directory.starts_with(&self.root))
        {
            self.frames.pop();
        }
        if self.frames.last().is_none_or(|f| f.directory != parent) {
            return Err(invalid("walker left active policy ancestry"));
        }
        if [
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
        .any(|name| entry.file_name() == *name)
        {
            return Ok(false);
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let override_match = self.overrides.matched(entry.path(), is_dir);
        let selected = if override_match.is_ignore() {
            false
        } else if override_match.is_whitelist() {
            true
        } else {
            match self.policy_match(entry.path(), is_dir) {
                Some(include) => include,
                None => self.include_hidden || !hidden(entry)?,
            }
        };
        if selected && is_dir {
            if !self.follow && entry.path_is_symlink() {
                return Ok(false);
            }
            // Selection uses PARENT policy before the directory can load any rules.
            self.push_directory(entry.path(), control)?;
        }
        Ok(selected)
    }

    fn policy_match(&self, path: &Path, is_dir: bool) -> Option<bool> {
        let any_git = self.frames.iter().any(|f| f.has_git);
        for slot in 0..4 {
            if slot >= 2 && !any_git {
                continue;
            }
            for frame in self.frames.iter().rev() {
                let matcher = frame.rules[slot]
                    .as_ref()
                    .or_else(|| frame.external[slot].map(|i| &self.external[i].matcher));
                if let Some(matcher) = matcher {
                    let matched = matcher.matched(path, is_dir);
                    if !matched.is_none() {
                        return Some(matched.is_whitelist());
                    }
                }
                if slot >= 2 && frame.has_git {
                    break;
                }
            }
        }
        if any_git {
            for external in self
                .external
                .iter()
                .rev()
                .filter(|p| p.category == PolicyCategory::Global)
            {
                let matched = external.matcher.matched(path, is_dir);
                if !matched.is_none() {
                    return Some(matched.is_whitelist());
                }
            }
        }
        None
    }

    fn finish(&mut self, control: &RequestControl) -> Result<PolicyProvenance> {
        control.check()?;
        self.evidence.sort_by(|a, b| {
            (&a.origin, &a.matching_base, a.category, &a.digest).cmp(&(
                &b.origin,
                &b.matching_base,
                b.category,
                &b.digest,
            ))
        });
        let mut digest = blake3::Hasher::new();
        digest.update(self.context_digest.as_bytes());
        for evidence in &self.evidence {
            control.check()?;
            for part in [
                evidence.origin.as_os_str().as_encoded_bytes(),
                evidence.matching_base.as_os_str().as_encoded_bytes(),
                evidence.digest.as_deref().unwrap_or("").as_bytes(),
            ] {
                digest.update(&(part.len() as u64).to_le_bytes());
                digest.update(part);
            }
            digest.update(&[
                evidence.category.map_or(255, |c| c as u8),
                u8::from(evidence.caller_authorized),
            ]);
        }
        Ok(PolicyProvenance {
            version: if self.local_tracked { 2 } else { 1 },
            root_only: self.root_only,
            digest: digest.finalize().to_hex().to_string(),
            evidence: std::mem::take(&mut self.evidence),
            peak_frames: self.peak_frames,
            compiled_policies: self.compilations,
            policy_bytes: self.bytes,
        })
    }
}

fn hidden(entry: &ignore::DirEntry) -> Result<bool> {
    if entry.file_name().as_encoded_bytes().first() == Some(&b'.') {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let metadata = entry
            .metadata()
            .map_err(|_| invalid("hidden-attribute observation failed"))?;
        Ok(metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN
            != 0)
    }
    #[cfg(not(windows))]
    Ok(false)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
