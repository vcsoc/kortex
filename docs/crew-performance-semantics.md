# Semantic discovery and resolver-performance wave

**Private/proprietary. No publication, uploads, delegation, commits, resets or cleanup commands.** This is scoped local worker evidence, not P0/P1 production completion. Parent owns integrated acceptance, storage, core/parser version, root manifests/lockfile and shared docs. All edits are under `crates/kortex-languages/**`, `crates/kortex-indexer/**`, and this document; no new dependencies. Existing parse implementation is unchanged.

## Completed vertical slice

`crates/kortex-indexer/tests/p1_discovery.rs` exercises actual repository discovery (not manually supplied parsed facts), grammar selection, `index_at_root`, global resolution, SQLite graph/files/FTS publication, readback and keyword queries for **Java, C#, C, C++, Ruby and PHP**. Repository owner's extension mappings are now present. Parent's parser version is `tree-sitter-0.25-kortex-7`.

For EACH provider, the fixture asserts:

- Three files really discovered and parsed with the expected provider; unrelated same-name declarations do not satisfy missing calls.
- One exact cross-file target, one two-element ambiguous candidate set without an exact target, and one unresolved negative.
- Persisted graph/files/generation match the indexer report; every check declaration appears in real FTS results.
- Unchanged generation reuses all three parsed files.
- A semantic edit changes both declaration/call `check` to `verify` (two parsed, one reused); old declaration disappears.
- Provider file rename/relocation and path-based import updates (C/C++/Ruby) yield the right new file target and no old file.
- Deletion reuses the two remaining files, removes exact/ambiguous targets and removes stale check declarations from FTS.
- At initial/edit/rename/delete stages, real incremental and forced-full **files and edges are identical**, with persisted readback for each. This retains the safe global-resolution fallback, not selective invalidation.

The opt-in `actual_cli_discovery_query_and_context_for_all_six_p1_languages` test was ALSO executed, not merely compiled/ignored: a freshly built trusted CLI is privately copied once, HOME/config/cache/registry are isolated, each language is indexed, real `query check` returns its persisted declaration and real `context path:check` returns exactly the expected caller edge. Fixture source is never compiled or interpreted. Indexer's full edit/rename/delete matrix above is not mislabeled as six CLI mutation journeys.

Root/cache regressions remain passing: already-selected `index_at_root` roots are not expanded/relocated again, configured storage is preserved. No storage/root/config source was edited.

## Labeled precision/recall: narrow and explicit

Run `cargo test -p kortex-indexer --test p1_discovery --locked --offline labeled_p1_callable_extraction_and_resolution_report -- --nocapture`.

These denominators cover ONLY all callable declarations (path/name multiset) and CALLS in the three-file handwritten fixture for each provider. They do **not** measure all language constructs, ownership/signature accuracy, all references, source-span precision, compiler semantics, real repositories, dynamic runtime targets or general language accuracy. Separate existing tests cover spans, values/types, malformed syntax, unknown receivers, macros/cfg, visibility and scopes, but have no invented aggregate accuracy score.

| Provider | Callable TP / FP / FN | Fixture callable precision / recall | Call sites extracted / labeled | Exact-target TP / FP / FN | Exact-target fixture precision / recall | Correct ambiguous sets | Correct unresolved negatives |
|---|---|---|---|---|---|---|---|
| Java | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |
| C# | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |
| C | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |
| C++ | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |
| Ruby | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |
| PHP | 5 / 0 / 0 | 5/5 / 5/5 | 3 / 3 | 1 / 0 / 0 | 1/1 / 1/1 | 1 (two candidates) | 1 |

The exact-target recall denominator is the ONE labeled statically exact site, not the ambiguous/negative sites. Candidate-set correctness is separately asserted against actual target IDs/files; it is not converted into an exact call. C has conflicting declarations, Ruby/PHP have redefinitions/conflicting declarations: this does not claim compiler-valid overload support. No dynamic receiver site occurs in this particular metric population (`unknown_runtime_targets=0` means absent from this fixture, not zero unknowns in the language).

Unknown/out-of-coverage examples: Java/C# argument-type overload selection, inheritance/virtual dispatch and full visibility/generic analysis; C/C++ preprocessing, build flags, linker semantics/function pointers; Ruby metaprogramming/load paths; PHP Composer/autoload/dynamic dispatch. Existing conservative unresolved behavior remains. Broader held-out labeled corpora and P0-wide precision/recall are still gates.

## Resolver changes and safety

`crates/kortex-languages/src/lookup.rs` builds borrowed per-pass lookup tables for ID, file path, per-file name, language/name, language/qualified name, owner children and file directories. Java/C#/PHP namespace-prefix module lookup and Rust canonical paths/public reexports/conditional-module prefixes are registered once. Stable module-file and imported-name candidates, including empty results, are cached only inside that immutable pass. Keys retain source file/module/name/Go module context. No static/global cache survives edits or generations.

`resolve`, `owners`, P1 candidate lookup, Rust module lookup, receiver lookup and relationship completion use these tables instead of repeated `Snapshot::symbol`/whole-workspace name/file scans. Existing scope, availability, type/value kind, mutation/shadowing, package, visibility, export, static receiver, macro/cfg, prototype-vs-definition, overload and dynamic-candidate filters remain in force. Reexports retain their visited-set and 32-hop bound. Candidate IDs and final edges remain sorted/deduplicated. ID lookup preserves first-match snapshot semantics.

`resolve_with_context` avoids cloning a whole snapshot when no JS/TS alias transformation can occur (no alias/baseUrl inputs OR no JS/TS files). This is provably unchanged semantic input; it still executes full resolution. Derived CONTAINS/DEPENDS_ON/READS edges are appended from a bounded original-edge prefix instead of cloning the entire edge vector.

Focused tests:

- Borrowed lookups compared with independent linear name/module-prefix queries, with overload multiplicity, language separation, directory/child membership and deterministic file order.
- 100 repeated cached module/name requests including missing targets; a new pass after provider removal cannot retain old targets.
- 256 repeated Python imported calls plus a shadowing parameter and unrelated missing name; provider removal leaves all calls unresolved.
- C++ -> C headers through cyclic transitive includes, prototype superseded by actual visible definition, inaccessible same-name file excluded.
- Cyclic Rust reexports preserve public exact, private unresolved and absent unresolved behavior.
- Context fast path and alias-transform path produce identical relative-import edges.
- All previous 49 owned-crate tests remain and pass.

GitNexus impact was attempted before existing production function/method changes (including resolve, owners, module_candidates/module_files, P1 imports/candidates, receivers/type_targets, Rust lookup/source/conditional/imported/qualified/Index, relationship completion, context resolution and lookup helpers). Every call returned **unindexed repository**. Direct source/caller tracing and the above real tests are fallback evidence, not a graph-tool blast-radius result or independent review. Parent independent review remains required.

## Reproducible diagnostics, NOT acceptance

New owned diagnostic examples:

```text
cargo build -p kortex-indexer --examples --release --locked --offline
/usr/bin/time -l target/release/examples/resolution_profile 1000 10 7
/usr/bin/time -l target/release/examples/resolution_profile 10000 10 1
/usr/bin/time -l target/release/examples/resolver_scale 10000 10 7
/usr/bin/time -l target/release/examples/resolver_scale 10000 10 1 publish replace
```

`resolution_profile` creates real files and invokes indexer discovery/persistence; `resolver_scale` isolates real grammar parsing and resolution of fixed-ID facts, optionally calls actual Store publication. Neither executes generated Python. Both explicitly print `diagnostic_only` / no acceptance. Generated data: circular cross-file imports, ten functions/calls per file, 11 symbols per file including file nodes, 32 edges per file. Hash input is ordered `path + NUL + source` (not the parent's acceptance fixture format).

Fixed dataset SHA-256:

- 1,000 files / 10,000 functions / 11,000 symbols / 32,000 edges: `9dbaa66892211268a296259156174e931058809ea75c2a838e38483faa88c961`
- 10,000 files / 100,000 functions / 110,000 symbols / 320,000 edges: `3d03be3f05a84b6a4963bf410c12ce055a8a5fac588e88d6815f12cfafa89bf3`

Environment: local aarch64 macOS Darwin 25.5.0, Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.3), optimized Cargo release profile, parser `tree-sitter-0.25-kortex-7`, default bounded indexing pool (at most eight workers); isolated parser example explicitly uses eight workers. Checkout has **no HEAD commit**, so binary SHA is the exact executable version identifier. Peer code was not frozen: build/package-cache locks and recompilation occurred; one process snapshot showed a competing `target/release/kortex` at ~99% CPU and host VM/FSEvents activity. Later publication diagnostic overlapped focused debug tests/clippy. None of these runs is claimed quiescent. Ratios are not controlled statistical attribution. Temporary repository paths change repository/symbol/edge IDs, so index-profile edge hashes differ across runs despite fixed source hashes; the fixed-ID phase diagnostic has stable graph hash below.

### Before/after fixed-dataset observations

| Diagnostic version | Full index, ms | Resolver samples: median / nearest-rank p95, ms | Single comment index + publication: median / p95, ms | Whole diagnostic max RSS, bytes |
|---|---:|---:|---:|---:|
| Before lookup, 1k files, n=7 | 2382.616 | 639.052 / 713.749 | 2434.452 / 4554.709 | 296550400 |
| Initial lookup version, 1k files, n=7 | 1365.838 | 40.521 / 46.001 | 2033.688 / 2522.150 | 314490880 |
| Preserved pre-lookup binary, 10k files, requested n=1 | 132784.517 | **incomplete**: diagnostic killed at 240 s while next resolver pass ran | not reached | not captured |
| Final lookup/no-edge-clone version, 10k files, n=1 | 14863.526 | 438.771 (one sample, NOT a distribution claim) | 44462.461 (one sample) | 2266644480 |

Before binary: `c66cde986d646fd6b7ad1b68a3fff230b722bdf686519cac092192c33023f0c7`, preserved as `target/release/examples/resolution_profile-prelookup`. Initial lookup binary: `84d0ad99b69750b7168e49b31d6c219fb93725c5a7766500be63e9a68477a923`. Final index diagnostic binary: `ade9ecc2a548d806c89852508c38446f7246ccf74d54312c147bbc50e68405ee`. The timeout left no running profiler process at the subsequent process check. It is not reported as a completed old resolver sample.

Parent's separate old-prewave-release acceptance-harness diagnostic timed out at 45 seconds BEFORE full indexing completed, with `runs_complete=false` and no query metrics: `target/scale-baseline-prewave-100k-nonquiescent.json`, dataset `5b8b8cf55f5254c8f1c5623ec68a8b538610a7c72a7e54ccf21c5029cfa2ac48`. That is a different fixture/binary and demonstrates scale failure, not profiler attribution or a comparable latency distribution.

### 10k-file phase isolation

Seven-pass initial-map diagnostic: parsing 187.656 ms; resolver samples 502.938, 622.259, 531.163, 525.105, 439.274, 430.304, 457.916 ms; median 502.938 / nearest-rank p95 622.259 ms. Default-context resolution 436.302 ms. Binary `95394d09bc5bd706f0cbc01b400252c23bf9049680e7b942910db4b957d4ad33`, max RSS 1727873024 bytes including JSON graph hashing/validation and allocator retention. RSS is not a resolver-only allocation measurement. Stable serialized edge SHA `3abc9f84083b92b243ee10e237adace0a44172827362761dbb4e90400306e734`. All 100,000 calls are checked against the actual expected next-file `f0`, not merely counted as exact.

After removing whole-edge cloning: one-pass publication diagnostic binary `7543e4cc076e45f8e2d37125991a7957a13d113bf7042d7f3a51bac47782f429`: parse 174.754 ms; resolve 501.408 ms; default context 439.458 ms; first SQLite commit **12693.798 ms**; load/edge validation 954.792 ms; max RSS 1459601408 bytes. This is not a controlled memory improvement claim (different sample counts).

Latest explicit replacement diagnostic binary `38eeeae502ccb18bf0a90fef8dc1d3851f323fcd109317ec81909725d0398a01`: parse 173.820 ms; resolve 452.289 ms; default context 449.288 ms; first SQLite commit **15960.399 ms**; load/edge validation 530.532 ms; **replacement Store::commit 36573.437 ms**; max RSS 1724710912 bytes. Same stable graph SHA. One sample per publication phase, with concurrent focused checks: no percentile/target claim. This independently isolates substantial publication cost without assigning the parent's old timeout to storage.

### Concrete parent/storage-owner requests and residual scale gates

1. Profile/reuse prepared SQL statements across file/node/FTS/edge loops in `crates/kortex-storage/src/lib.rs:140–180`; current source calls `tx.execute` for each row. This fixture writes 10k file rows, 110k node rows, 110k FTS rows, 320k edge rows per full generation, then deletes the previous generation/FTS.
2. Profile replacement-generation deletion/FK/FTS work and repeated whole-snapshot/file/symbol/edge serialization. Actual isolated replacement commit is 36.57 s versus ~0.45 s resolution in this run. Preserve atomic publication, pinned-reader semantics, locks, referential integrity and crash rollback while changing storage.
3. Memory remains a gate: the end-to-end diagnostic reaches ~2.27 GB, and phase diagnostics include hashing/validation overhead. Obtain phase-specific allocation/RSS evidence and profile snapshot/JSON duplication; do not cite these as acceptable production memory.
4. Parent must rerun its unmodified reference acceptance harness against a stable current binary, with required 1,000 query/context/impact samples, 100 startup samples and 30 updates, quiescent contention notes and real target enforcement. No query, watcher, cold-cache, cross-platform or full reference-performance acceptance is claimed here.

## Exact worker validation

```text
KORTEX_TEST_CLI=/Users/chris/projects/vcsoc/kortex/target/release/kortex cargo test -p kortex-parse -p kortex-languages -p kortex-indexer --locked --offline -- --include-ignored
cargo clippy -p kortex-parse -p kortex-languages -p kortex-indexer --all-targets --locked --offline -- -D warnings
cargo fmt -p kortex-parse -p kortex-languages -p kortex-indexer -- --check
```

Final complete owned suite INCLUDING the opt-in CLI test: **58 passed**, zero failed/ignored, all doc tests pass. Strict all-target clippy and owned fmt check both PASS after that final run. Default runs without `--include-ignored` have 57 passing tests and one explicitly ignored external-binary test. The CLI test was also explicitly executed separately and passed for all six languages:

```text
cargo build -p kortex-cli --release --locked --offline
KORTEX_TEST_CLI=/Users/chris/projects/vcsoc/kortex/target/release/kortex cargo test -p kortex-indexer --test p1_discovery --locked --offline actual_cli_discovery_query_and_context_for_all_six_p1_languages -- --ignored --nocapture
```

Final owned-source/manifests SHA-256 (24 ordered `.rs` / `Cargo.toml` files; each relative path + NUL + contents + NUL): `18a8be9002242b9ac4ac37830e373ba666bdf17b155bd424050fd1323e2a8b14`. CLI SHA `df256a2c36c53abac4bc47c5432f4768b1b19d8f5cc41b91ec725504f0570efc`. Build and six real index/query/context journeys PASS. Default suite intentionally labels this external-binary test ignored unless explicitly requested; it is not silently counted as default coverage. Earlier temporary Rust borrowed-tuple iterator lifetime compile errors were corrected by nested borrowed-key maps; final test/clippy checks above passed. Owned-only formatting, never workspace formatting. No edits to parent acceptance harness or peer CLI/storage, no first-party unsafe, no new dependencies, no manual manifest/lockfile edits. Runtime completion of this slice is separate from independent review and production acceptance.
