# Implementation plan and requirement ledger

Authoritative source: `KORTEX_SPECIFICATION.md` v1.0.0. `verified` means the named gate was actually exercised locally, not that an entire milestone or a release is complete. No public release is authorized. Four native crew workers were explicitly authorized and launched for semantic graph facts, Git change analysis, repository/storage hardening, and CLI/MCP contracts. They use disjoint source ownership in this shared, uncommitted checkout; the parent owns integration, shared contracts, dependency locking, documentation and final acceptance. No public remote changes have been launched. The owner subsequently authorized continued crew implementation through full P0 and P1 production readiness; current ownership, required evidence and unresolved owner decisions are tracked in PRODUCTION_READINESS.md. The earlier integrated snapshot below is a baseline, not completion of that expanded contract. Worker runtime completion is not acceptance: integrated tests, lint, audit and coverage must be rerun.

| Milestone / spec sections | Implementation | Tests/evidence | Overall status |
|---|---|---|---|
| M0 foundation, §§4,14,17,18 | all 13 workspace crates, pinned toolchain, lockfile, CLI, CI, AGENTS, decisions | local build/fmt/clippy/tests/help/version verified; platform CI pending | in_progress |
| M1 discovery/persistence, §§5,6.1,10 | config, repo, storage, registry, indexer, CLI | ignores, confinement, restart, locking, actual killed transaction verified | in_progress |
| M2 parsing, §§6.2–6.4 | Tree-sitter parse crate; Rust/Python/TS/JS/Go; bounded pool, facts cache | real spans/imports/calls, malformed input, shadowing regressions verified | in_progress |
| M3 graph/search, §§5,7.1–7.2 | languages, storage FTS5, search, analysis, CLI | exact cross-file fixtures; ambiguous/unresolved; search/context/export; pinned reader generation verified | in_progress |
| M4 analysis/incremental, §§6.5,7.3–7.4 | graph BFS, analysis diff, indexer, reconciliation watcher | CLI impact/trace/changes; full equivalence; rename/delete; atomic save/watch/killed writer verified | in_progress |
| M5 MCP, §§7.5,9 | rmcp SDK stdio; eight tools, three resources, input schemas, bounds | real SDK client initializes/lists/calls all tools and reads resources; error test verified | in_progress |
| M6 hardening/release, §§8,12–14,16.2,18–19 | doctor, docs, CI/release preparation, benchmarks and provenance | 97 integrated tests, format/clippy and automated dependency audit pass locally; 90.92% coverage overall, critical 92.30% passes 90%; platform/legal/scale gates pending | in_progress |
| M7 extended languages/embeddings, §§6.4,7.1 | not implemented; offline policy refuses embedding enablement | not tested; keyword mode is functional | not_started |
| M8 HTTP/explorer/intelligence, §§6.6,7.2,11 | HTTP crate boundary only; serve explicitly errors, never synthetic success | non-loopback unauthenticated refusal verified; API/UI/routes/rules absent | not_started |
| M9 advanced roadmap, §15 | separately scoped | separate correctness/security approval needed | not_started |

## Verified vertical slices

- Foundation CLI/help/version, all crates build, format and lint.
- Discovery respects Git/custom/default ignores, confines symlinks, caps files and bytes, hashes contents.
- SQLite WAL, FK constraints, atomic graph/FTS pointer replacement, read-only pinned reader transactions, per-repo advisory writer lock, restart, corrupt-index error, killed transaction rollback.
- Four P0 languages (JS/TS share a family) use real grammars; symbols/spans, imports, lexical owners, call sites, bindings and syntax diagnostics.
- Rust module imports, Python module imports, relative ESM imports and Go module imports resolve fixture calls. Same-name unrelated symbols, scoped imports, local bindings, and non-exported ESM declarations do not become guessed exact calls.
- Exact/prefix/path/keyword ranking, context, JSON/DOT/GraphML graph exports; ambiguity is explicit with no guessed target.
- Bounded upstream/downstream/both traversal, shortest dependency paths, line-hunk Git mapping, stale-index warnings, cached-fact incremental indexing and globally conservative re-resolution.
- Native file events, bounded debounce and periodic watch reconciliation detect content changes, atomic saves, renames/deletions. All generation publication remains transactional.
- Native MCP SDK protocol, eight tools, resources, JSON Schema inputs, errors, request size/concurrency/deadline controls, JSON-safe result budgets.

## Remaining requirement gates (do not claim P0 complete)

| Requirement | Implementation gap / acceptance still needed | Milestone |
|---|---|---|
| §§5.1–5.3 full semantic model | Common fields/properties, locals/parameters and Go struct/interface declarations are extracted. Complete declaration variants, exports, external/reference nodes, namespace/package and process/route facts remain. No rename continuity aliases. No case-insensitive-path collision suite. | M2/M3 |
| §6.2 all references/type facts | Persisted semantic edges are DECLARES, IMPORTS, CALLS, REFERENCES and USES_TYPE, with polyglot positive/negative scope fixtures. General reference/type facts are not exhaustive; inheritance/trait/receiver inference is incomplete. No complete compiler-accurate lexical scopes. | M2/M3 |
| §6.3 language completeness | CommonJS, tsconfig path aliases, Rust glob/reexports/macro/cfg handling, trait candidate sets, Go receiver/interface inference, Python star imports and rich module layouts require implementation/fixtures. Unsupported constructs remain unresolved, not fabricated exact edges. | M2/M3 |
| §6.4 correctness evaluation | Small positive/negative fixtures pass; labeled broad precision/recall and relevance datasets remain to be built. | M2/M3/M6 |
| §6.5 scalable invalidation | Cached unchanged facts are reused, but resolution and SQLite generation serialization rebuild globally. Native notify events now wake a bounded debounce queue, with two-second reconciliation for missed events. Large-workspace performance and overflow/failure injection coverage remain pending. | M4 |
| §§7–9 bounded public contracts | MCP query/registry cursors are generation/content-pinned; bounded resources and encoded tool budgets have real SDK regressions. Complete per-operation data schemas, CLI/shared-Service cursor pinning, strict whole-wire/token budgets, all cancellation paths and hostile-input fuzzing remain. | M3/M5 |
| §7.4 Git edge cases | Content-verified indexed-side hunk coordinates, staged/unstaged/commit pairs, renames/deletions and hostile paths are tested. Nested dirty-submodule inspection is explicitly partial to prevent Git filter execution. Untracked support, concurrent Git snapshot mutation, broad pathological diffs and richer route/test consumers remain. | M4 |
| §8 CLI UX | Human output currently pretty JSON rather than tailored summaries. `serve --mcp stdio` is implemented and SDK-tested. Shared-cache/workspace-manifest indexing absent. `doctor --repair` rebuilds a readable index, not an irrecoverably corrupted DB. | M1/M5/M6 |
| §10 config | Strict TOML layering, nested KORTEX_SECTION__FIELD overrides and legacy flat aliases work; root relocation, schema migrations and multi-repo manifest configuration remain. Discovery flags intentionally cause a different config fingerprint from later default queries. | M1 |
| §12 security | Static confinement and policy tests pass; hostile concurrent filesystem replacement/TOCTOU hardening, disk-full, permissions, JSON bombs, parser fuzz/property tests and rigorous no-network sandbox proof remain. | M6 |
| §§13–14 metrics | Reference-scale 100k-symbol/10k-file p95/RSS and broad correctness metrics remain pending. Overall line coverage is 90.92% (passes 80%); critical repo/storage coverage is 92.30% (passes 90%) in the final integrated local run. Small smoke measurements are published, not reference-scale claims. | M6 |
| §§14,16.2 release | Only local macOS execution verified. Linux/Windows CI configured but not executed here. Local private installation and release build pass; CycloneDX inventory and development packaging exist. Automated advisory/license-policy/source audit now passes after dependency upgrades. Owner has selected proprietary/private policy and prohibits public release. Full SBOM/provenance review, signing, cross-platform clean private installation and binary reproducibility remain pending. | M0/M6 |
| §§17–18 legal/product | Owner explicitly chose closed-source/private with no permission for others to use it; LICENSE records that decision and public packaging is unconditionally disabled. React/TypeScript/Cytoscape explorer stack is approved. Remote embeddings remain disabled; HTTP/explorer are separate unimplemented P1 scope. | release/M7/M8 |

## Crew integration acceptance

All four authorized workers completed. Parent reviewed source, independently ran all 97 workspace tests, format/clippy, dependency auditing and instrumented coverage. Parent also reproduced and fixed a nested Git clean-filter execution path, added its regression, and bumped parser cache version to tree-sitter-0.25-kortex-6. Private release build/install and smoke benchmarks were refreshed. Worker handoffs in crew-semantics.md, crew-changes.md, crew-storage.md and crew-contracts.md retain scoped evidence and limitations; historical intermediate failures are not final acceptance results.

## Next sequence

Finish M1–M3 semantic/config gaps and add their acceptance fixtures before declaring these milestones verified; complete M4/M5 bounds/failure testing; execute M6 platform/security/audit/performance gates. Then implement M7/M8 independently. Do not interpret local happy-path success as full specification acceptance.
