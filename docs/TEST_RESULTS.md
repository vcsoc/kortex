# Executed test results

## Environment and boundaries

Local Apple M5 Pro / aarch64 macOS 26.5.2, Rust/Cargo 1.99.1. No Windows/Linux test run, hosted CI execution, public push, tag, release, account operation or remote service launch has been performed. The specification and pre-existing `.pi` content are preserved. Git was initialized on main; origin is https://github.com/vcsoc/kortex.git. No commit was requested or made.

## Current production wave: not accepted complete

- Parent config/repository/storage verification: **67 scoped tests** and strict native clippy pass. This includes actual P1 extension discovery and the two parent-added writable-artifact regressions. Current full-workspace acceptance and refreshed coverage remain open.
- Genuine Windows-target storage compilation and strict cross-clippy pass using official Rust 1.99.1 and a checksum-verified local Zig/Clang compiler. Real SQLite output is AMD64 COFF; an intentionally invalid C probe fails normally. Two Windows-only directory-builder lint errors were fixed. This is NOT Windows execution or ACL/privacy acceptance; see `WINDOWS_VALIDATION.md`.
- Five private-policy regressions, six compiler-adapter unit tests and seven benchmark-orchestration unit tests pass. These unit tests are not substitutes for real application or reference-performance acceptance. The hardened tiny real-binary benchmark self-test also passes.
- The initial all-feature audit found four new ZIP vulnerabilities and unmaintained `paste` through the embedding backend. Candle was upgraded to 0.11.1 / ZIP 9.5.1; parent reran the locked all-feature audit. ZIP findings are gone, but **audit still FAILS** on RUSTSEC-2024-0436 (`paste` 1.0.15, compile-time proc macro). No suppression or owner health-risk exception has been added. Latest log: `target/production-wave-audit-latest.log`.
- A reference-sized OLD pre-wave release diagnostic exceeded its 45-second full-index timeout at 10,000 files/100,000 functions. It recorded `runs_complete: false`; no completed reference query/RSS result exists. The smaller 1,000-file diagnostic does not prove scale. See `PRODUCTION_READINESS.md` and the raw `target/scale-baseline-prewave-100k-nonquiescent.json` failure report.
- Semantic worker runtime reports 49 scoped tests and six real P1 providers; transport checkpoint reports 27 scoped passing tests, one preserved regression and browser files-API failures. Continued implementation and independent integration are separate from those checkpoints. Parent parser cache version is now `tree-sitter-0.25-kortex-7`.

## Last accepted development baseline

The following command table, milestones and integrated crew section record the PREVIOUS 97-test development snapshot. They do not describe current-wave source acceptance or current coverage/audit results.

| Command / scenario | Observed outcome |
|---|---|
| `cargo fmt --all -- --check` (and format during implementation) | passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | passed after fixing reported issues |
| `cargo test --workspace --locked` | 97 tests passed across unit/integration suites, including one child-process crash helper; no failing tests in the latest run |
| `cargo build --release --locked -p kortex-cli` | passed; native optimized binary produced |
| `cargo install --path crates/kortex-cli --root target/install --locked --offline` | passed; installed binary executed `version --json` successfully |
| `kortex --help`, `kortex version --json` | executed successfully |
| Actual executable core journey | index -> search -> context -> impact -> trace -> graph -> edit -> stale -> Git changes -> incremental -> doctor/restart passed |
| Actual rmcp SDK client/stdin-stdout child transport | initialized, listed eight tools and three resources, called every tool, read every resource, checked cited context/error and clean shutdown: passed |
| Native watcher E2E | atomic save + rename/delete, committed update, process kill and restart lookup: passed |
| SQLite killed-writer subprocess | process killed after transaction insert; old generation intact, incomplete row rolled back, integrity=ok, writer lock re-acquired: passed |
| Graph/FTS concurrent generations | old read transaction retains old graph and FTS while new writer generation publishes: passed |
| Full/incremental comparison | normalized files/edges equal after content update and rename/delete: passed |
| `python3 benches/smoke.py` | completed; raw measurements in benchmark-local.json and scope explained in PERFORMANCE.md |
| `python3 packaging/sbom.py` | generated CycloneDX inventory; declared licenses are not legal approval |
| `cargo-deny 0.20.2 check` | advisories, bans, licenses and sources passed after upgrades; duplicate/unused-license warnings remain, no advisory ignores |
| `cargo llvm-cov --workspace --locked --summary-only` | 90.92% line coverage locally with official Rust 1.99.1 LLVM tools; instrumentation tests passed |
| critical repo/storage coverage report with 90% threshold | 92.30% measured; 90% threshold passed with exit 0 after crew integration |
| `python3 packaging/artifact.py --development` | private native archive/checksum generated; repeated packaging produced identical bytes for identical inputs; public packaging without approval refused with exit 2 |
| extracted archive binary | copied eight-file polyglot fixture indexed; version/search/context/impact/trace/doctor all passed from the extracted executable; target/package-validation.json records local evidence |

## Milestone evidence

M0 foundation: all prescribed workspace crates, pinned toolchain, lockfile, CLI help/version, standards, CI/release-preparation configuration exist. Local installation/build pass. The owner subsequently selected proprietary/private policy: no public or third-party use grant, and public release prohibited.

M1: strict TOML/default layering, nested environment precedence/security refusal, ignores, byte/file caps, canonical confinement, traversal/symlink rejection, transactional SQLite generations, reader/writer consistency, registry/restart and corruption diagnostics are exercised. Complete config/root/workspace/migration/discovery acceptance remains pending.

M2: real Rust/Python/TS/JS/Go grammars are exercised on declarations, actual source spans, imports/aliases, call sites, syntax errors and lexical binding fixtures. No source/package/build execution occurs during indexing. Not a broad labeled precision/recall acceptance report.

M3: cross-file direct call resolution for P0 families passes. Duplicate candidates remain ambiguous with no target; unrelated same names, non-exported ESM functions, import scope, Python class scope and parameter/local shadowing are regression tested. Search ranks exact check above incidental matches in the CLI journey. Graph persistence/restart and FTS generation isolation pass. General references/type/inheritance/receiver inference remain incomplete.

M4: directed shortest paths and reverse dependencies match fixture expectations. BFS reachability and evidence chains match independent transitive closure across all 4,096 loop-free four-node directed graphs (16,384 source traversals). Git unstaged hunk mapping, stale source detection, incremental reuse, full-equivalence, native watch and crash recovery pass. Large-scale invalidation/Git edge cases remain unverified.

M5: actual SDK client protocol behavior is tested, not a fabricated protocol-success handler. Tests reject unknown/unbounded arguments and oversized lines; encoded response budgeting preserves graph integrity and query cursors without skipping omitted hits. Broad cancellation/hostile-input/client-application coverage and stable per-operation output schemas remain gates.

M6: local release build/install, security regressions, dependency audit and small benchmarks pass. Local overall coverage is 90.92%, critical repo/storage coverage 92.30%; both configured line-coverage thresholds pass. CI matrices, thresholds and private packaging are configured, not claimed executed on remote hosts. At that historical baseline, P1/P2 were not implemented.

## Failures found and fixed

- Multiline TOML configuration initially failed with the dependency's Value FromStr path; switched to the document deserializer and retained a partial-config regression.
- Build/lint failures (numeric type inference, deprecated SDK alias, collapsible conditions and range checks) were corrected, then tests/lints rerun.
- Negative resolution cases exposed missing import scopes/export checks/Go short-variable shadowing/Python class-namespace handling; added independent fixtures and conservative fixes.
- Oversized MCP read regression exposed an AsyncRead contract violation: bytes were appended before returning an error. The wrapper now restores the original filled length on refusal; the test passes.
- Reader/FTS generation race risk was addressed by read-only pinned SQLite transactions and verified against a replacing writer. Schema initialization now also holds the writer lock; database/WAL/SHM/lock symlinks are refused.
- Initial dependency audit found RUSTSEC-2026-0459 in bundled SQLite and RUSTSEC-2026-0457/0461/0462 against the older MCP SDK. Upgraded rusqlite to 0.42.0 (libsqlite3-sys 0.40.0) and rmcp to 3.9.0; reran protocol/storage regressions and audit successfully. No advisory suppressions added.
- An explicit native-event test timed out on this host with the default FSEvents backend, despite reconciliation making the earlier watcher journey pass. Selected notify's macOS kqueue backend and retained a direct native-event assertion; native delivery and end-to-end watch now pass. Large-scale file-descriptor usage remains a performance gate.

## Integrated crew verification

All four worker tracks completed, followed by parent workspace acceptance: 97 tests passed, workspace fmt/clippy passed, and cargo-deny advisories/bans/licenses/sources passed. AST REFERENCES/USES_TYPE and richer declarations, content-verified Git side mapping, storage/root-boundary regressions and real SDK contract tests are included. Parser cache version is tree-sitter-0.25-kortex-6.

Parent review additionally reproduced recursive submodule clean-filter execution using a harmless test marker, then disabled nested dirty checks and forced short submodule output. All 17 analysis tests pass; comparisons with submodule worktrees report partial evidence and require querying each submodule separately. Parent also replaced recursive native-watch registration with individually confined, non-symlink targets. A native regression confirms outside file/directory symlink modifications do not wake the watcher while in-root writes still do. No indexed source is intentionally executed by production analysis.

Coverage summaries: coverage-local.txt and coverage-critical-local.txt. Previous 84.62%/83.47% figures and intermediate worker failures are superseded by the integrated 90.92%/92.30% measurements; scoped handoffs preserve their historical observations.

## Owner privacy-policy verification

The owner explicitly requires closed-source/private first-party code with no permission for anyone else to use it, and approved React + TypeScript + Cytoscape.js. LICENSE now records that restrictive notice. Public packaging refuses unconditionally even with LICENSE present and legacy KORTEX_RELEASE_APPROVED=true. Five private-policy regressions pass: public refusal without creating artifacts, publish=false/restrictive inherited license metadata, private explorer metadata, private-repository PLUS explicit-owner-approval guards for every hosted job, and immutable external action pins. Checkout credentials are not persisted. See CI_PROVENANCE.md for the upstream-resolved action hashes and the limits of that evidence. No hosted jobs or publication were initiated. The earlier 97-test snapshot is historical baseline evidence; active crew source is not yet accepted as a new integrated production snapshot.

## Remaining unverified or incomplete

No full P0/P1 production acceptance claim. Current remaining criteria and exact evidence are tracked in PRODUCTION_READINESS.md; IMPLEMENTATION_PLAN.md also preserves earlier implementation milestones. Windows/Linux/aarch64/x86 release matrix, owner legal approval and complete provenance review, signing/attestation, complete output schemas, disk-full/permissions/TOCTOU/fuzz testing, network-disabled sandbox proof, reference-scale performance/RSS and labeled quality evaluation remain outstanding. Local build/test success is not evidence those gates passed.

GitNexus impact checks were attempted before symbol edits, but this newly initialized project has no GitNexus index; no dependency blast-radius report was available. Direct source inspection and executable regression tests were used instead. No restricted upstream implementation was read or copied.
