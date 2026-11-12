# Independent Local Linux Validation

## Latest Parent Follow-Up: Corrected Frozen Baseline PASS

The initial failure below was retained and fixed, not retried until green. Parent corrected `experimental::read_source_impl` to rewind a clone of the actual opened descriptor and compare a second bounded content read before final metadata/path checks. Same-size writes no longer rely on timestamp changes. The original mutation assertion is unchanged; an additional 64-write regression requires SOURCE_CHANGED for every actual same-size mutation without assuming a particular timestamp resolution. This is still two observations, NOT arbitrary-writer atomicity, and stamps never authorize metadata-only freshness.

Parent native repository validation: **69 tests PASS**, strict all-target/all-feature clippy PASS (`target/parent-linux-stamp-fix-tests.log`, `target/parent-linux-stamp-fix-clippy.log`). The new Linux snapshot overlays ONLY the completed repository crate and parent correction onto the prior development baseline. Other workers' active changes were deliberately excluded; this is NOT live-head validation.

- New source: 162 inputs, SHA256 `e973dd15053442ab11f7945bf5215f261432d4d1a3f7bc24798b3bdf207c4a26`; manifest `target/stamp-corrected-linux-source.json`, including changed input list and parent pin.
- All-feature locked/offline Linux workspace: **313 passed, 0 failed, 1 ignored**, all54 summarized targets successful. Both the original mutation test and new repeated-write test actually executed and passed.
- Explicit ignored six-language CLI journey: **PASS**, exactly one test executed, all six per-language evidence lines verified.
- Private debug CLI archive/extraction/install smoke: **PASS** with byte comparison and SHA256, separate private HOME/registry, actual extracted `version/index/query/context/impact/doctor`, verified JSON freshness/exact-call/direct-impact/integrity semantics. This is NOT release packaging, web installation, signing or reproducibility acceptance.
- Executable SHA256: `c266f7dcfba6b45d251f6990c4991b6fe1c4ec232c4b7801b8c76fd1e793546d`; actual aarch64 Linux ELF.
- Runner exit0 in321.36s, source pin verified after execution. Same pinned local image/socket, UID501:GID20, network none, read-only source/registry, bounded4CPU/8GiB, private newly labeled native target volume, no upload.
- Report: `target/local-linux-validation/native-5m9echoq/report.json`; log: `target/local-linux-validation/native-5m9echoq/kortex-tests-1245b868f7684da58a930bf642376a41.log`; parent invocation: `target/parent-stamp-corrected-linux-run.log`.

No x86/Windows, power-loss, hardware performance, current live-head or full production acceptance follows from this result. The earlier failure and unchanged-binary retry evidence below remain valid historical observations.

## Initial Result: Blocked By A Genuine Application Regression

This is **native ARM64 Linux VM functional evidence for an explicitly frozen source**, not acceptance of the live checkout or full production readiness. The owner-authorized independent run on 2026-11-11 reached and executed the real application tests. It did not time out.

| Gate | Actual result |
| --- | --- |
| Frozen source pin, before/after execution and final checkpoint | PASS: 160 inputs, SHA256 `199b64509c4d9683871575f57cac9f2ea39557b6711fdd4652720de1e470fca4` |
| All-feature locked/offline workspace tests | FAIL: 301 passed, 1 failed, 1 ignored across 54 summarized targets; exit 101 |
| Explicit normally ignored six-language CLI journey | PASS: exactly 1 passed, 2 filtered out; all six per-language evidence lines present |
| Trusted native executable build | PASS: all-feature CLI build; actual aarch64 Linux ELF |
| Private installation/extraction smoke | NOT RUN: withheld because workspace tests failed |
| Runner unit tests | PASS: 15 tests; these are orchestration tests, not application acceptance |
| Overall Linux runtime gate | FAIL / blocked; no production or installation acceptance |

Counts take the final summary of each top-level Cargo target, avoiding double-counting a nested crash-test helper. The original run report's `workspace_passed_test_invocations: 257` counted only entirely successful target summaries and included that nested helper. It is preserved unchanged. The corrected full totals above are in `target/linux-independent-checkpoint.json`, produced from the retained original log; this is not a replacement passing test run.

The separate journey executed `actual_cli_discovery_query_and_context_for_all_six_p1_languages`, using `KORTEX_TEST_CLI=/build/debug/kortex`. Java, C#, C, C++, Ruby and PHP each reported three discovered/persisted files, a real `check` query hit and one exact context caller. The runner refuses a zero-filtered-test success, a missing named test, incomplete language evidence, nonzero exit or truncated phase.

## Source And Executable Identity

- Frozen source: `target/local-linux-validation/accepted-wave-phedqvpr/source`.
- Owner pin: `target/accepted-wave-source.json`.
- Input digest: sorted relative UTF-8 path + NUL + raw file bytes + NUL, for every entry returned by `source_inputs`; 160 records. This is SHA256, not the application's BLAKE3 content hash.
- Per-file SHA256/size manifest: `target/linux-accepted-source-manifest.json`.
- Native debug executable SHA256: `e702eddb40ad059dea6487dbca04208a9998843683c1b1f88f8c6ed7b70d5887`.
- ELF: 64-bit little-endian aarch64 PIE, dynamically linked with `/lib/ld-linux-aarch64.so.1`, with debug information, not stripped. The log records `file` and `ldd` output.

The supplied snapshot was reused read-only, not recreated from changing peer files. Its content pin was verified again after execution. No assertion or crate implementation was changed, and no old source was described as current. Host-side concurrent mutation is outside a read-only container mount's protection; the pin checks detect observed changes but are not malicious same-UID isolation.

## Genuine Failure And Follow-Up Evidence

The failing test is:

```text
experimental::tests::swaps_and_writes_around_actual_hash_reads_report_source_changed
crates/kortex-repo/src/stamp_tests.rs:133
called Result::unwrap_err() on an Ok value: StampedSourceV1 { source: "fn one() {}\n", ... }
```

These locations refer to the frozen source. `read_source_impl` in `crates/kortex-repo/src/experimental.rs` reads/hashes the opened file, invokes the injected mutation, then compares file/parent/root metadata and named identity. It does not reread file content after that mutation. The test's first mutation replaces `fn one()` with same-size `fn two()` in place.

Three subsequent executions of the **unchanged compiled test binary** produced exit codes **0, 101, 101**. A standalone filesystem diagnostic (not an indexed fixture or modified crate) performed 1,000 controlled same-size writes on the native test filesystem: **937 writes changed the bytes while inode, size, mtime and ctime all remained equal**. That observation supports a real timestamp-granularity blind spot in the mutation check; nanosecond fields do not guarantee a distinct timestamp for each write. The isolated passing retry does not supersede the workspace failure or establish safety. No sleep, assertion weakening, test skip or runtime fabrication was used to hide it.

Parent/implementation-owner follow-up is required. Do not promote this frozen source to Linux acceptance. Any implementation correction requires a new independently pinned snapshot and renewed tests; do not edit the accepted snapshot in place. An additional content read can improve detection but must not be described as arbitrary-writer atomicity or adversarial same-UID isolation without a justified consistency contract.

## Local Runtime And Confinement

- Endpoint checked as an actual local Unix socket before mounts: `unix:///Users/chris/.docker/run/docker.sock`, context `desktop-linux`. Subsequent commands pin `--host` to this socket instead of relying on a mutable context.
- Already-local official image, no pull: `rust@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`.
- Docker Desktop server 29.6.2; kernel `6.12.76-linuxkit`; native aarch64 Linux VM; Debian GNU/Linux 12 (bookworm).
- Rust/Cargo 1.99.1. Rust commit `58cf8f9c35b355199727627d07ae34a4e2846d67`; LLVM 23.1.1; target/host `aarch64-unknown-linux-gnu`; Git 2.39.5; GCC/CC 12.2.0.
- Application/build/test processes: UID501:GID20, no added capabilities, all capabilities dropped, `no-new-privileges`, no host ports, network `none` in a private network namespace. Tests can bind loopback inside that namespace.
- Limits verified from saved Docker inspect: 4 CPUs, 8 GiB memory including the memory+swap bound, 1,024 processes. `GNU timeout --signal=TERM --kill-after=10s` inside the container, at most 1,800 seconds; outer subprocess timeout adds 30 seconds. Cleanup forcibly removes only this runner's newly named container. No owned container remained running at the final checkpoint.
- Source and the existing Cargo **registry subtree only** are mounted read-only. No Docker socket, credentials, host home, Cargo credential/config files, model or browser cache is mounted in application containers. Docker uses an empty private CLI config so automatic proxy/credential configuration is not forwarded. The saved environment was checked against its explicit allowlist plus the pinned image defaults.
- Root bootstrap mounts only a newly created empty private target volume. Its sole added capability is CHOWN; it chmods/chowns only that new volume root before all application activity runs nonroot. It never recursively chowns a host path or another owner's volume.
- Private native cache volume: `kortex-local-linux-bf565fa79d1a4c32a615b7eaa0b8b480`, local driver, owner label `io.kortex.local-linux-validation.owner=bf565fa79d1a4c32a615b7eaa0b8b480`. It is retained, not deleted. A failed empty bootstrap volume from an earlier runner check is also retained and not reused.

Inspect/configuration assertions and their evidence hashes are recorded in `target/linux-independent-checkpoint.json`. This is sandbox functional evidence, not proof against a compromised kernel/daemon.

## Cargo Cache, Clock And Filesystem Diagnosis

The previous two 300-second timeout reports/logs remain untouched. They concern a different 137-file source pin, `ef07896a3f93642cea6f1ce6630467f87420b6ee0d9bbb7834c3785d15f519f3`, not this 160-file accepted snapshot. `target/linux-cache-diagnostic.log` also remains intact.

The old 6.3 GiB host-bind target was mounted read-only and copied, preserving metadata, into the new private native Linux target. No original cache was chowned or removed. Container paths remain `/src`, `/build`, `/work/cargo` and `/work/home`; `/work` is a subdirectory mount of the same private volume. Cargo registry/home paths are stable across retries. No host Cargo configuration is used.

The observed native target/work filesystem is the Linux `/dev/vda1` filesystem (reported by `stat -f` as `ext2/ext3`). Source/registry/old target use Docker Desktop's host-sharing filesystem (`fakeowner` in `df -T`, magic `0x6a656a63` in `stat -f`). The VM reported 15 CPUs and 33,599,774,720 bytes RAM; 49 unrelated containers were reported running. At run start, about 26,008,612 KiB were available in `/proc/meminfo`, load averages were 1.18/1.18/1.53, and host disk free space was about 300 GiB. Other containers were not stopped or changed. No OOM kill occurred.

Actual Cargo findings:

1. A preserved copy of the old cache was diagnosed under the same absolute Cargo paths. Fingerprint logs identify the frozen `kortex-core/src/lib.rs` mtime (`1794436919.824632397`) as newer than the cached dep-info reference (`1794433509.000000000`), causing root/transitive recompilation. Recapturing identical source into a newly written snapshot can also cause this kind of invalidation; the new runner does not recapture it.
2. The same diagnostic identifies missing `tract_core` and `rmcp` rlibs, stale transitive dependency timestamps, and absent fingerprints for newly added test/example targets. A timed-out or materially older target is not a complete warm cache. Most registry dependencies were reusable in the substantive run; no CARGO_HOME drift was observed at the stable guest path `/work/cargo`.
3. Workspace-all-features, package-only CLI and package-only indexer invocations have distinct feature/dev-dependency resolution graphs and produce additional dependency variants. Those package-specific rebuilds are not evidence that an unchanged workspace cache was lost. Do not change flags between attempts and call them identical warm builds.
4. After the substantive run, unchanged all-feature workspace `cargo test --no-run` completed in **3.83 seconds** with no compilation, demonstrating genuine cache reuse. The native seeded workspace compilation in the initial run took **3m44s**; total elapsed time including seeding, application tests, explicit CLI build and journey was **365.38s**.
5. Cargo 1.99.1's effective log target is **`cargo::compiler::fingerprint`**, not the older `cargo::core::compiler::fingerprint` path. The first substantive run's older directive emitted no fingerprint details; dedicated follow-ups captured the actual diagnostics and the runner directive is corrected. The full warm trace is retained, not represented as an application-test pass.
6. UTC observations agree between host and container and the inspected source/registry times are not in the future. Clock skew was not demonstrated as the Cargo cause. Separately, the actual application race demonstrates equal Linux file timestamps across real byte changes.

A bounded n=1 filesystem diagnostic created/wrote/stat-ed 1,000 4-KiB files: native volume **57.179ms**, host bind **385.418ms**. Ten thousand warm metadata calls took **5.182ms** native and **4.556ms** host bind. This supports host-bind small-file write overhead, not a blanket claim about all reads, a causal partition of prior build times, or a reference performance result. It did not fsync or simulate power loss. Historical timeout logs lack the fingerprint tracing needed to attribute every old registry rebuild retrospectively; the concrete findings above are from the retained cache and current controlled follow-ups.

## Evidence Paths

- Machine checkpoint and evidence hashes: `target/linux-independent-checkpoint.json`.
- Original substantive report: `target/local-linux-validation/native-apkhcu1u/report.json`.
- Application tests/build/journey: `target/local-linux-validation/native-apkhcu1u/kortex-tests-37a0b2cf72274af6879a63c36f724d37.log`.
- Container config/state: same directory, corresponding `-inspect.json` and `-state.json` files.
- Unchanged failing test repetitions and native timestamp probe: `target/local-linux-validation/native-apkhcu1u/kortex-diagnostic-1e19953098c24c7395e39c41d0d62354.log`.
- Actual old-cache fingerprint reasons: `target/local-linux-validation/native-apkhcu1u/kortex-oldfingerprint-5c1ac8f06d2e4c80b28e898612fa9551.log`. Its eight-second inner cutoff is deliberately diagnostic, not a runtime test attempt or a passing compile claim.
- Warm full Cargo trace: `target/linux-cargo-warm-trace.log`.
- Filesystem I/O diagnostic: `target/local-linux-validation/native-apkhcu1u/kortex-io-c5cce322cbef4adbbadeb5e9aad193b9.log`.
- Final runner tests: `target/linux-runner-unit-tests-final.log`.
- Read-only-cache diagnostic refusal and bootstrap failure logs are retained as well; neither is application evidence.

## Reproduction And Remaining Bounds

Runner unit checks (no Docker/application execution):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s benches -p test_linux_validation.py -v
```

Explicit same-snapshot bounded retry, only if useful to an authorized investigation:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benches/linux_validation.py \
  --snapshot /Users/chris/projects/vcsoc/kortex/target/local-linux-validation/accepted-wave-phedqvpr/source \
  --expected-source-sha256 199b64509c4d9683871575f57cac9f2ea39557b6711fdd4652720de1e470fca4 \
  --expected-source-files 160 \
  --resume target/local-linux-validation/native-apkhcu1u/cache-state.json \
  --timeout 1800
```

Do not run repeated retries merely to obtain a green outcome from the observed race. Each invocation creates a fresh uniquely named evidence directory and preserves previous reports/logs; resume checks the image/endpoint/source identity and private volume label, without root bootstrap/chown. Without `--resume`, it creates a new private local volume. Optional `--seed-cache` accepts a read-only old host target under the validation evidence root.

If all application gates pass on an appropriately pinned snapshot, the runner installs the trusted debug executable into a new private tree, archives/extracts it inside the volume, compares executable bytes and hashes, and exercises extracted `version/index/query/context/impact/doctor` against inert two-file source with a separate home/registry. It verifies actual JSON semantics, not only exit status. This path was **not reached here**, and therefore is not installation evidence.

No public artifact, upload, hosted execution, image build/push, implicit model/browser install, fixture-code execution, approval variable, commit, reset, clean, release or deployment occurred. Only the two owned runner/test files, this new document and generated evidence under `target` were written. This does not establish x86 Linux/Windows behavior, full release packaging/signing, native-hardware performance, hardware power-loss behavior, all supported platforms or full production acceptance.
