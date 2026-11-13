# Independent Frozen F6F5312 Acceptance Evidence

## Decision And Scope

The requested **scoped functional gates pass for the frozen committed baseline**, not the changing live checkout. **Full private-production acceptance remains OPEN; parent independent acceptance is separate.** This record does not waive security, platform, performance, model-quality, signing, privacy or specification gaps.

- Commit: `f6f5312afb0d0c5490ca86c9da9911c3773a6b17`.
- Frozen source: `target/production-f6f5312-8bekgnkr/source`.
- Parent pin: `target/production-f6f5312-source.json`.
- Cargo/fixture scope: **168 files**, SHA256 `175587752b24fc04f0a0f0b593f1439ecdfad5526826d15bca606c7dd7c21085`.
- Complete committed tree, including frontend source/lock: **241 files**, SHA256 `b24dd8a30c8f86e5d84e95e2f853e10ddee8bd59609baa844832ed8032de3612`.
- Digest encoding: sorted relative UTF-8 path, NUL, raw bytes, NUL. Every complete-tree file was also compared byte-for-byte with `git show COMMIT:path`; missing/extra files are refused. Before/after/final manifests agree.
- Machine reconciliation: `target/acceptance-f6f5312/acceptance.json`; final log: `target/acceptance-f6f5312/final-gate-reconciliation.log`.

Only the Linux runner/tests, packaging harness/tests, this document and `docs/LINUX_VALIDATION.md` were edited. The original frozen browser tests were reused byte-identically; no browser-test or frontend production-source edits were needed. No crate, root manifest/config/lock, CI or core implementation changes were made by this validation owner. Concurrent peer edits are outside this acceptance identity. No nested agents were launched.

## Measured Gates

| Gate | Current frozen-source evidence |
| --- | --- |
| Native ARM64 Linux all-feature workspace | **PASS: 331 passed, 0 failed, 2 normally ignored**, 54 summarized Cargo targets; locked/offline, exit 0 |
| Linux explicit analysis CLI proof | **PASS: exactly 1 executed**, named lazy-query/eager-context regression and actual CLI BLAKE3 identity present |
| Linux explicit six-language CLI proof | **PASS: exactly 1 executed**, all six language-specific discovery/persistence/query/exact-caller records present |
| Linux private debug CLI install | **PASS**, extracted executable bytes/hashes and real version/index/query/context/impact/doctor semantics checked |
| Native macOS instrumented workspace | **PASS: 341 passed, 0 failed, 2 normally ignored**, 54 summarized targets; both ignored proofs then explicitly executed and passed |
| Full-workspace LLVM line gate | **PASS: 15,086 / 16,627 = 90.731942%**, unchanged 80% threshold, exit 0 |
| Critical repository/storage LLVM line gate | **PASS: 3,546 / 3,814 = 92.973256%**, unchanged 90% threshold and configured crate-exclusion expression, exit 0 |
| Offline frontend install/build/format | **PASS**, cached `npm ci --offline --ignore-scripts --no-audit --no-fund`, TypeScript/Vite build and format check |
| Offline npm audit command | Exit 0, reports **0 vulnerabilities**; **NOT fresh advisory-service/security acceptance** |
| Actual Rust-served Chromium journeys | **PASS: 10 expected, 0 failed, 0 skipped, 0 flaky**, 14.677 s Playwright duration; 15.211 s invocation |
| Current private development archive/install | **PASS: 708 payload files, 4 web assets, 665 notice files, 439 inventory components**, every payload hash verified after extraction |
| Harness regressions | **PASS: Linux runner 16; packaging 36**. All original 15 runner and 29 packaging tests remain present and passing |

Cargo totals take each top-level target's final result, not nested crash/helper summaries. The normally ignored tests are not silently counted as ordinary-suite execution. Their separate exact invocations bring executed named tests to 333 on Linux and 343 in the macOS instrumentation run; counts are not combined across platforms.

## Linux Runtime And Confinement

Report: `target/local-linux-validation/native-a0iu1mi4/report.json`.

Application log: `target/local-linux-validation/native-a0iu1mi4/kortex-tests-08d724821c4043dfa50254af33b7908c.log`.

- Runner exit 0 in **774.032 s**, within the original 1,800-second bound; no retry of the Linux application run.
- Already-local official ARM64 Linux image `rust@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`; local endpoint only `unix:///Users/chris/.docker/run/docker.sock`. No pull/push or remote daemon.
- Nonroot UID501:GID20; network `none`; all capabilities dropped; `no-new-privileges`; 4 CPUs, 8 GiB memory and memory+swap, 1,024 PIDs; inner GNU timeout and outer watchdog retained. No host ports, privileged mode, devices or added application capabilities.
- Source, Cargo registry subtree and seed target are read-only binds. No host home, credentials, Docker socket, browser cache or model artifacts are mounted into application containers. `/build` and `/work` use only the newly labeled private native volume.
- Private volume: `kortex-local-linux-607e16d62ea94dd3a162179312b9f086`, owner label `607e16d62ea94dd3a162179312b9f086`. It remains retained; no other volume/container was changed or removed.
- Read-only seed: `target/local-linux-validation/linux-target-cache`. This is compilation-cache provenance, NOT runtime evidence. The current workspace/CLI/proofs actually rebuilt/executed; Cargo fingerprint trace is retained.
- Only the empty new volume bootstrap ran as root with CHOWN; no source/cache host ownership was changed. Application/build/test/install phases ran nonroot.
- Saved inspect/state were independently reconciled against the constraints. Exit 0, no OOM, and removal of the owned container were verified; see `target/acceptance-f6f5312/linux-owned-container-cleanup.json`. Docker Desktop's explicit `/host_mnt` normalization of host binds is recorded in inspect.
- Current native ELF SHA256: `8c21947e44cdcb6cb5864be2ee3ef8f292ceaf50dea02af43c64eeedef94214e`.

The new runner requires BOTH `analysis_lazy_cli_proof_passed` and `six_language_cli_journey_passed`. It builds `cargo build --workspace --all-features --locked --offline` and passes that exact binary through `KORTEX_TEST_CLI`. The analysis test uses the workspace all-feature library graph and an exact fully qualified name:

```text
read_query_tests::actual_cli_query_uses_lazy_dispatch_but_context_keeps_honest_eager_fallback
actual_cli_discovery_query_and_context_for_all_six_p1_languages
```

The first verifies real CLI lazy query behavior with corrupted unused eager blobs and preserves the honest eager context `CORRUPT_INDEX` failure instead of fabricating an empty result. The second verifies Java, C#, C, C++, Ruby and PHP. Named-test success, one-test execution, exit status and required evidence are guarded; zero-filtered runs cannot satisfy either gate.

Linux installation evidence is a private **debug CLI-only** archive/extraction smoke in the volume, not the macOS CLI+web archive, release signing, x86 Linux or Windows installation acceptance. JSON checks require fresh/nontruncated results, a real `check` symbol, exact context caller, one direct upstream `entry` impact and doctor integrity `ok`.

## Fresh Coverage And Uncovered Behavior

Installed cargo-llvm-cov **0.9.1** and the official installed Rust **1.99.1** LLVM tools instrumented the frozen source. The isolated target is `target/acceptance-f6f5312/coverage-target`; no historical runtime profiles were used. A partial compilation from the aborted path-setup attempt was retained, not represented as a test run. Current workspace tests, current instrumented CLI build and both external-CLI ignored proofs all executed before reporting.

- Original commands/results: `coverage-{workspace,cli-build,analysis-proof,six-language}.log` under `target/acceptance-f6f5312`.
- LCOV: `target/acceptance-f6f5312/coverage-all.lcov` and `target/acceptance-f6f5312/coverage-critical.lcov`.
- Full JSON: `target/acceptance-f6f5312/coverage.json`.
- Missing-line listing: `target/acceptance-f6f5312/coverage-corrected-summary.log`.
- Final unchanged-threshold report invocations: `target/acceptance-f6f5312/coverage-report-followup.json`.

**1,541 overall lines, 268 critical lines, 200 functions and 2,976 regions remain uncovered.** Branch and MC/DC denominators are zero because those modes were **not instrumented**, NOT because all branches passed. Regions/lines are not branch-coverage proof. macOS instrumentation does not measure Linux/Windows conditional paths.

Notable remaining weak areas: CLI output **52.83%**, CLI HTTP entry **65.52%**, HTTP listener **67.07%**, repository watch **78.70%**, storage location **80.47%**, maintenance **82.38%**, recovery **84.62%**, and repository experimental source-stamp logic **86.48%** lines. The missing-line file retains exact ranges, including fallback/error handling and repair/recovery paths. No exclusion/threshold change was made to hide these deficits. ENOSPC/fsync/hardware power-loss, broader hostile filesystem/concurrency and complete cancellation/error branching remain open independently of line gates.

## Actual Browser And Private Package

Current macOS development executable was built from the frozen source into the separate `target/acceptance-f6f5312/native-target`, not assumed identical to a live target. Build exit 0 in **137.469 s**; `file` verifies ARM64 Mach-O.

- Native executable SHA256: `0dc31d016fde63c10ea839fa23ba3e4a7e6b528e49a266e1812de5e98724b9b1`.
- Archive: `target/acceptance-f6f5312/private-artifacts/kortex-0.1.0-aarch64-apple-darwin-development.tar.gz`.
- Archive SHA256: `7f880dbd9692c4746868769134b3482f70ef90d77edefeec16beef6c5c514af1`.
- Detailed package/browser reconciliation: `target/acceptance-f6f5312/package-browser-followup.json`.
- Original Playwright JSON and stderr: `target/acceptance-f6f5312/browser-results.json`, `target/acceptance-f6f5312/browser-stderr.log`.
- Private install command JSON: `target/acceptance-f6f5312/private-install-smoke-git-root/commands`.

The frontend source/manifest/lock/tests were copied byte-identically to an external build directory; cached npm installation disabled scripts and all implicit browser downloads. The frozen source stayed dependency/output-free after correction of the early coverage cache-path error. Runtime uses the **extracted executable and extracted web assets**, actual explicit loopback/bearer-token Rust server, original ten Chromium tests, no API mocks, CDN or Node/Vite runtime server. Tests cover real selection/provenance, evidence/graph/exports/routes/impact/changes, file tree/deep paths, hostile labels, bounded behavior, and light/dark/mobile accessibility.

Only existing cached Google Chrome for Testing **156.0.8078.4** was used; executable SHA256 `c8509aee32abd6977968e9999769980b7a420dea05402d90042d9e5b41db15ef`. Installation smoke and browser/test/server process trees ran under the saved `browser-sandbox.sb`: direct external IP networking denied, explicit loopback inbound/outbound allowed, local Unix IPC allowed. This is not protection against a malicious same-user local proxy or compromised browser/kernel. No browser/model artifact was downloaded.

The archive includes proprietary LICENSE, docs, CycloneDX inventory, supplemental grammar notices bound to published VCS revisions and notice hashes, installed npm notices, web asset manifest and complete payload hash manifest. External frontend notice collection requires byte-identical frozen manifest/lock and rejects unsafe aliases. All 708 payload hashes and the installed executable identity were verified. Real extracted version/index/query/context/impact/doctor semantics pass, with 0700 HOME and 0600 registry, followed by all ten browser journeys.

Two archives made from the **same already-built inputs** have identical bytes. This proves deterministic archive serialization for those inputs, **not independently reproducible executable builds**. The 439-component Cargo/npm inventory is an all-feature/build/test/other-platform superset, not exact linked composition or legal/notice completeness approval. No trusted distribution signing/notarization or public release was performed. `--development` remains mandatory; original and additional tests prove legacy approval variables and explicit frozen-project options cannot bypass `PUBLIC_RELEASE_DISABLED`.

## Retained Failures And Procedural Review

These are not erased or described as successful first attempts:

1. Initial coverage `show-env` selected a generated `source/target` cache despite the initial external setting. The process was stopped; its compile log/environment were retained as `aborted-output-path-*`; only the generated cache was relocated. Every committed byte was reverified. The renewed measured run explicitly pinned Cargo target/build/profile paths outside the source. This procedural deviation requires parent review; do not claim the initial filesystem tree was never touched.
2. Initial report-only calls rejected unsupported `--all-features` for `report`, despite shared help listing that option. Their exit-1 logs remain. Corrected report syntax reused the same already-collected profiles, preserved 80/90 thresholds and did not rerun application tests to seek green.
3. Two initial installation fixture attempts returned real `PATH_REFUSED` exit 8. The fixture was nested inside the live Git worktree without its own Git marker, so discovery selected outer-checkout artifacts. Making HOME private alone did not address that. Both command JSON/errors remain in `private-install-smoke` and `private-install-smoke-corrected`. A new private fixture with explicit template/hook-free Git initialization fixes the test setup; original application assertions/privacy policy were not weakened.
4. Final evidence reconciliation preserved its early administrative assertion logs: Docker Desktop normalized some read-only host binds using `/host_mnt`, and the Docker CLI spelled `no such object` lowercase. Checks now accept only the explicit approved host mapping and exact owned-container absence; no application rerun or confinement relaxation resulted.

All previous historical logs, including the old baseline Linux failure/corrected run, remain intact. The earlier 313-test Linux result is historical and is not substituted for this 331-test current-baseline result.

## Still Open

- Parent review/acceptance of the scoped code, evidence and procedural deviations; no aggregate production-ready decision.
- Fresh advisory-service review: offline npm reports zero but cannot establish freshness. Parent's prior Rust audit is separate evidence, not a newly performed audit here.
- Native Windows runtime/privacy/ACL/install/crash and x86/other architecture coverage; cross-compilation is not runtime proof.
- Approved pretrained artifacts/license/relevance, broad held-out language/search correctness and editor-client evidence.
- Required reference-scale distributions/sample counts/RSS and remaining specification latency/update targets; these validation runtimes are not performance acceptance.
- Uncovered branches/error cases, hostile same-UID/root/daemon trust, arbitrary-writer atomicity, ambient SQLite pathname races, ENOSPC/fsync/power-loss and independent security review.
- Trusted signing/notarization, independent executable reproducibility, exact-binary SBOM/provenance and legal/notice completeness review.

No upload, model/browser download, image pull/push, arbitrary remote daemon, project commit/push/reset/clean, hosted CI approval/run, public release or deployment occurred. Fixture-local Git setup/commits in the existing test suites do not modify the project repository and do not execute indexed source. Owned-file scope hashes and retained evidence hashes are published under `target/acceptance-f6f5312`; they are local integrity records, not signed attestations.
