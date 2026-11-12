//! EXPERIMENTAL v1 observations, not a production metadata-freshness proof.
//!
//! Every capture/verification hashes real confined reads. Nanosecond fields do
//! not establish timestamp resolution, local-filesystem semantics, coherency or
//! protection from an administrator. No filesystem is approved for metadata-only
//! reuse. Unknown/coarse/network filesystems therefore take the SAME hash path.
//! A walk is not an atomic filesystem snapshot; concurrent changes can produce
//! SOURCE_CHANGED or uncertainty and callers must not publish those as fresh.
use crate::{Config, Error, RequestControl, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const SOURCE_STAMP_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataV1 {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub mtime_seconds: i64,
    pub mtime_nanoseconds: i64,
    pub ctime_seconds: i64,
    pub ctime_nanoseconds: i64,
}

#[cfg(unix)]
impl MetadataV1 {
    fn capture(value: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            device: value.dev(),
            inode: value.ino(),
            size: value.len(),
            mtime_seconds: value.mtime(),
            mtime_nanoseconds: value.mtime_nsec(),
            ctime_seconds: value.ctime(),
            ctime_nanoseconds: value.ctime_nsec(),
        }
    }
    fn same_object(&self, other: &Self) -> bool {
        self.device == other.device && self.inode == other.inode
    }
}

/// Opaque capture provenance; no public constructor or deserialization shortcut.
/// Clone preserves provenance, not authority to skip future content reads.
#[derive(Clone, Debug)]
pub struct SourceStampV1 {
    root: PathBuf,
    relative: String,
    resolved: PathBuf,
    root_metadata: MetadataV1,
    parent_metadata: Vec<MetadataV1>,
    metadata: MetadataV1,
    content_hash: String,
}
impl SourceStampV1 {
    pub fn version(&self) -> u32 {
        SOURCE_STAMP_VERSION
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn relative(&self) -> &str {
        &self.relative
    }
    pub fn resolved_relative(&self) -> &Path {
        &self.resolved
    }
    pub fn root_metadata(&self) -> &MetadataV1 {
        &self.root_metadata
    }
    pub fn parent_metadata(&self) -> &[MetadataV1] {
        &self.parent_metadata
    }
    pub fn metadata(&self) -> &MetadataV1 {
        &self.metadata
    }
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
    /// Always false until a separately reviewed local-FS coherency proof exists.
    pub fn permits_metadata_reuse(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct StampedSourceV1 {
    source: String,
    stamp: SourceStampV1,
}
impl StampedSourceV1 {
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn stamp(&self) -> &SourceStampV1 {
        &self.stamp
    }
    pub fn into_parts(self) -> (String, SourceStampV1) {
        (self.source, self.stamp)
    }
}

fn changed(message: &str) -> Error {
    Error::new("SOURCE_CHANGED", message, 5)
}

/// Hash a real opened regular source and capture metadata around THAT read.
/// `root` must be canonical and trusted at acquisition, as for `read_confined`.
/// This is an observed hash, not proof of an atomic filesystem snapshot.
/// Unix only for now; Windows has no experimental identity proof here.
pub fn read_source_stamped_v1(
    root: &Path,
    relative: &str,
    max: u64,
    control: &RequestControl,
) -> Result<StampedSourceV1> {
    read_source_impl(root, relative, max, control, || {})
}

#[cfg(unix)]
fn read_source_impl(
    root: &Path,
    relative: &str,
    max: u64,
    control: &RequestControl,
    after_read: impl FnOnce(),
) -> Result<StampedSourceV1> {
    use rustix::fs::{Mode, OFlags, open};
    use std::{fs::File, io::Seek};
    control.check()?;
    let canonical = crate::resolve_confined(root, relative)?;
    let resolved = canonical
        .strip_prefix(root)
        .map_err(|_| changed("resolved source escaped root"))?;
    let anchor = File::from(
        open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    let root_before = MetadataV1::capture(&anchor.metadata()?);
    let (file, parents) = crate::confined::open_anchored_with_parents(&anchor, resolved)?;
    if !file.metadata()?.is_file() {
        return Err(Error::new("PATH_REFUSED", "not a regular source file", 8));
    }
    let before = MetadataV1::capture(&file.metadata()?);
    let bytes = crate::confined::read_opened_bytes(file.try_clone()?, max, Some(control))?;
    control.check()?;
    let content_hash = crate::hash_controlled(&bytes, Some(control))?;
    control.check()?;
    after_read();
    // Coarse filesystems can retain every timestamp across a same-size write.
    // Compare actual bytes again; metadata alone cannot detect that mutation.
    // Two observations still do not provide arbitrary-writer atomicity.
    let mut verification = file.try_clone()?;
    verification.rewind()?;
    let verified = crate::confined::read_opened_bytes(verification, max, Some(control))?;
    if bytes != verified {
        return Err(changed(
            "source content changed around the actual hash read",
        ));
    }
    drop(verified);
    let after = MetadataV1::capture(&file.metadata()?);
    let (named, parents_after) = crate::confined::open_anchored_with_parents(&anchor, resolved)?;
    let parent_metadata: Vec<_> = parents.iter().map(MetadataV1::capture).collect();
    let parent_after: Vec<_> = parents_after.iter().map(MetadataV1::capture).collect();
    let root_after = MetadataV1::capture(&anchor.metadata()?);
    let root_named = MetadataV1::capture(&root.symlink_metadata()?);
    if before != after
        || before != MetadataV1::capture(&named.metadata()?)
        || parent_metadata != parent_after
        || root_before != root_after
        || !root_before.same_object(&root_named)
    {
        return Err(changed(
            "source, parent or root changed around the actual hash read",
        ));
    }
    if bytes.contains(&0) {
        return Err(Error::new("BINARY_FILE", "binary file", 7));
    }
    let source =
        String::from_utf8(bytes).map_err(|_| Error::new("INVALID_ENCODING", "not UTF-8", 7))?;
    control.check()?;
    Ok(StampedSourceV1 {
        source,
        stamp: SourceStampV1 {
            root: root.into(),
            relative: relative.into(),
            resolved: resolved.into(),
            root_metadata: root_before,
            parent_metadata,
            metadata: before,
            content_hash,
        },
    })
}

#[cfg(not(unix))]
fn read_source_impl(
    _root: &Path,
    _relative: &str,
    _max: u64,
    control: &RequestControl,
    _after_read: impl FnOnce(),
) -> Result<StampedSourceV1> {
    control.check()?;
    Err(Error::new(
        "SOURCE_STAMP_UNSUPPORTED",
        "experimental source identity stamps currently require Unix descriptor metadata",
        8,
    ))
}

/// Rehash rather than infer freshness from equal timestamps, including on APFS.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceVerificationV1 {
    pub content_matches: bool,
    pub identity_matches: bool,
    pub metadata_matches: bool,
    pub hash_reads: usize,
}
pub fn verify_source_v1(
    root: &Path,
    stamp: &SourceStampV1,
    max: u64,
    control: &RequestControl,
) -> Result<SourceVerificationV1> {
    let current = read_source_stamped_v1(root, &stamp.relative, max, control)?;
    let current = current.stamp();
    let object = |a: &MetadataV1, b: &MetadataV1| a.device == b.device && a.inode == b.inode;
    Ok(SourceVerificationV1 {
        content_matches: current.content_hash == stamp.content_hash,
        identity_matches: root == stamp.root
            && current.resolved == stamp.resolved
            && object(&current.root_metadata, &stamp.root_metadata)
            && object(&current.metadata, &stamp.metadata)
            && current.parent_metadata.len() == stamp.parent_metadata.len()
            && current
                .parent_metadata
                .iter()
                .zip(&stamp.parent_metadata)
                .all(|(a, b)| object(a, b)),
        metadata_matches: current.metadata == stamp.metadata
            && current.root_metadata == stamp.root_metadata
            && current.parent_metadata == stamp.parent_metadata,
        hash_reads: 1,
    })
}

/// Experimental hash-only inventory baseline. No serialized/persisted format is
/// promised. Capture deliberately re-reads each discovered source to bind stamps;
/// it is an investigation API, NOT a latency optimization or atomic snapshot.
#[derive(Debug)]
pub struct InventoryStampV1 {
    root: PathBuf,
    root_metadata: MetadataV1,
    config_hash: String,
    root_inputs: String,
    nested_inputs: BTreeMap<String, String>,
    files: BTreeMap<String, SourceStampV1>,
    diagnostics: Vec<String>,
}
impl InventoryStampV1 {
    pub fn version(&self) -> u32 {
        SOURCE_STAMP_VERSION
    }
    pub fn files(&self) -> &BTreeMap<String, SourceStampV1> {
        &self.files
    }
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

fn input_hashes(root: &Path, control: &RequestControl) -> Result<BTreeMap<String, String>> {
    let mut builder = ignore::WalkBuilder::new(root);
    let walk_control = control.clone();
    builder
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            if walk_control.check().is_err() {
                return false;
            }
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
            .any(|n| entry.file_name() == *n)
        });
    let mut inputs = BTreeMap::new();
    for entry in builder.build() {
        control.check()?;
        let entry = entry.map_err(|e| Error::new("SOURCE_STAMP_UNCERTAIN", e.to_string(), 7))?;
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            continue;
        }
        if ![
            "go.mod",
            "Cargo.toml",
            "pyproject.toml",
            "tsconfig.json",
            "jsconfig.json",
            "package.json",
            ".gitignore",
            ".ignore",
            ".kortexignore",
            ".kortex.toml",
        ]
        .iter()
        .any(|n| entry.file_name() == *n)
        {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| changed("input escaped root"))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| Error::new("SOURCE_STAMP_UNCERTAIN", "non-UTF8 input path", 7))?;
        let bytes =
            crate::read_confined_bytes_with_control(root, relative, 2 * 1024 * 1024, control)?;
        inputs.insert(
            kortex_core::public_path(Path::new(relative)),
            crate::hash_controlled(&bytes, Some(control))?,
        );
    }
    control.check()?;
    Ok(inputs)
}

#[cfg(unix)]
fn root_metadata(root: &Path) -> Result<MetadataV1> {
    use rustix::fs::{Mode, OFlags, open};
    let file = std::fs::File::from(
        open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    Ok(MetadataV1::capture(&file.metadata()?))
}
#[cfg(not(unix))]
fn root_metadata(_root: &Path) -> Result<MetadataV1> {
    Err(Error::new(
        "SOURCE_STAMP_UNSUPPORTED",
        "experimental inventory identity stamps currently require Unix",
        8,
    ))
}

pub fn capture_inventory_v1(
    root: &Path,
    config: &Config,
    control: &RequestControl,
) -> Result<InventoryStampV1> {
    control.check()?;
    if config.repository.follow_symlinks {
        return Err(Error::new(
            "SOURCE_STAMP_UNCERTAIN",
            "experimental inventory stamps do not yet prove complete manifest coverage through directory symlink aliases; use controlled eager discovery",
            7,
        ));
    }
    let root_before = root_metadata(root)?;
    let root_inputs = crate::inputs::resolution_fingerprint_with_control(root, control)?;
    let nested_inputs = input_hashes(root, control)?;
    let inventory = crate::discover_with_control(root, config, control)?;
    let mut files = BTreeMap::new();
    for source in inventory.files {
        control.check()?;
        let read = read_source_stamped_v1(
            root,
            &source.path,
            config.repository.max_file_size_bytes,
            control,
        )?;
        if read.stamp.content_hash != source.content_hash {
            return Err(changed("source changed since discovery"));
        }
        files.insert(source.path, read.stamp);
    }
    // A second input pass catches policy/manifest changes across source capture.
    if nested_inputs != input_hashes(root, control)?
        || root_inputs != crate::inputs::resolution_fingerprint_with_control(root, control)?
    {
        return Err(changed(
            "ignore/configuration/manifest inputs changed during capture",
        ));
    }
    if root_metadata(root)? != root_before {
        return Err(changed("repository root changed during inventory capture"));
    }
    if files.values().any(|stamp| {
        stamp.root_metadata.device != root_before.device
            || stamp.root_metadata.inode != root_before.inode
    }) {
        return Err(changed(
            "source stamps do not share the captured root identity",
        ));
    }
    control.check()?;
    Ok(InventoryStampV1 {
        root: root.into(),
        root_metadata: root_before,
        config_hash: kortex_core::hash(
            toml::to_string(config).map_err(|e| Error::new("INVALID_CONFIG", e.to_string(), 2))?,
        ),
        root_inputs,
        nested_inputs,
        files,
        diagnostics: inventory.diagnostics,
    })
}

/// Observed source-inventory equality, not whole-generation freshness or an
/// atomic snapshot. No metadata-only reuse is approved by this experimental API.
#[derive(Debug, PartialEq, Eq)]
pub enum InventoryFreshnessV1 {
    UnchangedByHash,
    Changed,
    Uncertain,
}

pub fn verify_inventory_v1(
    root: &Path,
    config: &Config,
    baseline: &InventoryStampV1,
    control: &RequestControl,
) -> Result<InventoryFreshnessV1> {
    let current = capture_inventory_v1(root, config, control)?;
    // Ordinary unsupported files and intentionally skipped source symlinks are
    // inventory policy, not failed reads. All other diagnostics remain uncertain.
    if current
        .diagnostics
        .iter()
        .chain(&baseline.diagnostics)
        .any(|d| !d.ends_with(": unsupported language") && !d.ends_with(": symlink skipped"))
    {
        return Ok(InventoryFreshnessV1::Uncertain);
    }
    let mut same = current.root == baseline.root
        && current.root_metadata.device == baseline.root_metadata.device
        && current.root_metadata.inode == baseline.root_metadata.inode
        && current.config_hash == baseline.config_hash
        && current.root_inputs == baseline.root_inputs
        && current.nested_inputs == baseline.nested_inputs
        && current.diagnostics == baseline.diagnostics
        && current.files.len() == baseline.files.len();
    for (path, stamp) in &current.files {
        control.check()?;
        same &= baseline.files.get(path).is_some_and(|old| {
            old.content_hash == stamp.content_hash
                && old.metadata.device == stamp.metadata.device
                && old.metadata.inode == stamp.metadata.inode
                && old.root_metadata.device == stamp.root_metadata.device
                && old.root_metadata.inode == stamp.root_metadata.inode
                && old.resolved == stamp.resolved
                && old
                    .parent_metadata
                    .iter()
                    .map(|p| (p.device, p.inode))
                    .eq(stamp.parent_metadata.iter().map(|p| (p.device, p.inode)))
        });
    }
    control.check()?;
    Ok(if same {
        InventoryFreshnessV1::UnchangedByHash
    } else {
        InventoryFreshnessV1::Changed
    })
}

#[cfg(all(test, unix))]
#[path = "stamp_tests.rs"]
mod tests;
