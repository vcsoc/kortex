# Confined Ignore-Policy Work: Open Production Gate

Status: **READY - additive authorized SDK foundation; scoped native tests and native/pinned Windows cross-clippy pass.** Legacy discovery/CLI defaults are unchanged. This is NOT default-gap closure, Windows runtime proof, independent acceptance or production readiness.

## Reproduced Boundary Violation

`target/parent-ignore-follow-observation.json` preserves six harmless actual-CLI cases: regular and outside-root symlink variants of `.kortexignore`, `.ignore`, `.gitignore`, each selecting a repository containing `safe.py`. An outside dummy policy containing `safe.py` excludes the source through every policy symlink. No real private user file was used; fixture source was parsed/indexed, never executed.

A follow-up negative control without policy indexes one file. Changing only the outside policy to `unrelated.py` and explicitly setting `repository.follow_symlinks=false` makes all three symlink repositories index one file. The executable SHA256 before and after those four follow-up runs is identical: `ed18023f8e34697f21f6c7808fe8c6e1c56834da16efeea64383776f2270b64c`. Source/build pins are not a frozen reference acceptance run. The earlier malformed-marker probe (`target/parent-ignore-boundary-observation.json`) showed NO direct marker disclosure. Do not claim arbitrary secret logging from these observations.

The mechanism is automatic policy opening by `ignore::WalkBuilder` in repository discovery, independent of `ConfinedReader`. Confined source reads do not authorize repository-controlled outside-root ignore contents. Specifications 248-250 and 515 require both ignore behavior and filesystem confinement.

## Accepted Safety Decisions

Parent rejected root-check amortization. Per-file absolute root observations and all existing parent/leaf descriptor checks remain. Object-capability endpoint-only semantics are not a security-equivalent replacement for the current pathname observations. Ambient enumeration remains a distinct namespace/race boundary.

Only reusable initialized 64KiB read scratch has been implemented. Its actual returned prefix, EOF/size+1 behavior, controls, descriptor checks and hashing remain authoritative. Parent repository tests76/native strict clippy and official pinned1.99.1 Windows-target strict clippy pass. That is not performance or Windows runtime/privacy acceptance.

## SDK Foundation: Frozen Additive Contract

This slice owns only `crates/kortex-repo/**` and this document. Parent owns standard owner-context acquisition, config/indexer integration, default/CLI adoption and acceptance. Legacy discovery entrypoints temporarily retain their existing behavior; this foundation alone does NOT close the reproduced default-path gap.

Exact entrypoints selected before caller integration:

```rust
pub fn discover_authorized_with_control(
    root: &Path, config: &Config, inputs: &PolicyInputs, control: &RequestControl,
) -> kortex_core::Result<Inventory>;
pub fn discover_authorized_profiled(
    root: &Path, config: &Config, inputs: &PolicyInputs, control: &RequestControl,
) -> kortex_core::Result<(Inventory, DiscoveryStats, PolicyProvenance)>;
```

`PolicyInputs::root_only()` explicitly selects no inherited/global authority. `PolicyInputs::standard(context: Option<PolicyContext>)` requires a complete trusted SDK-supplied context; absent/incomplete context returns `POLICY_AUTHORIZATION_REQUIRED`. These types have no serde/config deserialization or filesystem acquisition. External policy origins and matching bases are metadata only: matchers consume supplied bytes and never open those paths. Repository-controlled aliases never inherit external byte authority. Public context fields are now implemented and fixed for caller integration:

- `PolicyContext { complete: bool, ancestors: Vec<PolicyAncestor>, policies: Vec<AuthorizedPolicy>, repositories: Vec<PolicyRepository>, tracked_paths: Vec<String> }`. `complete=true` is a TRUSTED caller attestation, not filesystem discovery by this crate. Empty vectors attest absent inputs. The parent must acquire all relevant inherited/global rules and boundary/case metadata before asserting completeness.
- `PolicyAncestor { directory: PathBuf, repository_boundary: bool }`: absolute strict ancestors, farthest-to-nearest, including empty boundary frames. No ancestor path is opened.
- `AuthorizedPolicy { origin: PathBuf, category: PolicyCategory, matching_base: PathBuf, bytes: Vec<u8> }`; categories `Custom`, `Ignore`, `GitIgnore`, `GitExclude`, `Global`. Non-global external policies must bind a declared ancestor or an explicitly authorized external worktree exclude. Global matching bases are explicit absolute metadata, including owner CWD when required by the locked oracle. Duplicate category/base inputs fail rather than inventing ordering.
- `PolicyRepository { directory: PathBuf, external_git_metadata: bool, marker_digest: Option<String> }`: root-relative authority is NOT added. A gitfile/commondir requires `external_git_metadata=true` and a BLAKE3 matching its actual confined bytes; caller-supplied excludes may be empty by explicit complete attestation. The file's path directives are NEVER followed or used to acquire policy. Aliased/nonregular repository markers explicitly refuse.
- `tracked_paths` are the complete actual Git tracked names in selected-root-relative spelling, including nested repositories, for portable case checks without Git/config subprocess reads. Root-only mode explicitly refuses an existing Git index because silently skipping its tracked-case check would change the old contract. Standard mode needs complete caller data, not a grant to inspect that index.
- `PolicyProvenance { version: u32, root_only: bool, digest: String, evidence: Vec<PolicyEvidence>, peak_frames: usize, compiled_policies: usize, policy_bytes: usize }`; evidence has `origin`, `matching_base`, optional `category`, optional BLAKE3 `digest`, and `caller_authorized`. Absent attempted policies and observed repository-marker presence/type are included. The digest binds mode, override/include-hidden/follow settings, supplied context/boundaries/tracked names and the sorted observed evidence, but is NOT a full source/config/generation freshness token or atomic snapshot.

These types deliberately have no serde implementation and are not Config fields. A repo JSON/TOML value cannot instantiate authority through this API. Standard caller integration and effective freshness/publication revalidation remain parent-owned.

## Proposed Implementation Boundary

Parent owns application adoption; the repository worker owns this additive implementation. Read-only design reviews `4947f01b-1d51-497` and `bf81cc24-776d-458` are completed; neither changed code. Storage/query and config/indexing owners remain separate. No config field, CLI authority flag or external-policy default is selected by this document.

1. Disable automatic repository, ancestor, global and Git-exclude policy content loading; remove custom ignore auto-loading. Never preflight a path and then allow the library to reopen it ambiently.
2. Load bounded UTF-8 policy bytes through one confined root authority, using typed optional-missing classification before native IO errors become generic application errors. Missing is not permission denial, stale ancestry, dangling alias, nonregular file or substitution. Preserve all root/parent/leaf validation around successful and absent outcomes.
3. Build matchers from bytes with `GitignoreBuilder::add_line`, not path-opening `add`. Reproduce BOM/CRLF semantics, bind diagnostics to path/line/category without echoing raw policy contents, and abort on malformed/over-budget policy rather than treating it as permissive empty input.
4. Keep policy state on an active DFS stack, not a retained map for every encountered directory or recursively cloned accumulated patterns. Bound bytes, patterns, depth and compilation work. Select a directory using parent policies before loading its own child policies. Seed root explicitly because the locked walker does not invoke `filter_entry` for depth zero.
5. Coordinate callback errors and source reads through a single confined reader/state, with nonrecursive lock scopes if using Arc/Mutex. Check pending errors after every walker step, including exhaustion, before any inventory/stats publication. A successful partial inventory must not escape failed policy loading.

## Semantic Oracle And Authority Choices Still Required

Locked dependency `ignore 0.4.34` uses category precedence:

`overrides > .kortexignore > .ignore > .gitignore > info/exclude > global`

Nearest directory wins within each category; last matching line wins within a file. Flattening categories is incorrect. Preserve Git-required gating, nested boundaries, hidden fallback/Windows attributes, override/include-whitelist behavior, default exclusions, source aliases/deduplication and negations. Automatic hidden filtering must not preempt a valid whitelist.

Inside-root policy bytes and ordinary inside-root `.git/info/exclude` use confined reads. Authorized internal policy aliases and both source-follow modes need explicit tests. Outside policy aliases cannot gain authority merely because repository configuration names them.

Ancestor policies, repository-directed gitdir/commondir targets, and owner-global policy are distinct authorities. Trusted initial path acquisition is not consent to read arbitrary ancestor-policy symlink targets. Owner environment/home/XDG/system configuration is a separate legitimate trust domain, but the library's existing global loader is unbounded and relative paths can depend on CWD. Do not call it confined. An additive caller authority API can supply bounded authorized external policy bytes/matching bases/categories; its production caller integration must be specified before adopting defaults. Preserve authorized P0 behavior; otherwise return an explicit authorization error, not silently omit inherited rules. Do not re-enable ambient external metadata resolution as a shortcut.

`inputs.rs::fingerprint` already uses confined reads and records unavailable root-policy digests; it does not fix the walker's separate reads or describe all nested/effective policy evidence. Policy provenance/digests must be coordinated with indexer/freshness integration rather than declared a complete token prematurely.

## Additive Tracked-Name Decoder, Not Default Caller Adoption

Parent integrated public `kortex_repo::git_index::decode(bytes, Option<&RequestControl>) -> Result<TrackedNames>` in crates/kortex-repo/src/git_index.rs. It accepts explicitly acquired bytes only: NO path, config, Git process, object loading or filesystem authority. Returns detected ObjectFormat (Sha1/Sha256), index version2/3/4 and ordered UTF-8 names, retaining legitimate repeated paths at ascending conflict stages. Caller must independently acquire/reobserve authorized index/marker dependencies and namespace nested/ancestor names; this API does not attest complete policy context or bind repository config/object format.

Independent read-only audit 0a591155-1e89-499 identified three documented-format gaps, NOT demonstrated policy bypasses or name mismatch versus permissive Git ls-files. Parent verified installed Git2.53 gitformat-index.adoc and corrected exact .git component rejection, ordinary-mode/reserved-bit whitelist (100644/100755/120000/160000, sparse directory still explicitly unsupported), and unsigned-byte path/strict stage ordering without deduplication. New regressions use checksum-valid mutations of ACTUAL Git indexes for all six version/hash pairs plus actual Git-created conflict stages1/2/3. Existing split/sparse/unknown-required-extension, encoding/corruption/optional extension/control/oversize proofs remain. No partial successful names on refusal.

Fixed limits:64MiB input,1M entries,32KiB single name,64MiB aggregate expanded names; these are NOT RSS/allocator proofs. Native hash updates check control per64KiB; entries/extensions and return check it again. SHA1 is ONLY the Git-format corruption checksum, never authentication. Split/sparse/unknown required representations require explicit failure or separately implemented complete expansion, never empty-name fallback.

Maintained MIT/Apache RustCrypto sha1=0.11.0 added OFFLINE; SHA256 reuses already locked0.11.1. Parent compared all pre-existing locked version/source/checksum identities: unchanged, only sha1 package added. Source/dependency evidence target/parent-git-index-adoption-lock-review.json. Repository95tests; rebuiltCLI/fullworkspace ALLfeatures --include-ignored (BOTH actualCLI proofs), strict workspace clippy/fmt/pinnedofflineaudit, pinned1.99.1 Windows CROSSclippy PASS. Logs target/parent-git-index-adoption-*.log; audit/fix resolution target/parent-git-index-review-resolution.json. No Windows runtime, complete platform/coverage/package or default-authority acceptance follows. Prior189-input Linux pass predates this module/lock change. New190-input Cargo/fixture pin SHA2562873bdeb7c6b0115dface6dcdef120c18db7d7d2f90e8012797723a8bad2e872 at target/parent-git-index-adoption-source.json; original six-file SDK handoff freeze remains historical, not current lib.rs bytes after additive export.

## Selected Local And Explicit Operator Acquisition Checkpoint

`PolicyInputs::standard_local(Some(complete_context))` now demand-acquires ordinary in-root indexes ONLY after parent policy selects a directory, independently namespaces nested names and checks collisions separate from current inventory (unstaged case-only renames remain valid). Initial context attests inherited/global/inherited-tracked inputs only; nonempty repositories, gitfiles/commondir/external metadata explicitly refuse. Original standard/root_only constructors and version1 semantics stay unchanged; local acquisition uses version2 dependency evidence and64MiB request-wide index/name byte +1Mname budgets.100k actual Git names are admitted without consuming8MiB policy budget. Ignored nested corrupt/outside metadata is NOT prewalked. Worker12fff72a handoff/source verified by parent; expected concurrent parent reader addition caused a correct worker stop, followed by independent combined revalidation, not a restart.

New `policy_authority::OperatorPolicyScope` and exact `OperatorPolicyFile` are explicit trusted Rust caller capabilities, no serde or automatic repo/config grants. Canonical initially granted directory FD retained, all parent/leaf aliases refuse including inside aliases; typed optional absence never swallows permission/stale/nonregular/control errors. Actual bounded bytes AND hashes re-read every acquisition,1MiB/input lowered by remaining budget. `OperatorGlobalPlan` consumes explicitly supplied ordered selector files, independently supplied target scopes, captured home/CWD and explicit fallback. Pure first-assignment regex exactly matches locked ignore0.4.34 valid UTF8 behavior; every tilde expansion is size-admitted BEFORE allocation. No Git/config includes, process environment capture, target canonicalization or automatic target grants. Selected-but-missing policy does NOT fall back. Full operator environment/ancestor/external acquisition and APPLICATION DEFAULT adoption remain pending.

Independent reviewerffb851ac PROVED two NEW defects in initial additive operator layer: P1 lexical case-sensitive starts_with accepted source-owned selectors under actual case-insensitive spelling, P2 removing ParentDir silently changed effective file. Parent remediation uses actual HELD source-directory identity and CURRENT granted scope ancestry plus SAME actual no-follow observed parent IDs before selector read, not ASCII/Unicode lowercase guesses. Scope below source also refuses by ancestry identity. Windows IDs supplement, NEVER replace no-SHARE_DELETE leases;64bit file-ID collisions conservatively deny, no ReFS/runtime acceptance. Every ParentDir now explicitly refuses POLICY_AUTHORIZATION_REQUIRED before target lookup; no silent reinterpretation as another file. Root/parent/leaf byte-read observations remain, no cached root-check amortization or ambient reopen. Namespace ABA/arbitrary same-UID/hardlink adversaries remain unaccepted, not an atomic filesystem snapshot.

Native regression actually exercises BOTH case-insensitive AND Unicode-normalization aliases with ZERO selector bytes/file opens; absent alias scenarios on other filesystems are explicitly reported unavailable, not fabricated positives. Missing/../rules/link/../rules/../owner/rules refuse, ordinary granted selection and content-change/retry/cancellation/absence controls pass.119repo tests + freshly rebuiltCLI/fullworkspace ALLfeatures --include-ignored/BOTH proofs452PASS/0ignored, strictworkspaceclippy/fmt/pinnedofflineaudit + pinnedWindows CROSSclippy PASS. Original failure modules independently rehashed and preserved, target/parent-operator-policy-original-review; raw evidence target/parent-operator-policy-review-resolution.json. New independent fix verification9201579c launched, no result inferred.193-input Cargo/fixture manifeste2b708acbdd8daba39a54c117fe00b77963f81b64703e70eee25ffbd1f85daec at target/parent-local-operator-policy-source.json is NOT a platform/frozen-copy/production acceptance. Only already-locked regex-automata0.4.18 direct edge added; all375 existing package identities unchanged. Earlier190SDK/189Linux checkpoints stay historical.

## Parent-Verified Global Selector Contract

Parent inspected the actual locked ignore0.4.34 `gitignore.rs` helpers586-708 before selecting caller acquisition. Do NOT invoke these helpers for content acquisition: they ambiently open files with unbounded read_to_end. Their observable selector order is GIT_CONFIG_GLOBAL, HOME/.gitconfig, XDG_CONFIG_HOME/git/config (HOME/.config fallback), GIT_CONFIG_SYSTEM (default /etc/gitconfig), then XDG git/ignore default. Despite the replacement comment, the CODE still falls back to home/XDG when an explicit global file has no matching selector. Preserve source behavior rather than silently following the comment.

Selection is a bytes regex for the FIRST case-insensitive excludesfile assignment, NOT a full section-aware Gitconfig/includes parser. It accepts only its existing non-whitespace token/optional-quote syntax, validates the captured path UTF-8 and replaces EVERY tilde with the captured home string. Relative paths are opened relative to process CWD; global matchers also bind process CWD, not automatically the selected source root. A new caller must not invent full Git include semantics or silently change precedence/anchoring. Owner context is a separate explicitly bounded authority domain; repository aliases cannot inherit its grants. Absent versus permission/dangling/nonregular/oversize outcomes must remain typed, not fall through as harmless absence. Safely rejecting unsupported authority is preferable to silently dropping an applicable inherited rule.

Default integration is still OPEN. Parent is obtaining a focused independent review of selected-directory dynamic acquisition versus typed static-context authorization needs; raw prewalks and unconditional Git subprocesses are not accepted shortcuts. This section is source evidence/design input, not implemented global acquisition.

## Implemented SDK Mechanics

`src/policy.rs` disables automatic hidden/ignore/parent/Git/global/exclude policy loaders ONLY for the additive safe API. It builds matchers exclusively with `GitignoreBuilder::new/add_line/build/matched`, seeds root explicitly, and applies overrides, category precedence, nearest-within-category and Git boundaries before hidden fallback. Selected directories load their own rules only AFTER parent selection. Default excluded names remain unconditional. Active frames are a DFS stack, not a per-directory matcher map or cloned accumulated patterns. External bytes are separately compiled explicit inputs, never repository path grants.

One Arc/Mutex state shares the same `ConfinedReader` for callback policy bytes and source reads; callback and source locks are nonrecursive. `discover_prepared` is the shared old/new source enumeration/read/hash/limit/dedup/case loop. Pending callback failures are checked after EVERY `next()`, including `None`, and before final sorting/publication. No old per-file root/parent/leaf check or bounded EOF/hash/control behavior is removed.

`src/policy_read.rs` is private typed optional IO. Unix classifies `rustix::Errno::NOENT` before generic error conversion, performs two no-follow descriptor observations around optional absence, and compares full parent/leaf identities around actual confined descriptor reads. Required missing parents are stale errors, not None; `.git/info` is optional only below an observed real `.git` directory. Windows uses the existing cap-std root, per-component no-follow handle opens, and retained no-SHARE_DELETE parent/leaf leases across actual reads/absence observations. Reparse parents refuse; permitted final policy aliases resolve only inside root and actual bytes still use the existing confined reader. Outside policy aliases refuse regardless of source follow mode or separately authorized external bytes. Windows implementation/test compilation is not runtime/privacy acceptance.

Fixed fail-closed budgets: 1 MiB per policy/marker, 8 MiB aggregate policy/marker/override bytes, 32,768 lines/override patterns, 4,096 bytes per line/pattern, 128 active frames including supplied ancestors, 16,384 selected directories, 2,048 total matcher compilations, 1,024 external policy/repository inputs, 32,768-byte supplied paths, 16 MiB evidence accounting, and 64 MiB / 1,000,000 supplied tracked-name limits. Existing source max-files/byte limits remain separate. NUL/invalid UTF-8/oversize/malformed or over-budget rules abort; BOM/CRLF and last-line semantics use the locked parser, and parse errors report category/path/line without raw pattern text. Cancellation/deadline still propagates; native/kernel calls and matcher compilation remain indivisible, not forcibly interruptible.

## Executed Scoped Validation And Source Freeze

Final native command `cargo test -p kortex-repo --all-features --locked --offline` passes **92 tests / zero failures / zero ignored**: 80 unit + 3 input-boundary + 1 language-discovery + 8 actual Darwin ACL tests. All **76 pre-existing tests/assertions remain unchanged and pass**; 16 new native policy tests cover the seven required groups. The additional Windows hidden-attribute/whitelist fixture is compile-checked only, not counted as native execution. Log `target/ignore-policy-foundation-tests.log`.

- All three outside dummy-policy aliases versus regular controls, both source-follow modes; separately supplied external bytes do not grant repository aliases. A direct reader assertion records ZERO actual bytes and ZERO confined byte-read file opens for the refused outside alias. Internal final-policy aliases and source file/directory aliases, loops and canonical-source dedup are exercised.
- Locked `ignore 0.4.34` inventory oracle comparisons cover category versus directory precedence, nearest/last line, BOM/CRLF, negations, Git-required gating, ordinary exclude and nested repository boundaries. Explicit external global/exclude/category/override tests exercise all precedence tiers. Parent-ignored child policies cannot load or resurrect themselves; final callback failure at iterator exhaustion aborts.
- Real absent policies, missing optional `.git/info`, required stale parents, permissions, dangling/cyclic aliases, FIFO, encoding/NUL/oversize/line/aggregate/compilation bounds and cancellation are exercised. Deterministic hooks mutate ACTUAL missing-leaf/parent/root names between observations and actual leaf/parent/root objects after real reads; failures return no inventory. Original source scratch, max+1/EOF/root/parent/leaf/deep-reader regressions remain intact.
- Caller-authorized ancestor/global bytes with nonexistent origins demonstrate bytes-only matching and provenance changes without origin opens. Gitfile supplied excludes/digest matching succeeds; changed/ungranted gitfile, ungranted commondir, missing/incomplete standard context and root-only Git index fail explicitly. Full trusted owner-context acquisition, real worktree/global default integration and publication/restart acceptance remain parent-owned.
- Wide 100-directory policy fixtures keep peak active frames at TWO; a depth-128 tree fails the explicit active-frame bound. Aggregate-byte and total-compilation limits are exercised without a benchmark or per-directory matcher cache. This is bounded functional evidence, not performance/reference acceptance.

Strict native command `cargo clippy -p kortex-repo --all-targets --all-features --locked --offline -- -D warnings` PASS (`target/ignore-policy-foundation-clippy.log`). Existing installed official pinned 1.99.1 Windows-target ALL-target/ALL-feature strict clippy PASS, including new Windows handle backend and hidden-attribute test compilation (`target/ignore-policy-foundation-windows-clippy.log`):

```sh
toolchain="$(dirname "$(rustup which --toolchain 1.99.1 rustc)")"
PATH="$toolchain:$PATH" RUSTC="$toolchain/rustc" CARGO_TARGET_DIR=target/crew-storage-windows \
  cargo clippy -p kortex-repo --all-targets --all-features \
  --target x86_64-pc-windows-gnu --locked --offline -- -D warnings
```

A small independent actual-parser SDK proof uses `tests/fixtures/authorized_parser_sdk.rs`, compiled explicitly against current workspace rlibs, without adding a manifest/dependency. Current core/config/repo/parse build PASS (`target/ignore-policy-foundation-sdk-build.log`); proof PASS (`target/ignore-policy-foundation-parser-sdk.log`): **6 regular-policy exclusions, 12 outside-alias refusals (both dummy contents in both source-follow modes), 6 real Tree-sitter Python parses of the safe API's one-file negative controls**. Actual source/hash/read-byte assertions pass. The inert Python fixture is NEVER executed; only the authorized Rust SDK proof program runs.

```sh
cargo build -p kortex-core -p kortex-config -p kortex-repo -p kortex-parse --all-features --locked --offline
rustc --edition=2024 -D warnings crates/kortex-repo/tests/fixtures/authorized_parser_sdk.rs \
  -L dependency=target/debug/deps \
  --extern kortex_repo=target/debug/libkortex_repo.rlib \
  --extern kortex_core=target/debug/libkortex_core.rlib \
  --extern kortex_config=target/debug/libkortex_config.rlib \
  --extern kortex_parse=target/debug/libkortex_parse.rlib \
  -o target/ignore-policy-foundation-parser-sdk
target/ignore-policy-foundation-parser-sdk
```

Owned `cargo fmt -p kortex-repo -- --check`, explicit standalone-proof `rustfmt --edition 2024 --check`, and scoped `git diff --check` PASS. Initial compile/lint fixes and concurrent parent missing-config-module/locked-manifest interruptions occurred before these final passing runs; no peer module, manifest or lockfile was edited to bypass them. GitNexus pre-edit impacts returned UNINDEXED; direct source/caller/locked-oracle inspection substitutes, not a graph-completeness claim. Existing dirty examples/confined/reader tests and every old root check are preserved. No delegation, first-party unsafe, dependency/root/lock/config/other-crate edit, indexed-code execution, upload/download, listener, commit/push/reset/clean/publication/deploy or large benchmark.

Source is frozen for parent review/adoption. SHA256 values (not core BLAKE3):

| Source / proof | SHA256 |
| --- | --- |
| `src/lib.rs` | `878c3759211701353d5af08c64a3fe86f0cbb8247384e3ec71a418b7258b4b25` |
| `src/reader.rs` | `7494820e8a4aab6b0e4ed5d14e6f8e9d8f9347745eec8869c07f77cf99b8f55d` |
| `src/policy.rs` | `c532e6f2c60b31636aadd2c50bfcab584a42c65df8f360d1e57e8ae00f9e0fd4` |
| `src/policy_read.rs` | `2cac5424e0c0bed03337eaea8e22ad3f36da866ab943acc5cbd4bba68ec77b4e` |
| `src/policy_tests.rs` | `412ce738aba744fe9bca99bbbf3c1403705b651229ef2c0879ee86e110c606cb` |
| `tests/fixtures/authorized_parser_sdk.rs` | `9a7d8eaf737d4a55c3e9e8d0115f70c89265f48ad54a7b067dd16bcd4b0ad77f` |
| `target/ignore-policy-foundation-parser-sdk` | `17ddf39350ff7f21386c309fc75007e63d449440dbbb2c300bd79ece96f08cd0` |

Parent owns independent review, standard owner-context acquisition/completeness, config/indexer/freshness/caller adoption, actual CLI default-gap closure, failure publication/restart/retry, full workspace/Linux/Windows runtime/private release acceptance. Initial root/ancestor acquisition remains trusted; ambient walker enumeration is a separate race boundary. No atomic filesystem snapshot, arbitrary namespace ABA defense or isolation from malicious same-UID/administrator writers is claimed.

## Required Regression Evidence (Production Adoption Gates)

- All three outside symlink policies versus regular controls, both follow modes: refuse without outside content opening; previous generation remains intact.
- Authorized internal policy aliases; ordinary file/directory aliases, loops and duplicate elimination retain behavior.
- Category, nesting, line precedence, BOM/CRLF and negations agree with the locked semantic oracle.
- Ignored directories cannot load policies or resurrect themselves; includes/hidden/default exclusions retain their behavior.
- Typed missing versus permission, dangling/FIFO/device, oversize/encoding/NUL and cancellation failures; deterministic root/parent/leaf replacement tests remain unchanged.
- Nested-root ancestors, ordinary excludes, linked worktrees and owner-global policies have authorized parity or explicit failure, never an undocumented authority expansion.
- Deep/wide policy state stays bounded; actual CLI/SDK, failed publication/restart/retry and current full workspace acceptance are rerun.

This work removes ambient policy-content authority; it does not create an atomic filesystem snapshot, defeat arbitrary namespace ABA or isolate malicious same-UID/administrator writers. Full production acceptance stays OPEN.
