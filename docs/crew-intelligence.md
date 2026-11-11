# Application intelligence checkpoint

Runtime checkpoint only; parent integration/production acceptance is separate. Owner policy is CLOSED SOURCE / PRIVATE, with no third-party use/access license or public release authorization. All three owned crates inherit `license-file.workspace = true`; no public SPDX license was added for first-party code. No delegation, commits, destructive Git actions, root manifest/lockfile manual edits, or peer source edits. Cargo performed ordinary dependency resolution; parent owns lockfile reconciliation.

## Changed owned files

- `crates/kortex-analysis/Cargo.toml`; `src/lib.rs`, `changes.rs`, new `contracts.rs`, `intelligence.rs`, `rules.rs`, `test_map.rs`, `intelligence_tests.rs`, `intelligence_extra_tests.rs`, `inventory_tests.rs`, `schema_tests.rs`.
- `crates/kortex-graph/Cargo.toml`; `src/lib.rs`, new `control_tests.rs`.
- `crates/kortex-search/Cargo.toml`; `src/lib.rs`, new `hybrid.rs`, `local.rs`, `local_tests.rs`, `retrieval.rs`, `retrieval_tests.rs`.
- This document. Owned-only rustfmt also follows the existing analysis test modules.

Read AGENTS, full specification, implementation ledger, readiness contract and prior changes handoff before work. GitNexus impact queries returned unindexed; direct Service/transport/search/graph caller inspection was the fallback. Existing Git `--ignore-submodules=dirty`, `--submodule=short`, explicit partial submodule comparisons and their regression are preserved. The only logical change in `changes.rs` is test recommendations using explicit evidence or existing persisted test-kind nodes; Git safety behavior is not rewritten.

## AVAILABLE shared contracts

Unchanged `Service::execute(&self, operation: &str, args: &Value) -> Result<Value>`. Exported `pub fn input_schema(operation: &str) -> Value` and `pub fn response_schema(operation: &str) -> Value`, with versioned IDs, concrete data/edge/symbol/span/route/process/change definitions, not generic data placeholders. Operations: status, query, context, impact, trace, graph, changes, routes, check, files; separate list_repos page schema. Data objects add `schema_version:"1.0"` and `operation` while retaining existing fields. Unknown input fields, types, versions and bounds are validated.

Common Service fields: `require_fresh`, `generation_id`, `max_response_bytes`, `timeout_ms`. Public query bounds remain policy constrained. JSON byte budgets repaginate independent query/routes/files items without skipping, or refuse indivisible oversized evidence objects. Whole-wire framing and token budgets remain transport responsibilities.

Shared query/routes/files continuation: opaque `kx1.<repository-generation-operation-request hash>.<offset>`. Transports MUST carry these unchanged, not parse numeric offsets or double-wrap them. Numeric input remains a legacy compatibility adapter, never emitted. Preserve the request filters (including an explicitly supplied generation_id) across pages; supplying/removing optional request keys can invalidate continuation. Query filters `language`, `kind`, `path_prefix` apply before ranking/pagination. New `list_repos_page(args)` has bounded registry pages and registry-content-pinned `kr1` cursors; old unpaged `list_repos()` is a compatibility adapter.

`Service::open_with_config` now loads config once at the invocation Git root, resolves `Config::resolved_root` once, and opens `Store::open_with_storage`. Additive `Service::open_at_root(root, config)` accepts an already selected effective root/config without Git expansion or applying relocation again. Index callers with an effective root must use peer `index_at_root`, not reapply `index`.

### files — ready for HTTP/explorer

Args: `{"path_prefix":"src/","limit":20,"cursor":null,"generation_id":"<optional pinned generation>"}` plus common controls. All fields optional; limit1..500. Returns the real `snapshot.files` inventory, sorted by path:

`data:{schema_version:"1.0",operation:"files",items:[{path,language,size_bytes,content_hash}]}`.

This is not wildcard search. Pagination, prefix changes, generation mismatch, real hashes/byte sizes and output JSON Schema are covered by tests.

### routes

Args: `{"limit":20,"cursor":null,"framework":"fastapi","path_prefix":"/api","include_processes":true,"depth":3,"max_nodes":500}`. Optional framework enum fastapi/flask/express; omitted means all. limit1..500; normal graph policy bounds.

Data: `{schema_version,operation:"routes",items:[{route_id,framework,http_method,path,handler_id,location,confidence,resolution,provenance,consumers,tests,process,partial,truncated}],diagnostics,partial,truncated}`.

- Actual Tree-sitter framework import, unshadowed constructor/application binding, declaration order, literal registration and indexed handler declaration are required. Source must match the indexed whole-file hash before AST extraction. Known local Python framework shadow modules, receiver/factory rebinding, nested registrations, late arrow handlers, dynamic method lists and duplicate handlers are not guessed exact.
- Supports module-level FastAPI/Flask decorators, Flask literal methods, Express ESM/CommonJS/TypeScript direct registration with a uniquely bound same-file handler. APIRouter/Blueprint mounting, middleware chains, imported/dynamic/inline handlers and implicit Flask HEAD/OPTIONS expansion remain unsupported; recognized unsupported forms produce diagnostics.
- Consumers are indexed incoming exact/inferred CALLS, explicitly kind `direct_handler_call`, NOT invented HTTP requests. External HTTP request/origin matching is not implemented.
- Process algorithm `bounded-call-v1` follows CALLS only, returns edge paths plus full edges, entry/route/model-generation provenance, heuristic=true, ambiguity/unresolved counts, partial/truncated. These response facts are not persisted process/route nodes.
- Route/context test maps require explicit Rust #[test], imported unshadowed unittest.TestCase method AST, or actual persisted TESTS edges, plus graph reachability. Name-only fixtures do not become verified framework tests.

### impact/test compatibility

The existing `likely_tests` field also retains reachable **persisted kind=test nodes** through reverse dependency paths, as required by the original CLI journey. Additive `likely_test_associations` includes real edge paths and labels classifier-only recommendations `framework_verified:false, heuristic:true`, basis `persisted test node classification ... not independent test-framework verification`. No test name is inspected to manufacture a relationship. This preserves the unchanged CLI assertion while keeping weaker classification distinct from verified route/context test maps. Context additionally exposes incoming REFERENCES, outgoing USES_TYPE and test association paths.

### check

Args: `{"rules_version":"1.0","rules":[{"id":"no-ui-db","kind":"forbid_dependency","from":{"path":"src/ui/**","kinds":["function"]},"to":{"path":"src/db/**"},"edge_kinds":["CALLS","IMPORTS"],"max_depth":1}],"max_violations":100}` plus common controls.

Only validated inline rules; no arbitrary rule-file path, remote config, executable policy, SQL or shell. rules_version optional default1.0; rules1..100, unique nonempty IDs<=128 bytes; max_violations1..500. Relative path glob supports `*` one segment, `**` all segments, `?` one Unicode character; traversal, absolute paths, backslashes, brackets/braces and unknown fields/kinds rejected. Kinds are optional selector filters. Dependency edge kinds default CALLS/IMPORTS/REFERENCES/DEPENDS_ON/INHERITS/IMPLEMENTS/USES_TYPE; explicit READS/WRITES/TESTS also supported. max_depth default1, range1..10.

Data: `{schema_version,operation:"check",rules_version,passed,complete,exit_code,violations:[{rule_id,source_id,target_id,edge_path,edges,confidence}],uncertain:[{rule_id,edge_id,resolution,candidates,evidence}],rules_evaluated,evaluated:[{rule_id,sources_examined,edges_examined,complete}],partial,truncated}`.

- 10: evidenced violations, even if report also incomplete.
- 7: incomplete without known violations (uncertain/dangling/invalid-confidence edges, syntax/index diagnostics, stale generation, bounded evaluation).
- 0: complete clean evaluation only. Deadlines return Error exit9. Invalid rules return Error exit2.
- CLI must exit `data.exit_code` while preserving successful report JSON. MCP/HTTP return the report, not synthetic transport errors for violations. `rules_evaluated` reflects started rules, not unvisited rules after a cap. Evaluation is scoped to selected kinds/depth over the indexed graph, not a claim of compiler-complete dependencies.

## Native local model work — experimental, NOT production accepted

Production implementation exists in optional search feature `local-embeddings`: `local::NativeBert::load(LocalModelManifest)`, `hybrid::verify_model`, bounded `EmbeddingCache`, weighted reciprocal-rank fusion, `retrieval::query_hybrid` with candidate/limit/filter bounds and cooperative deadline/cancellation, and graded MRR@10/nDCG@10 evaluation helpers. No keyword hashes are labeled semantic embeddings. No model downloads, hub clients, remote providers, source uploads, pickle/Python or custom repository model code. The native backend is SDK-only; it is not silently enabled by transports. Service query explicitly returns `LOCAL_MODEL_GATE` exit7 when local embeddings are configured rather than pretending to honor them with keyword results. Keyword queries remain functional with embeddings=disabled.

### Declared dependencies and blocking audit

Analysis reuses existing workspace serde/Tree-sitter/Rust/Python/JS/TS grammar pins (existing licenses/provenance); dev-only indexer and jsonschema0.33 with default features disabled validate actual index-to-service contracts. Search reuses workspace BLAKE3 for streaming artifact checksums and dev indexer/config for real integration fixtures.

Initial native versions Candle0.9.2/tokenizers0.22.2 passed six real inference/retrieval tests but parent all-feature audit found vulnerable ZIP7.4 plus unmaintained paste1.0.15. They were NOT accepted. Current manifest upgrades EXACT Candle core/nn/transformers0.11.1 (MIT OR Apache-2.0; default features off, CPU only, no accelerators) and tokenizers0.23.1 (Apache-2.0; default features off, fancy-regex only; no hf-hub/http/TLS/onig/training). License/features checked via cargo info before changes; parent notified. Cargo resolves ZIP9.5.1, removing the four reported old ZIP CVEs (RUSTSEC-2026-0367/0368/0369/0429). Current production search library compiles.

**Exact remaining blocker: Candle0.11.1 still uses GEMM0.19/pulp and stable tokenizers0.23.1 also uses paste1.0.15, RUSTSEC-2024-0436 unmaintained. No suppression, fake wrapper, vendored fork or owner risk acceptance was added.** paste is a compile-time proc macro in dependencies, not runtime inference/indexed-source macro expansion; this reachability distinction does not waive the supply-chain policy gate. Parent must rerun the audit and either obtain a maintained chain/runtime or an explicitly authorized narrow owner decision. Public/private production release remains blocked independently.

Researched alternative only, NOT installed/accepted: tract-onnx0.23.10 (MIT OR Apache-2.0) uses maintained pastey; current tokenizers1.0.0-rc.4 is a rewritten split implementation, whereas stable0.23.1 still pulls paste. No blind RC/backend replacement was performed. No new tract/ort/RC dependencies were added to the project. Their manifests/API/audit/conformance would need proper evaluation before adoption.

### Artifact contract and exact missing evidence

`LocalModelManifest` is deny-unknown-fields JSON: `{schema_version:"1.0",model_id,revision,license,architecture:"bert",model:{path,blake3},tokenizer:{path,blake3},config:{path,blake3},dimensions,max_tokens,pooling:"attention_mean_l2"|"cls_l2"}`. Paths are explicit absolute local files, lowercase BLAKE3 checksums. Bounds: model512MiB, tokenizer32MiB, config1MiB; native BERT architecture/token dimensions additionally bounded. Bytes actually consumed are rehashed on load, not only checked in a prior path lookup. WordPiece/BertNormalizer/BertPreTokenizer/TemplateProcessing tokenizer components only; custom regex tokenizer programs refused. Mean pooling uses actual attention masks, then L2 normalization. Content/model/revision/config/pooling keys distinguish cached vectors; bounded memory FIFO cache. Persistent cache_path integration remains storage work.

Required for genuine pretrained acceptance: owner-supplied `sentence-transformers/all-MiniLM-L6-v2` model.safetensors, tokenizer.json, config.json (upstream Apache-2.0), an owner-selected immutable upstream revision and exact checksums/license record. Expected6-layer encoder, hidden384,12 heads, intermediate1536,vocab30522,max sequence256, attention mean+L2. No pretrained artifact/revision/checksum has been supplied or downloaded. The generated tiny BERT fixtures exercise actual CPU transformer inference and tokenization; they are NOT pretrained semantic quality/relevance evidence. A curated representative dataset and measured relevance improvement remain required. Config proposal for a later accepted backend: treat trusted search.model_path as this JSON manifest, model_checksum as the manifest digest, model_version as manifest.revision; no transport model-path input. This proposal is not activated.

## Commands and observed results

1. `cargo test -p kortex-analysis -p kortex-search -p kortex-graph --all-features`: **43 passed** (33 analysis,4 graph,6 search), before subsequent inventory/root/compatibility additions and before Candle upgrade. All17 preexisting analysis/Git safety tests remained passing. Native fixtures execute actual CPU BERT forward passes, token-dependent normalized vectors, artifact corruption/config/tokenizer-program refusals, model/content cache invalidation, real indexed hybrid retrieval, cancellation and deadline.
2. `cargo clippy -p kortex-analysis -p kortex-search -p kortex-graph --all-targets --all-features -- -D warnings`: **passed** dependency-inclusive at that same snapshot.
3. Later `cargo test -p kortex-analysis`: **36 passed**, including actual inventory/cursor/hash/schema tests, effective-root-once test and persisted-test-kind reverse CALLS regression.
4. Later `cargo test -p kortex-cli --test journey complete_cli_core_journey`: **1 passed,4 filtered**, with original journey assertion unchanged. No CLI file edited.
5. After stable upgrades, `cargo check -p kortex-search --features local-embeddings --lib`: **passed**. `cargo tree ... -i zip` confirms9.5.1; `-i paste` confirms remaining1.0.15 through stable Candle/GEMM/tokenizers.
6. The upgraded native `cargo test` attempt was **blocked by concurrent peer compilation errors**, not an accepted run: `kortex-languages/src/lookup.rs` E0700 iterator captures; `rust_modules.rs` Lookup/Snapshot type mismatch; `extended.rs`/lib.rs lifetime errors. No peer file edited. Parent is actively integrating its lookup changes.
7. Final `cargo fmt -p kortex-analysis -p kortex-search -p kortex-graph` and the same `-- --check`: **passed**, owned-only.
8. `cargo audit --no-fetch`: unavailable (`no such command: audit`). Parent's actual deny/audit log is `target/production-wave-audit.log`; original audit advisories failed, licenses/bans/sources passed. New ZIP resolution is verified, not a claimed fresh advisory pass.

Last small changes after the36-test run (formatted but NOT rerun due turn checkpoint/concurrent peer compile blocker): shared-cache child integration test and local-config refusal test; Service explicit LOCAL_MODEL_GATE; changes ref schema cap1024; enhanced response-schema fixture with nonempty Git changes/check violations. Parent must rerun the current full tests/lint/audit; do not treat earlier results as final acceptance of these last additions.

## Residual acceptance / next owner actions

- Preserve optional model gate until audit + real pretrained conformance/relevance acceptance; no public release/publishing or remote uploads.
- Parent/transport adopt files and shared schemas/cursors; browser inventory no longer needs a fake wildcard search. Re-run actual HTTP/MCP/CLI/browser tests after peer compilation stabilizes.
- Re-run latest owned/all-workspace tests, lint, coverage and dependency audit. No current full-platform, full100k-symbol/10k-file/1000-sample performance, complete token/wire budget or hard cancellation claim.
- Snapshot loading/freshness still materialize/scan broadly. Graph adjacency/expansion and AST loops check deadlines; filesystem discovery, SQLite operations and one native forward pass cannot be forcibly interrupted mid-call. Legacy registry helper builds all rows; new page helper bounds returned pages, not all intermediate memory.
- Potential measured optimization targets: repeated Snapshot::symbol linear lookups in context/impact/search; existing rules already use a per-pass ID map. Coordinate indexed Store lookup helpers with parent/persistence-platform; do not change resolution semantics blindly.
- Route/test/process response facts are genuine but adapter-bounded; persisted domain-node integration, richer frameworks/test runners, actual HTTP-client consumer origins, native config/cache integration and broad relevance datasets remain future integration work, not fabricated completion.
