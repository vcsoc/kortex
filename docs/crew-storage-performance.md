# Storage performance and repository-selection wave

## Current continuation: exact read projections (READY storage slice; NOT transport/production acceptance)

READY application-owner contract (new methods on the SAME pinned `ReadSession`; no new connection or SQL escape hatch):

```rust
pub struct SymbolInventoryEntry {
    pub symbol_id: String, pub name: String, pub qualified_name: String,
    pub file_path: String, pub language: String, pub kind: String, pub start_byte: usize,
}
pub struct FileDiagnosticPresence {
    pub id: String, pub path: String, pub has_parser_diagnostics: bool,
}
pub struct OrderedEdge { pub ordinal: usize, pub edge: kortex_core::Edge }
pub struct OrderedEdgePage {
    pub items: Vec<OrderedEdge>, pub next_after_ordinal: Option<usize>,
}
impl ReadSession<'_> {
    pub fn read_projection_source(&self) -> Result<MetadataSource>;
    pub fn symbol_inventory(&self) -> Result<Vec<SymbolInventoryEntry>>;
    pub fn file_inventory_snapshot_order(&self) -> Result<Vec<FileInventoryEntry>>;
    pub fn parser_diagnostic_presence(&self) -> Result<Vec<FileDiagnosticPresence>>;
    pub fn outgoing_edges_ordered(&self, id: &str, after_ordinal: Option<usize>, limit: usize) -> Result<OrderedEdgePage>;
    pub fn incoming_edges_ordered(&self, id: &str, after_ordinal: Option<usize>, limit: usize) -> Result<OrderedEdgePage>;
}
```

READY additional application dependency: `file_inventory_snapshot_order` is IMPLEMENTED (the workspace compile exposed the documented new intelligence request). It preserves exact Snapshot file-vector order using column-only joins, even with invalid Snapshot/ParsedFile JSON. `read_projection_source` returns LegacySnapshot for the known earlier projection manifest version1 lacking file order, so callers can explicitly fall back BEFORE this method rather than hide full materialization. Current transactional projection manifest version2 includes file order; this is NOT a SQLite physical schema/version change. Present version2 missing/malformed file order refuses. No app/transport files edited. `cargo check -p kortex-analysis --all-targets --all-features --locked --offline` now PASS against this exact method; storage59 unit +3 integration PASS.

Inventory preserves Snapshot symbol iteration order; it contains NO doc/attributes/source/full Symbol JSON. Caller MUST apply existing Rust Unicode `to_lowercase` name-prefix/qualified-substring/path lexical semantics before ranking/pagination; this API intentionally does not substitute SQLite ASCII `lower`. Inventory is O(symbols), not bounded pagination. Ordered edges use exact zero-based global Snapshot edge ordinals, exclusive ordinal cursor, one lookahead and limit1..1000; old ID-keyset APIs stay unchanged. Incoming matches actual targets only. Parser presence is authoritative for EVERY file (including explicit false), ordered by path/ID; top-level diagnostics stay in SnapshotMetadata. `read_projection_source` explicitly reports optional new projection vs truthful SAME-Store legacy Snapshot fallback. Present malformed/incomplete new projection refuses; never infer clean parsing. Transactional optional schema1 side tables only; physical version stays1. GitNexus impacts attempted for ReadSession/write_metadata/commit_profiled/DifferentialStore: unindexed, using direct source/caller/test tracing. Publication is HIGH RISK; no default schema2/migration/adoption. First current `cargo test -p kortex-storage --all-features --locked --offline`: 56 unit + 3 integration PASS, zero ignored; doc tests pass. New regression source proves lightweight column-only reads despite invalid Snapshot/ParsedFile/Symbol JSON, Unicode Rust matching equivalence, exact reversed edge order and incoming-target behavior, explicit per-file diagnostics with no top-level diagnostics, pinned replacement, new-projection corruption refusal, same-Store legacy fallback and each side-table publication failure/retry. Existing kill/FULL/FK/privacy/writer suites also passed. Current final validation/profile results follow; these scoped results are NOT parent acceptance.

### Current exact-order evidence and integration boundaries

`read_projection_source` validates manifest identity/counts/types, complete gap-free symbol/edge/file ordinals, diagnostic booleans and normalized joins ONCE per immutable pinned session (cached only after successful validation). Current optional projection manifest version2 requires file order; known version1 validates its existing rows but reports LegacySnapshot because it lacks file-order evidence. True absence of all new generation evidence also reports LegacySnapshot and only an explicit fallback loads the SAME Store's real Snapshot. A present manifest with missing/invalid rows/tables refuses. Partial orphan projection rows WITHOUT their manifest also refuse: do not simulate a legacy fixture by deleting ONLY the new manifest while leaving current evidence. To represent a genuine legacy generation, remove ALL new optional projection tables/rows or use the known version1 manifest without its file-order table. No clean diagnostics or file order is guessed. Existing `file_inventory()` remains path/ID sorted, unmodified.

New storage tests cover original reversed file/symbol/edge order while all duplicate full JSON is invalid; Rust Unicode lexical matching equivalence on lightweight rows; true/false per-file diagnostics without top-level diagnostics; incoming actual-target semantics; exact pagination through 1,201 edges at limits1/500/501/1000; empty generations; current/known-legacy order evidence; corrupt/incomplete evidence; pinned old/new replacement; and each of the FIVE optional projection table insertion failures followed by successful retry. New fixtures are private at creation; no production permission policy, original assertion or parent evidence was relaxed. Existing signed-zero/unsigned/signed/nested-number/diagnostics exact-byte differential tests, all kill/retry/FULL/FK/writer/privacy cases remain enabled and pass.

**Integration checkpoint, not green workspace:** `cargo check -p kortex-analysis --all-targets --all-features --locked --offline` PASS after publishing file order. Full rebuilt CLI + locked/offline ALL-feature workspace test attempt with `--include-ignored` ran the six-language CLI journey successfully and passed every target EXCEPT two NEW analysis continuation tests: `missing_legacy_query_projection_falls_back_on_same_store_and_never_guesses_diagnostics` deletes only the manifest, leaving orphan current evidence (now correctly CORRUPT_INDEX; fixture must represent actual legacy evidence); `actual_cli_query_uses_lazy_dispatch_but_context_keeps_honest_eager_fallback` fails at the anticipated not-yet-adopted transport dispatch. Parent/application/transport own review/fix; NO peer code/assertions edited or waived. Log `target/storage-continuation-workspace-final-tests.log`. Earlier compile-blocked attempt is preserved at `target/storage-continuation-workspace-tests.log`. Parent's newly reported Linux313/frozen-install/explicit-six-language evidence is acknowledged as a separate frozen-source receipt, NOT live-head acceptance.

### Actual fixed-source differential profiling, before any optimization

New OWNED `examples/differential_profile.rs` parses the EXACT historical 10k Python files/ten functions each (110k symbols/320k edges), validates every actual cross-file call, globally resolves every publication, moves sealed owned cached parse facts, reparses ONE real source edit and globally re-resolves. The edit renames module1's f0: module0's source/hash is unchanged, but all ten calls must become unresolved rather than silently retain old semantic edges. Independent COMPLETE fresh reparse+resolution matches the cached path. Arbitrary full Snapshot serialization/order and ALL actual FTS columns are hashed, not just a few query hits; initial/unchanged/updated schema1 and differential facts/FTS agree across all FIVE retained full-size runs. Old pinned readers retain exact old facts/FTS after both replacements. No indexer/application/parse/language edits or indexed-code execution.

New diagnostic APIs: `DifferentialStore::publish_profiled(&mut self, &Snapshot, Option<&Baseline>, &WriterLock) -> Result<(DifferentialChanges, DifferentialTimings)>`; existing `publish` delegates unchanged. Timings cover incoming validation, COMPLETE actual-row/FTS/header fingerprints, baseline binding, retirement, files, nodes/FTS, edges, pointer+FK check and FULL durable COMMIT. `ReadSession::keyword_fingerprint()` and read-only `DifferentialStore::keyword_fingerprint()` return `Result<String>` over every actual FTS identity/name/qualified_name/path/text column, identity-sorted, physical generation tag excluded; digest is BLAKE3 of JSON row-digest vector. Each row digest includes all five exact string values; duplicate/null/nontext identities refuse. No SQL escape hatch, second unpinned reader or exposed source strings. These are O(corpus) diagnostics, not fast query operations.

Commands (separate processes, explicit NEW private scratch stores, release/locked/offline):

```text
cargo build -p kortex-storage --example differential_profile --release --locked --offline
/usr/bin/time -l target/release/examples/differential_profile schema1 10000 10
/usr/bin/time -l target/release/examples/differential_profile differential 10000 10
```

Local aarch64 macOS; source BLAKE3 `3d03be3f05a84b6a4963bf410c12ce055a8a5fac588e88d6815f12cfafa89bf3`, initial edges BLAKE3 `3abc9f84083b92b243ee10e237adace0a44172827362761dbb4e90400306e734`, unchanged from retained reference. Each row below is ONE nonquiescent diagnostic, NOT a percentile, accepted speedup, memory bound or reference edit-to-commit measurement.

| Run | Initial publish ms | Unchanged publish ms | Real one-file/global-fact publish ms | Whole-process max RSS bytes |
|---|---:|---:|---:|---:|
| Schema1 first comparison (before file-order addition) | 12,725.781 | 13,788.879 | 33,847.999 | 2,333,949,952 |
| Differential BEFORE measured fix | 685,073.300 | 3,511.060 | 3,778.757 | 2,559,672,320 |
| Differential first AFTER measured fix | 20,513.528 | 3,600.420 | 3,888.237 | 2,891,972,608 |
| Final schema1 including file order | 7,432.066 | 10,874.603 | 10,016.664 | 2,740,043,776 |
| Final differential repeat | 15,690.552 | 3,580.217 | 3,545.371 | 2,650,144,768 |

The pre-optimization initial node/FTS phase alone was **667,491.472ms**: unindexed FTS DELETE scanned a growing corpus for EACH new node although a complete verified actual-FTS map already proved absence. After measuring and preserving the old executable, the ONLY differential algorithm change skips that empty delete IF `actual.fts` proves the ID absent. It does NOT use file/revision/content hash as an invalidation shortcut, skip serialization of incoming facts, omit FTS/header/actual-row tamper verification, suppress global resolution or bypass constraints. Existing-ID FTS updates/deletes remain; a NEW actual-orphan-FTS regression proves a new node's identity is not assumed absent and its real orphan row is retired exactly once. All actual baseline scans and actual tamper tests remain. Final node/FTS initial phase3,160.990ms; initial edge insertion10,398.325ms remains expensive. No further unmeasured algorithm redesign was made.

Final differential changed-publication phases: incoming validation299.922ms; actual fingerprints1,266.283ms; baseline binding69.584ms; retirement64.203ms; file compare64.728ms; node/FTS compare355.281ms; edge compare608.350ms; pointer/FK751.033ms; durable COMMIT44.943ms. BEFORE publication, actual baseline/cache scan1,190.872ms plus complete global re-resolution421.324ms remain. This is **over5 seconds** even excluding discovery/freshness/watch/verification, NOT the <2s target. Zero-fact-write unchanged publication still costs3.58s because complete verification is retained. RSS remains high/variable and NOT improved/accepted: whole-process peaks include full source/facts, independent reparse, exact serialization/hash comparisons, maps/SQLite/allocator retention and pinned WAL. Two optimized observations are not an accepted distribution; NO p95 claim.

Complete digest equivalence (schema1/before/after/final all identical):

| Generation | Exact Snapshot BLAKE3 | ALL actual FTS BLAKE3 |
|---|---|---|
| initial | `63bfcdfeed194e4f1cad3a58f867ee6e5914273fdee70c822f3f3b550a28f366` | `dbbf0481bc6b567abdd2ae5b9e0fcd89df6026c419df7e794b9fcba0458b6200` |
| unchanged | `38df65b9ea414428a9947b507d9b9fcaaf5d33a6b85899c339a8a252a7f8cab0` | `dbbf0481bc6b567abdd2ae5b9e0fcd89df6026c419df7e794b9fcba0458b6200` |
| updated | `35d56947ca9fa4704d84cffc519e6625c80f6767243c4867912c0ff640211f77` | `92eb7061a6fd6bcf35f0792bb95fc935d85f7b8b9cb801ebe9398e23ce1a8925` |

Logs preserved: `target/storage-parsed-schema1-profile.log`, `target/storage-parsed-differential-before-profile.log`, `target/storage-parsed-differential-after-profile.log`, `target/storage-parsed-schema1-final-profile.log`, `target/storage-parsed-differential-final-profile.log`. Full file/edge hashes and phase/RSS raw observations are in each, independently script-compared across ALL three generations of ALL five runs. Pre-optimization executable retained `target/release/examples/differential_profile-before-fts-absence`, SHA256 `fed480fe57c2407f5ab5e3168be8c84cf51fbd699bf61102234b00f403e57db7`; final measured executable SHA256 `71de79b7db2ed6de90b7cd6f1f60ffe150fd48cf04b92c4032093c51f92b6fe9`. Original resolver/publication baseline executable SHA256 values below remain byte-identical. Accepted-source evidence `target/accepted-wave-source.json` was NOT rewritten.

Current source/ownership freeze: only `crates/kortex-storage/**` + this ledger edited; no dependency changes this continuation, no root/core/application/indexer/manual-lock edits. Owned Rust+manifest24 files, sorted relative path+NUL+contents+NUL SHA256 **`82c881b96ab30c97593d4dd08d1bd91533203cad464d9180857a13ec611641cb`**. Includes new `src/read_projections.rs` and `examples/differential_profile.rs`, not just tracked Git diffs. Full workspace strict all-target/all-feature clippy, ALL-feature audit and owned fmt PASS (`target/storage-continuation-workspace-clippy.log`, `target/storage-continuation-audit.log`); owned diff whitespace PASS. FINAL `(umask 022; cargo test -p kortex-storage --all-features --locked --offline)` and the SAME with umask077 each PASS **59 unit +3 integration**, zero ignored, doc tests pass. Logs `target/storage-continuation-umask022-tests.log` and `target/storage-continuation-umask077-tests.log`. Final owned strict clippy/fmt/diff checks rerun PASS after the file-order addition. Parent independently validates app/transport, benchmark/memory/privacy/platform/coverage and any separately authorized backed-up migration. Schema1 stays default, no adoption/migration/production-ready claim. No delegation, unsafe Rust, model download, source upload, listener, commit/reset/clean/push/publish/deploy or hosted approval.

## Active successor: additive pinned read contract (implemented; scoped checks passing)

Storage ownership only; no nested delegation. GitNexus impact attempted for Store, commit_profiled, location, Registry and repair: unindexed; tracing sources/callers/tests instead. Persistence/placement changes are HIGH RISK; schema1 remains the default and physical version 1. No schema2 migration/adoption is authorized. Existing evidence and binaries below are preserved.

Published signatures for application integration (implemented; scoped native validation below, not parent acceptance):

```rust
impl Store {
    pub fn read_session(&self) -> Result<ReadSession<'_>>;
}
impl ReadSession<'_> {
    pub fn metadata(&self) -> &SnapshotMetadata;
    pub fn metadata_source(&self) -> MetadataSource;
    pub fn file_inventory(&self) -> Result<Vec<FileInventoryEntry>>;
    pub fn get_symbol(&self, id: &str) -> Result<Option<Symbol>>;
    pub fn symbol_candidates(&self, filter: &SymbolFilter<'_>, after_id: Option<&str>, limit: usize) -> Result<ReadPage<Symbol>>;
    pub fn outgoing_edges(&self, id: &str, after_id: Option<&str>, limit: usize) -> Result<ReadPage<Edge>>;
    pub fn incoming_edges(&self, id: &str, after_id: Option<&str>, limit: usize) -> Result<ReadPage<Edge>>;
    pub fn keyword(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>>;
    pub fn load(&self) -> Result<Snapshot>;
}
```

All types re-exported from storage. `ReadSession` borrows an already read-only Store (`Store::open_with_storage(..., false)`); it refuses writable Stores rather than claiming multi-call pinning on an autocommit writer. First metadata read pins the existing WAL transaction; every row/FTS/full fallback uses that SAME connection/generation. Session has no unrestricted SQL. `SnapshotMetadata` owns ALL Snapshot header fields including exact diagnostics plus file/symbol/edge counts. `MetadataSource::{Projection, LegacySnapshot}` explicitly distinguishes a transactional optional schema1 metadata row from full legacy Snapshot fallback (never invented empty diagnostics). No row/table means fallback; malformed present metadata is CORRUPT_INDEX. `FileInventoryEntry { id, path, language, content_hash, size_bytes }` reads columns, not ParsedFile/symbol JSON, ordered by path then ID. Inventory is O(files) and not advertised as bounded pagination. `SymbolFilter { name, qualified_name, name_prefix, file_path, kind }` has optional borrowed strings, all predicates ANDed, case-sensitive literal prefix (not wildcard SQL). Pages have `items` and `next_after_id`; symbol/edge ID ascending ordering, exclusive keyset cursor, limit 1..=1000 (error otherwise), one lookahead, explicit continuation even for ambiguous/unresolved edge facts. Missing get_symbol returns None, no guessed resolution. Cursors MUST stay within this session/filter; application transport cursors remain generation-bound. Incoming matches actual target_id only, not ambiguous candidates. Keyword limit 1..=1000; existing Store keyword remains compatible.

Filesystem coordination: storage now consumes `kortex_repo::placement::{require_derived_data_support, DirectoryAnchor}` and the newly published `open_existing` for full-ancestor validation before derived I/O, descriptor-relative marker/lock/artifact opens, and actual opened-file ACL checks. No held-anchor/ambient SQLite VFS binding or mode-only macOS ACL privacy claim. Unsupported Windows fails before filesystem side effects. Native ACL helper acceptance remains filesystem/parent work; no crate edits outside storage.

Live integration checkpoint: missing experimental module noticed by parent during file creation is resolved with substantive `src/experimental_tests.rs`, not a disabled module. Scoped checks and both ordinary/private-umask tests pass (details below). The initial default-umask fixture failures are recorded below, now resolved after explicit owner authorization to create genuinely private fixtures WITHOUT weakening production ACL policy or original assertions. Baseline binaries and original assertions remain unchanged. A real concurrent-registry journal-disappearance/replacement race exposed by opened-file validation was corrected with bounded revalidation; an existing unsafe artifact is still refused.

Explicit experiment now available at `kortex_storage::experimental::{DifferentialStore, Baseline, DifferentialChanges}`: `create_scratch(&Path)`, `open_scratch(&Path, writable: bool)`, `writer_lock()`, read-only `baseline()`, `load()`, `keyword(&str,usize)`, writable `publish(&Snapshot, Option<&Baseline>, &WriterLock)`. Baseline has only `generation_id()` and consuming `split_cached_files(self)->(Vec<ParsedFile>, Self)`; no public constructor/mutable provenance/Clone or old resolved Edge objects. Scratch directory MUST be new under a private parent. Separate `differential.db`, application ID and physical version 2; Store/CLI/indexer never select it and cannot migrate/adopt it. Current-head file-parse rows omit symbol duplication, node/edge rows have no generation tag; exact header/layout preserve arbitrary file/symbol/edge ordering. Only differing actual serialized facts update rows/FTS; unchanged publication and ordering-only changes write zero fact rows and rebuild no indexes. Baseline fingerprints include actual JSON, normalized FK columns, FTS values and exact header/layout, with canonical directory/inode, private store nonce and pinned head binding; actual rows are rechecked under the write transaction. Complete recomputed facts are mandatory: deliberately unchanged content/revision hashes cannot suppress global edge changes. Read-only pinned transactions retain old facts/header/FTS across replacement and GC. FKs remain ON (deferred endpoint constraints checked before commit), synchronous FULL, one writer lock, building-to-committed atomic head switch. No production migration/adoption or speed/RSS acceptance is proposed. Scoped real-parser equivalence/privacy/failure results follow; large fixed-source diagnostics and parent acceptance remain open.

## Successor final checkpoint: implementation, not acceptance

- The concrete read API above is implemented in `src/reads.rs`, exported by `lib.rs`, and tested by `src/read_tests.rs`. Same read-only Store connection throughout; metadata and inventory demonstrably do not read the large Snapshot/ParsedFile JSON when a valid projection exists. Missing table/row uses the actual legacy Snapshot and exposes `LegacySnapshot`. Corrupt present metadata refuses, not a silent fallback. Generation-pinned FTS, exact lookup, AND filters, literal case-sensitive prefixes, complete ambiguous/unresolved edges and exclusive-ID pagination are covered. Limits bound returned rows, NOT all SQLite work/bytes/time. Inventory remains O(files); SQLite progress cancellation/hooks are not added in this slice.
- `src/experimental.rs` is an isolated CURRENT-HEAD normalized schema2 prototype. No generation column on facts, no per-generation retag, no secondary-index drop/rebuild, no full Snapshot blob. Header/layout preserve file, within-file symbol and edge vector ordering. Changed fact comparison serializes one fact at a time; complete comparison and baseline verification still scan the whole graph. No incremental-latency/memory improvement is claimed. Header/layout and FK columns, not merely file/revision hashes, participate in sealed actual-store fingerprints. A baseline can move out owned ParsedFiles while retaining no old resolved Edge objects. Old readers can retain WAL pages until closed; bounded retained-reader/WAL growth acceptance is not established.
- `src/experimental_tests.rs` includes real Python/Rust/TypeScript parser/resolver facts, exact **serialized-byte** Snapshot equivalence (signed zero, maximum unsigned/minimum signed numbers, nested attributes and diagnostics included), owned-cache full global re-resolution versus fresh reparse and schema1 readback, unchanged and ordering-only zero-fact-write checks, all record duplicate/dangling endpoint/owner/candidate/NaN refusals, foreign/stale/actual-row-tampered baseline refusal, FK-column/FTS/header tampering refusal, real transaction abort/retry, actual deletion/addition/update kill checkpoints and pinned old/new facts+FTS. Child death is injected after retirement, file writes, node/FTS writes, edge writes and pointer publication, before COMMIT; restart takes a fresh writer lock and successfully retries. These are scoped correctness tests, not a crash/power-loss/platform acceptance campaign.
- `src/placement_tests.rs` adds production Store/registry/lock/repair refusal before derived bytes under non-sticky writable ancestors, unchanged refusal of exposed existing artifacts, revalidation on already-open writers and real native macOS allow/inherit-only ACEs. Storage consumes the filesystem owner's descriptor-relative `create_new/open_existing` and root-down policy. Windows/non-Unix fails at the capability gate before filesystem side effects. No first-party unsafe, chmod/adoption of existing directories/files, ACL stripping, ambient-SQLite handle binding or same-UID isolation claim. Canonical root/cache acquisition and trusted administrator/same-user changes remain boundaries.

Final scoped commands: `cargo fmt -p kortex-storage -- --check`, `cargo check -p kortex-storage --all-targets --all-features --locked --offline`, `cargo clippy -p kortex-storage --all-targets --all-features --locked --offline -- -D warnings` PASS. `(umask 022; cargo test -p kortex-storage --locked --offline)` and `(umask 077; cargo test -p kortex-storage --locked --offline)` PASS **53 unit + 3 integration** each, doc tests pass. Existing four-connection/32-entry registry concurrency also passed eight isolated repetitions after correcting actual journal unlink/replacement races; validation retries genuine identity changes at most eight times and fails closed on ongoing churn. No unsafe existing artifact is reclassified as absent. Two cross-only cfg lint failures (never-loop after unsupported gate, Unix-only owner helper) were corrected without lint suppression/assertion removal. Genuine Zig/SQLite Windows all-target/all-feature strict cross-clippy PASS with the environment in `docs/WINDOWS_VALIDATION.md`; no Windows runtime pass implied.

**Default-umask fixture regression RESOLVED after explicit owner instruction.** Initial `(umask 022; ... --lib ...)` was **44 pass / 9 fail**, and selection integration **0 pass / 3 fail**: readable fresh test directories/files were correctly refused before corruption/version assertions. Owner then explicitly authorized genuine private fixture setup while retaining those assertions and distinguishing intended unsafe-placement tests. `src/test_support.rs` now creates NEW directories/files at 0700/0600 using Builder/DirBuilder/OpenOptions at creation, with exclusive new-file creation, no global umask mutation and no chmod/adoption of existing objects. Only affected corruption/version/repair/registry fixture SETUP changed (including integration selection's preexisting-SQLite setup); all original corruption/version/rollback, byte/owner/inode/mode/provenance assertions remain intact. Deliberate group/world-writable/readable, hardlink/symlink and allow/inherited-ACL fixtures keep their unsafe inputs and still prove refusal. The ordinary 022 suite now passes all **56** tests; no production privacy check was relaxed. Existing production-created 0700/0600 state is supported; insecure custom registry/index placement remains refused. Readable but non-writable cache-home ANCESTORS remain allowed by the full descriptor walk; only the actual artifact directory is required private.

Workspace attempt (while other owners are active): all-target/all-feature strict workspace clippy PASS. The first workspace fmt check reported repository-owned `src/experimental.rs`/`src/stamp_tests.rs` differences; after that owner finished, the FINAL `cargo fmt --all -- --check` now PASS. Private-umask workspace tests stopped at the preserved CLI `registered_config_show_and_path_use_original_explicit_origin_without_opening_index` regression (`repository.root` absolute vs `src`). Its corrupt-repair peer regression passed in that run. No peer files/assertions were edited; the CLI failure was not rerun or weakened here. These observations are NOT independent quiescent acceptance or proof of current peer state.

Dependency declarations only: existing workspace `kortex-repo` for shared policy, existing `kortex-parse`/`kortex-languages` dev dependencies for actual-source proof. Cargo automatically reconciled owned package dependency edges; no root manifest or manual lockfile edit. Parent owns the final lock/audit reconciliation, including the filesystem owner's separately added ACL dependency. Retained release baseline binaries are byte-identical to historical SHA-256 values: resolver `38eeeae502ccb18bf0a90fef8dc1d3851f323fcd109317ec81909725d0398a01`, publication `9899d0bd324301941de7ea4c6a8c68aa1700c7359b03fef844dca1c2865d1186`. No new profile or p95/RSS claim; historical diagnostics below are unchanged.

Final owned Rust/manifest freeze: **22 files**, sorted workspace-relative paths, each `path + NUL + contents + NUL`, SHA-256 **`d277943136a36692946835ebb8788773888276a24e38721628a5bbe7139be97b`**. Includes new/untracked sources and the fixture-only support module, not merely tracked Git diffs. Source/code edits stopped at this checkpoint; parent may independently review and consume the additive read APIs. Remaining profile/cancellation/WAL-retention/platform/coverage/integration gates are explicit; no production-ready claim.

### Migration design only: explicitly NOT wired or proposed for adoption

A future separately approved v1->v2 conversion must first acquire the validated repository writer lock, keep source state quiescent, and create a unique private **complete SQLite backup including committed WAL pages** (not copy only index.db). Sync it and verify integrity/version/repository/head before conversion. Build schema2 in a separate new private staging directory; preserve all header/layout/fact values and compare complete canonical serialized Snapshot bytes/fingerprints, FK integrity, counts and FTS against the backed-up v1 head. Conversion needs its own measured memory plan (the current v1 blob can require eager materialization), interruption/ENOSPC/kill and independent platform tests. Only after parent explicitly approves format dispatch/migration AND acceptance may an audited atomic format-pointer/rename protocol switch to the staged store with required file/directory fsync; all v1 DB/WAL/SHM/markers and the backup remain recoverable and untouched on failure. Current Store reads no such format pointer and rejects newer physical schemas; prototype scratch state is NEVER adopted as a production index. No migration function/default change is implemented. Same-UID/administrator races, filesystem crash guarantees, native ENOSPC/power-loss behavior, refreshable critical coverage, actual Windows/Linux runs and quiescent reference distributions remain gates.

Private/proprietary; storage-only ownership plus this document. No delegation, publication, uploads, downloads, commits or destructive Git operations. Runtime completion is not parent acceptance.

## Immediate additive API contract (implemented and scoped validation passed)

```rust
use kortex_storage::registry::{Registry, RepositorySelection};
pub struct RepositorySelection {
    pub invocation_root: std::path::PathBuf,
    pub explicit_config: Option<std::path::PathBuf>,
    pub effective_root: std::path::PathBuf,
}
impl Registry {
    pub fn register_with_selection(&self, s: &Snapshot, selection: &RepositorySelection) -> Result<()>;
    pub fn selection(&self, id: &str) -> Result<Option<RepositorySelection>>;
}
```

Also re-exported as `kortex_storage::RepositorySelection`. All paths must already be absolute; `effective_root` must match `snapshot.root` after `kortex_core::public_path` normalization (including Windows extended prefixes). Config/app owners resolve relative explicit config against invocation directory BEFORE registration, retain the invocation root used for config loading, and pass the already-resolved effective identity. Registry stores provenance paths only, never config contents/environment/secret values. `RepositorySelection` derives Debug, Clone, PartialEq, Eq, Serialize, Deserialize; unknown serialized fields refused. Side-table migration leaves existing registry/list shape intact; legacy or missing IDs return None (no ancestor guessing). `register`/`list`/`remove` signatures retained. Legacy `register` at unchanged root retains provenance; changed-root registration clears provenance. `remove` cascades provenance. Registration+provenance updates are atomic. Corrupt/mismatched provenance returns CORRUPT_REGISTRY, not a guessed selection. Remote app must independently validate reloaded effective root against registered root/allowed identity.

Mesh is disabled; attempted `agent_mesh_send` failed with that diagnostic. This file is the cross-worker contract. No API/CLI integration outside storage is owned here.

## Safety and measurement plan

GitNexus impact for Registry and Store::commit returned unindexed; direct production callers, schema, safety/recovery/registry tests traced instead. Preserve all parent permission-policy and cfg builder fixes. No schema version or root manifest/manual lockfile changes. Baseline publication reparses each row insert and deletes prior generations with cascades. Profile scoped prepared statements and explicit child-before-parent deletion while keeping foreign_keys=ON, synchronous=FULL, writer lock, single transaction and pinned readers. No fake selective invalidation.

Quiescent final measurements require parent coordination after all crew activity stops. Existing resolver_scale fixed source fingerprint is BLAKE3 3d03be3f05a84b6a4963bf410c12ce055a8a5fac588e88d6815f12cfafa89bf3 at 10k files/110k symbols/320k edges; prior nonquiescent single samples first commit 15,960.399 ms, replacement 36,573.437 ms. These are historical diagnostics, not new controlled measurements or acceptance. See measured current before/after below. The peer example labels core::hash values sha256, but core::hash uses BLAKE3; this worker does not own that example. Actual SHA-256 of ordered path + NUL + source bytes is independently computed below.

Windows ACL privacy and same-user ancestor replacement remain unresolved; NOFOLLOW is not parent pinning. Security worker owns shared private-ancestor/ACL hooks; no duplicate repo helper implementation here. See docs/WINDOWS_VALIDATION.md.

## Implemented publication changes

- `src/lib.rs::Store::commit` retains its signature and delegates to additive `Store::commit_profiled(&Snapshot, &WriterLock) -> Result<CommitTimings>`. Timings are coarse wall-clock durations, no source/secret/config data.
- `src/publication.rs` uses scoped file/node/FTS/edge prepared statements; drops the extra whole-snapshot serialization String immediately after its SQLite insertion; sorts only borrowed edge pointers by primary ID before insertion (no fact cloning or ordering changes in the persisted Snapshot).
- A bounded connection-local 64 MiB SQLite page cache replaces the approximately 2 MiB default on publication. Spill remains enabled. WAL, foreign_keys=ON, synchronous=FULL, lock validation and COMMIT/checkpoint behavior are unchanged. No unsafe Rust, VACUUM, background listener or disabled constraints.
- Only the five schema-1 NON-UNIQUE secondary lookup indexes are dropped/rebuilt in the same transaction. All primary/unique/FK parent constraints stay present. Old children are retired edge-before-node before replacement insertion, avoiding simultaneous two-generation secondary B-trees; old generation parents remain until the atomic pointer switch. New index definitions exactly match schema 1. Failure/death rolls back rows, pointer, FTS AND DDL. Existing readers use the old WAL schema/pages even while private retirement/index rebuilding occurs. No schema version/migration bump.
- No selective semantic invalidation or unchanged-file/hash shortcut. Every passed fact is still published, including global-resolution changes with unchanged file/revision hashes.
- An exact-edge-row reuse experiment required every normalized column AND serialized fact to match, and updated endpoint FKs against new nodes. It was correct but slower (29.756 s replacement in its nonquiescent trial) and was removed, not represented as an optimization. Current code does NOT reuse graph/FTS rows across generations.
- `integrity` formerly performed correlated node-to-FTS NOT EXISTS against UNINDEXED FTS identity columns, potentially quadratic. It now materializes the generation's non-null identity set once using NOT IN, explicitly rejects null node IDs, and retains count/FK/snapshot/physical verification. Equal-count duplicate/null FTS identity regressions pass. Actual full integrity at 110k nodes completes in 2,814.792 ms in the final fixed-domain diagnostic; no old full-scale integrity latency was measured.

## Current measurements: diagnostic, NOT quiescent acceptance

Local aarch64 macOS, Rust 1.99.1, optimized release. Independent crew/other host builds were active: strict clippy itself waited on build-directory locks; samples varied substantially. One sample per phase, no p95/target attribution, no cold-cache/watch/query/startup claim. Do not claim the <2 s reference update target or bounded production RSS is met.

### Real parser/resolver fixture, exactly fixed source and graph

Command: `/usr/bin/time -l target/release/examples/resolver_scale 10000 10 1 publish replace` (example is peer-owned; only built/executed here, never edited).

- Ordered path + NUL + source **actual SHA-256**: `ee09e1cc2b260ea740ef6760964bb613a5e0b00356a5a5148a65e99ddc18fd91` (independent Python hashlib, 10k deterministic modules, ten functions per module).
- Source BLAKE3 printed under the legacy sha256 label: `3d03be3f05a84b6a4963bf410c12ce055a8a5fac588e88d6815f12cfafa89bf3`.
- Serialized edges BLAKE3, identical before/after: `3abc9f84083b92b243ee10e237adace0a44172827362761dbb4e90400306e734`. All 100k cross-file calls are validated by that existing diagnostic.

| Version | First commit ms | Replacement ms | Whole-process max RSS bytes |
|---|---:|---:|---:|
| Preserved original binary | 12,435.862 | 30,388.914 | 1,550,123,008 |
| Current prepared/bulk-index/sorted-edge binary | 9,414.041 | 10,628.011 | 1,684,176,896 |

Baseline binary SHA-256 `38eeeae502ccb18bf0a90fef8dc1d3851f323fcd109317ec81909725d0398a01`, retained at `target/release/examples/resolver_scale-storage-baseline`. Current diagnostic binary SHA-256 `c02afb591bf07ac5d52c4a1dbf90d20b379588daf329ed9d2228658d5ee4b31e`. Logs `target/storage-publication-baseline.log`, `target/storage-publication-optimized.log`. RSS is NOT improved/accepted; it includes fixture construction, parsing/resolution, hashing, loads and allocator retention. New end-to-end one-file update was not measured by this worker.

### Storage-only fixed-domain fixture, exact before/after facts

`crates/kortex-storage/examples/publication_profile.rs` constructs deterministic inert domain records, NOT parsed source or indexer acceptance. 10k files, 110k symbols, 320k cross-file edges. Initial serialized Snapshot BLAKE3: `aed8a0a968e3af00385791cb4bbb070eaf0670233500dc0c53eecbb36692612b` (old log label snapshot_sha256 is incorrect; the owned example now correctly prints snapshot_blake3). Same root/IDs/data before/after; actual temporary Store directory does not affect the fixture hash. Real Store publication, graph/file readback and changed-term FTS checks are exercised; final diagnostic also runs full integrity.

```text
cargo build -p kortex-storage --example publication_profile --release --locked --offline
/usr/bin/time -l target/release/examples/publication_profile 10000
```

| Version | First ms | Unchanged replacement ms | One-file fact change publication ms |
|---|---:|---:|---:|
| Original per-row execute/cascade | 11,656.953 | 20,015.703 | 25,659.893 |
| Scoped prepared statements + ordered late deletes | 8,611.793 | 18,078.409 | 15,948.896 |
| Final bounded-cache/early-retirement/bulk-index/sorted-edge | 5,643.783 | 7,021.208 | 6,879.467 |

Final replacement phase ms: snapshot serialize 129.224; generation insertion 362.861; index drop 15.501; FTS delete 171.532; edge delete 382.067; node delete 273.762; file delete 53.559; file/node/FTS inserts 827.331; pointer sort 43.960; edge insert 1,262.367; secondary-index rebuild 907.931; publication 340.743; old generation parent deletion 99.906; durable COMMIT/automatic checkpoint 2,150.217. This isolates real FK/index/FTS work without relaxing it. Serialization is not the dominant wall-clock phase here; durable writing and global row replacement remain substantial.

Baseline binary SHA-256 `9899d0bd324301941de7ea4c6a8c68aa1700c7359b03fef844dca1c2865d1186`, preserved `target/release/examples/publication_profile-baseline`. Final measured binary SHA-256 `32b5e9703b1d6e04a89a0e72138298a2155f1bb31259a86bb3ed1d7b137f88dd`; measured before the output-label-only correction. Logs: `target/storage-fixed-domain-baseline.log`, `target/storage-fixed-domain-prepared.log`, `target/storage-fixed-domain-final.log`. Intermediate cache/reuse/retirement/bulk trials are retained in `target/storage-fixed-domain-*.log`, including slower results; they are not hidden as wins. Baseline whole-process max RSS 1,543,700,480 bytes; final 1,625,440,256 bytes (final additionally executes full integrity). No memory improvement claim.

## Correctness/concurrency/failure validation

Final local commands:

```text
cargo fmt -p kortex-storage -- --check
cargo test -p kortex-storage --locked --offline
cargo clippy -p kortex-storage --all-targets --all-features --locked --offline -- -D warnings
```

All PASS: **39 unit + 3 integration = 42 tests**, zero failures, doc tests pass. Guarded child entrypoints are counted as entries, and parent tests actually execute them in isolated processes. One final fmt check first found only newly added test formatting; owned-only cargo fmt corrected it before the complete passing sequence above. No workspace formatting.

New regressions cover transactional rollback on each edge/node/file/generation retirement failure; changed semantic facts/global edge resolution despite deliberately unchanged hashes; duplicate IDs; equal-count null/duplicate FTS corruption; exact five lookup-index definitions restored through failure, restart and successful publication. Real production commit is paused in isolated child processes after retiring old children, after row insertion/before index rebuild, and after pointer switch/before COMMIT. Parent verifies both pinned and newly opened readers still see old snapshot+graph+FTS+index definitions, refuses concurrent writer lock, kills child, verifies restart/lock reacquisition and successful retry; old readers remain coherent after retry. No mock successful publication.

Registry regressions exercise preexisting three-column DB migration, legacy None, restart, explicit config outside invocation ancestry, legacy refresh preservation, changed-root provenance clearing, cascading remove, mismatched/relative path refusal, corruption/unknown-field refusal, actual SQLite trigger failure with no torn generation/provenance, and four concurrent independent registry connections registering 32 selections. A real Windows extended-prefix identity fixture is type-checked only, not executed here.

Genuine SQLite Windows cross-clippy also PASS using the parent's actual Zig compiler adapter (not emulation), the environment in docs/WINDOWS_VALIDATION.md, and:

```text
cargo clippy -p kortex-storage --all-targets --all-features --target x86_64-pc-windows-gnu --locked --offline -- -D warnings
```

No Windows runtime, NTFS lock/crash, ACL/owner privacy or installation acceptance is implied. Existing parent permission_policy_tests and cfg-specific builder fixes remain intact.

## Metadata, dependency resolution and remaining gates

Only new owned dependency declaration is `serde.workspace = true`, already an audited workspace dependency (third-party MIT OR Apache-2.0, not a first-party license). The first parent integration attempt observed the brief import-before-manifest gap; dependency added and final tests/clippy pass. An intermediate --locked build refused concurrent peer manifest drift. A subsequent --offline Cargo build automatically resolved 53 peer backend dependency changes; no lockfile was manually edited. Parent owns reconciliation/audit of that resolution, not this worker. `cargo metadata --no-deps --locked --offline --format-version 1` verifies storage `license=None`, `license_file=../../LICENSE`, `publish=[]`. No first-party unsafe code added; proprietary policy preserved.

Final owned Rust/manifest SHA-256, 16 ordered files, each relative path + NUL + contents + NUL: `0e9e2115c8872e1198189e02e0091122c9e2207dc616b27505b7fdd92bffc44c` (after label correction, before any subsequent parent edits).

**Parent handoff:** implementation slice is stable and ready for independent source review/freeze. Please coordinate the final quiescent acceptance run only after all workers/builds stop, using the unmodified reference harness and fresh trusted binary. Need full reference sample counts, <2 s p95 edit-to-commit target, query/startup/watch distributions, RSS gate, full workspace tests/audit/coverage and actual CLI registered-ID relocated-config regression. App/config/transport owners integrate the proven selection API; no ancestor guessing. No final quiescent or production acceptance is claimed by the diagnostic tables.

Storage still rebuilds globally within schema 1. Durable COMMIT alone exceeds 2 s in one diagnostic; selective row storage or normalized generation-membership redesign would require parent-owned schema/version/backup approval and new equivalence/pinned-reader/failure proof, NOT a hash-based shortcut. Memory is still roughly 1.6 GiB in phase diagnostics. Actual Windows/Linux execution, Windows ACL privacy, private ancestor/parent-pinning hooks and same-user path races remain security/platform-owner work. Actual host-volume ENOSPC/fsync/power-loss/network-filesystem behavior, independent review and refreshed critical coverage remain separate gates. No completion of PRIVATE production P0/P1 is asserted.
