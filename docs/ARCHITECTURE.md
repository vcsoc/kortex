# Architecture

CLI and MCP call the same `kortex-analysis::Service`. Transport code does not implement independent resolution/search behavior. `kortex-core` is the embeddable serde model/error/identity boundary. `kortex-config` loads strict policy. `kortex-repo` provides confined discovery and argument-safe Git reads. `kortex-parse` wraps independently distributed Tree-sitter grammar crates; `kortex-languages` performs a separate conservative lexical/import resolution pass.

`kortex-indexer` scans and hashes, reuses immutable parsed facts, executes changed parses in a bounded Rayon pool, globally re-resolves dependencies, verifies source hashes and commits through `kortex-storage`. SQLite bundles FTS5. Graph/search services use deterministic tie-breaking and bounded breadth-first traversal. `kortex-http` is a reserved crate boundary only; it is not an implemented transport.

Native watch registration uses individually confined, non-symlink targets, disables backend link traversal, and excludes internal index/Git directories. Initial registration has depth/scan bounds. New/replaced targets and missed events remain covered by two-second inventory reconciliation; native delivery is not claimed lossless. Per-file descriptor scaling and hostile concurrent replacement remain acceptance gates.

## Generation invariants

One held advisory lock per canonical repository serializes index writers. Staging graph/files/FTS/metadata and the active-generation pointer are changed in one SQLite transaction. FK violations or process death roll back the transaction. Readers open read-only connections and hold SQLite read transactions, pinning both graph and FTS to the same committed generation. Obsolete rows are transactionally removed; WAL retains pages needed by older readers. Long-running readers may increase WAL disk use.

Snapshots are serialized for consistent Rust service reads, with relational nodes/edges/FTS for lookup and integrity. This duplicates data and currently materializes the whole graph in memory. Large-scale benchmarks and a bounded connection/snapshot pool are still needed. Every query opens a fresh service/snapshot; persistence is not a cloud dependency.

## Extension contract

`LanguageProvider` returns `ParsedFile` facts; it never resolves by looking for unrelated names. Resolver edges explicitly distinguish exact/inferred/ambiguous/unresolved. Dynamic/unsupported constructs stay unknown. New providers must add positive, negative, ambiguity, syntax-error, span and cross-file fixtures. See LANGUAGE_SUPPORT.md for current omissions and IMPLEMENTATION_PLAN.md for the acceptance ledger.
