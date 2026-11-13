# Indexing resource boundaries

Implemented aggregate/resolution/AST-admission slice; full resource acceptance remains OPEN. Source fixtures are only parsed/indexed, never compiled or executed. New per-emission work is separately owned and must not be inferred from this slice.

## API and policy

- Add `Config.index.resources: ResourceLimits`, a defaulted strict TOML table. Existing configuration remains valid. Limits affect admission, not semantics: they are excluded from the semantic config fingerprint, including at defaults. Changing limits does not invalidate immutable cached facts; reused facts must be charged again.
- Add `kortex_languages::resolve_bounded_with_context(snapshot, context, limits) -> Result<Vec<Edge>>` and `ResolutionLimits`. Production indexer uses this API. Existing `resolve` and `resolve_with_context` remain trusted, unbounded compatibility oracles; callers accepting untrusted repositories must not use them.
- Add `kortex_parse::parse_bounded` with per-file AST admission limits. Native parser allocation still occurs before AST admission; parse timeouts are cooperative, not allocator limits. Existing `parse` remains the compatibility API.
- A resource refusal is `kortex_core::Error` code `RESOURCE_LIMIT`, exit 9, with dimension/limit/attempted count. It aborts the entire attempt before publication and registration, never truncates candidates/facts, changes ambiguous calls to exact, or reports a false unresolved success.
- User config, explicit config and environment are trusted policy inputs, subject to hard validation ceilings. Repository `.kortex.toml` may only lower each inherited resource limit. The default limits below are also the validation ceilings in this implementation: raising a cap needs an explicit reviewed policy/code change, not untrusted repository input.

## Implemented Default Limits (Also Hard Ceilings)

| Dimension | Limit | Meaning |
| --- | ---: | --- |
| aggregate symbols | 500,000 | all parsed file symbols, including file nodes |
| aggregate references | 1,000,000 | immutable parsed reference sites |
| aggregate imports | 250,000 | parsed imports, plus transformed alias imports charged separately in resolution |
| aggregate parsed JSON bytes | 512 MiB | exact serialized ParsedFile bytes, including attributes, strings and diagnostics |
| emitted edges | 2,000,000 | pre-dedup edge emissions, including derived edges and overrides |
| candidate memberships | 2,000,000 | unique viable targets examined per reference/override, including singleton targets; summed across references |
| resolved edge JSON bytes | 512 MiB | exact serialized edge bytes, before dedup |
| per-file AST nodes | 250,000 | all named and unnamed nodes admitted before fact extraction |
| per-file AST depth | 192 | fail closed before recursive extraction/depth truncation |

These are explicit resource-policy ceilings, NOT an invented 2 GB acceptance threshold. The real reference fixture has 100,000 functions / 110,000 symbols / 320,000 edges, leaving 4.5x symbol and 6.25x edge-count headroom. Byte and reference headroom must be checked against actual fixture measurements by the parent; the limits do not prove a heap/RSS maximum or reference performance acceptance. Candidate memberships bound quadratic repeated candidate output independent of small source size: the parent's 80/80 and 160/160 real Go fixtures yielded 6,400 and 25,600 memberships respectively.

## Accounting design

Workers share aggregate admission counters. Parsed facts (including cached facts) are charged before retention in the collected result; cached facts are measured/charged before cloning. Serialization accounting uses a counting writer rather than allocating serialized byte vectors. A rejected worker aborts the attempt; no partial set is published. Already active workers may finish bounded per-file work; errors are not semantic truncation.

Resolution deduplicates borrowed candidate IDs before reserving memberships. It checks edge count before base-edge construction, then counts serialized bytes before cloning candidate IDs/retaining output. An empty-candidate base edge is temporarily constructed for accounting; other resolver temporary/index/cache memory is not proven bounded by this counter. All resolution paths (ordinary references, overrides, declarations, imports, exports, derived edges and TypeScript alias transformations) must use the bounded path or be listed as a remaining gap.

## Explicit limitations / acceptance still open

- Counts and JSON byte accounting are not allocator/RSS proof: Vec/BTreeMap capacities, lookup indexes, temporary borrowed candidate lists, serialization/SQLite buffers, source inventory, old snapshot loading and native Tree-sitter heap differ from wire bytes.
- Production now applies incremental per-file retained-output admission, including metadata enrichment, with a32MiB ceiling (lowered by aggregate configuration) and identical cached checks before cloning. Single newly constructed values and traversal/native/allocator temporaries are still not a hard allocation/RSS bound. See docs/PARSER_EMISSION_BOUNDS.md; this is not complete private-production resource acceptance.
- Discovery source bytes retain their existing repository limits; loading a previous committed Snapshot is still eager and outside the new worker/resolver accounting.
- Production acceptance, scale profiling and integrated private install/transport validation remain parent-owned. No performance threshold, freshness check, semantic assertion or fixture size is waived.

## Evidence

Worker726f43eb-0229-467 completed the scoped implementation and stopped for confirmation of changes to repository confined.rs. Those were expected PARENT scratch edits, not an unknown source modification. Parent confirmed ownership from its own patch and did NOT restart the stopped worker; final acceptance/evidence review stayed parent-owned.

Parent independently rebuilt the actual CLI and passed ALL-feature locked/offline whole-workspace tests with --include-ignored, including both explicit CLI proofs and the fresh six-language journey; all targets PASS/zero ignored. Strict workspace all-target/all-feature clippy and fmt PASS. Pinned target/tools/bin/cargo-deny ALL-feature locked/offline advisories/bans/licenses/sources PASS without waivers; an initial default-PATH cargo deny lookup failure is preserved separately, not counted as audit success. Logs: target/parent-query-resource-workspace-{tests,clippy,format,audit-pinned}.log and the initial audit.log. Resource regressions cover exact/refusal admission, cached facts, worker concurrency and unchanged prior DB bytes/generation/FTS/registry followed by private retry; they remain enabled.

Parent also ran two separate 10k-file/100k-function/110k-symbol/320k-edge diagnostics with CURRENT SDK/default resource limits: BOTH complete publication and every query response comparison pass. This proves the reference-shaped fixture is admitted by the implemented default count/byte guards, NOT a measured byte-margin or heap/RSS guarantee. Dataset BLAKE3 3d03be3f05a84b6a4963bf410c12ce055a8a5fac588e88d6815f12cfafa89bf3; source pin258files/SHA25607247946d8212c97127236819588aeb5fc7953bb45aee90ceed03c01683c38eb. See target/parent-query-resource-source.json and target/parent-query-scope-isolated-summary.json. Both SDK runs are the SAME current build, not before/after indexing measurements. No latency/quality threshold was waived.

Parser-only owner08341dda-ee52-4cc stopped after an expected parent manifest interruption and one invalid scale-fixture assumption. Parent recovered the handoff without restarting, corrected the actual-source fixture (UNCHANGED32MiB ceiling), independently passed12parser tests/clippy, and adopted the APIs for fresh/cached production facts. New real-source refusal/DB-preservation/restart/reuse tests PASS; subsequent freshly rebuilt fullworkspace --include-ignored/strictclippy/fmt PASS: target/parent-emission-wave-*.log. Details and temporary-allocation limits: docs/PARSER_EMISSION_BOUNDS.md. Native parser, legacy old-snapshot load, resolver temporary/index/cache memory and full adversarial resource/RSS proof remain open. No production-ready claim.
