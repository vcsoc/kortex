# Executed test results

## Environment and boundaries

Local Apple M5 Pro / aarch64 macOS 26.5.2, Rust/Cargo 1.99.1. Windows execution remains unavailable. Local Docker Desktop Linux arm64 compilation was attempted with networking disabled; bounded runs timed out before application tests, so no Linux runtime pass is claimed. No hosted CI execution, public push, tag, release, account mutation or remote service launch has been performed. The specification and pre-existing `.pi` content are preserved. Git was initialized on main; origin is https://github.com/vcsoc/kortex.git. No commit was requested or made.

## Latest Integrated Lazy-Query Development Checkpoint

All four workers completed. Parent resolved original-file-order API integration and corrected a legacy test fixture to remove ALL optional new evidence; a new negative regression retains CORRUPT_INDEX for orphan current projection rows. CLI/MCP/HTTP now invoke shared ReadService dispatch directly: query is genuinely projected; context/impact remain explicit same-Store eager fallback.

Actual rebuilt CLI + `cargo test --workspace --all-features --locked --offline --no-fail-fast -- --include-ignored` with explicit KORTEX_TEST_CLI: **ALL targets PASS, zero ignored**, including analysis59/search23/storage62/repository69, admin regressions and BOTH normally ignored CLI proofs. Strict all-target/all-feature workspace clippy, workspace fmt, all-feature advisory/bans/licenses/sources audit ALL PASS. Logs `target/parent-lazy-integrated-{tests,clippy,format,audit}.log`.

Real100k-function10-sample optimized diagnostic verifies complete eager/lazy response equivalence and no eager query materialization. Lazy open1.63–4.22ms; full lazy execute including freshness697–1830ms; actual CLI wall758–1893ms, CLI RSS61.0–65.4MB. Current latency targets remain NOT met. No universal speedup/reference-percentile/whole-process-memory-improvement claim; full profiler RSS includes eager/fixture generation. Evidence `target/parent-lazy-100k-profile.log`, binary hashes `target/parent-lazy-profile-binaries.sha256`, details `docs/crew-intelligence.md`. Earlier Linux pass below is an older isolated baseline, NOT this newly integrated source. Schema2 default adoption remains prohibited; current coverage/platform/browser/release acceptance still require refresh.

## Corrected Linux Functional Checkpoint

Parent fixed the genuine experimental stamped-read coarse-timestamp mutation blind spot identified by Linux execution: a second bounded read of the actual descriptor compares bytes, with original assertions retained and a new64-write regression. Native repository **69 PASS** and strict clippy PASS. A new immutable snapshot overlays ONLY the completed repository crate onto the prior all-feature development baseline (other active worker changes excluded):162 inputs, SHA256 `e973dd15053442ab11f7945bf5215f261432d4d1a3f7bc24798b3bdf207c4a26`.

Actual offline/nonroot ARM64 Linux VM workspace: **313 passed/0 failed/1 ignored**; explicit ignored six-language CLI journey PASS; extracted private DEBUG CLI archive/install `version/index/query/context/impact/doctor` smoke PASS with actual semantic assertions and executable hashes. Full report `target/local-linux-validation/native-5m9echoq/report.json`, details and preserved initial failure `docs/LINUX_VALIDATION.md`. This is NOT live-head, x86/Windows, release packaging, signing, hardware performance, power-loss or production acceptance.

## Fresh integrated parent verification

After all four workers completed, parent independently ran the stable source:

- `cargo test --workspace --all-features --locked --offline --no-fail-fast`: PASS across all workspace test targets; one CLI-dependent test initially ignored.
- `KORTEX_TEST_CLI="$PWD/target/debug/kortex" cargo test -p kortex-indexer --test p1_discovery --locked --offline actual_cli_discovery_query_and_context_for_all_six_p1_languages -- --ignored --nocapture`: PASS, actual required test executed (one passed, not zero filtered success).
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`: PASS.
- `cargo fmt --all -- --check`: PASS.
- `target/tools/bin/cargo-deny --all-features --locked --offline check`: advisories/bans/licenses/sources ALL PASS, no waivers.
- Independent `selection_admin` HTTP-feature target: all FOUR regressions PASS unchanged; earlier config-show/corrupt-repair failures are superseded.

Logs: `target/parent-wave-integrated-{tests,clippy,format,audit}.log`, `target/parent-wave-six-language-cli.log`, `target/parent-selection-admin-tests.log`. These are current integrated development checks, NOT full production acceptance, refreshed coverage, reference performance, complete transport lifecycle, pretrained quality or platform/install proof. All four worker sessions are completed; no active worker is implied by their prior launch receipts. Persistent private vector cache and native Darwin ACL implementation are now part of this passing source; general lazy query/context/impact dispatch remains unfinished.

## Prior production-wave checkpoints (superseded where noted)

Canonical current ledger: `docs/PRODUCTION_READINESS.md`; renewed implementation contracts: `docs/NEXT_ENGINEERING_CONTRACT.md`. Counts below are separate checkpoints, NOT a summed integrated-workspace result.

- Parent earlier verified67 config/repository/storage tests and strict native clippy. Later storage checkpoint42 tests, repository30 tests and intelligence130 scoped tests (including retained58 semantic/provider/indexer tests) pass. Fresh full-workspace acceptance/critical coverage remain open. Parser cache stays `tree-sitter-0.25-kortex-7`.
- Normal registered-ID relocation/provenance is fixed. Latest transport Rust checkpoint39 passes/two preserved `selection_admin.rs` failures: ID config-show and corrupt doctor-repair still lose selected explicit origin. HTTP10/MCP4 scoped tests and strict clippy/fmt pass. TEN actual Rust-served browser journeys, frontend build/format and npm audit pass. Files, impact likely-tests, binary assets and shared graph-schema parity are resolved, not current blockers.
- Parent ALL-feature locked/offline audit NOW PASSES: advisories, bans, licenses and sources. Maintained tract0.23.10/pastey replaces Candle/tokenizers/paste. No suppression or owner waiver was added. Historical ZIP/paste failure logs retained; current log `target/production-wave-audit-current.log` supersedes them.
- Intelligence scoped instrumented coverage87.20% across analysis/config/graph/search with72 passing tests, NOT workspace/critical acceptance. Real generated-tensor encoder and independent token-ID conformance pass; pretrained artifacts/relevance and persistent cache acceptance remain missing. Default automatic local model remains gated.
- Fixed real10k-file/110k-symbol storage diagnostic replacement30.39 ->10.63s, nonquiescent n=1; changed-fact storage-only publication6.88s is a different inert fixture. Query profile still shows eager Snapshot300-600ms and freshness400-1250ms. No reference p95/sample/RSS acceptance; old45s timeout remains historical evidence, not the latest completed diagnostic.
- Parent private-development archive smoke verifies707 payload hashes, four prebuilt explorer assets,439 Cargo/npm declared inventory records and three exact-commit/checksum grammar notices; extracted executable runs version without Node. This is NOT production/signing/reproducibility/install acceptance. Evidence `target/continuation-package-validation.json`; packaging/harness unit results are not application acceptance.
- Genuine Windows-target SQLite compile/cross-clippy uses official Rust and verified Zig/Clang, actual AMD64 COFF, intentional invalid-C refusal. Windows runtime/DACL/privacy still unverified. Unix mode placement alone does not establish macOS extended ACL privacy; current helpers need call-site integration and ACL review.
- Local Linux arm64 pinned-image/nonroot/offline/network-disabled frozen-source builds twice reached CLI compilation then timed out at300s. `target/local-linux-validation*.json` explicitly records false runtime acceptance; no Linux application test/install pass or source upload. Parent owns runner/cache follow-up.
- Core RequestControl is a shared cooperative primitive only; renewed application/repository/transport work must observe it in real loops. Native/kernel-call force-stop and complete cancellation acceptance are not implied. Four renewed worker IDs/scopes are recorded in `docs/CREW_CURRENT_WAVE.md`.

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
