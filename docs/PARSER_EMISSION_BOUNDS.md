# Incremental Parser Output Admission

Status: implemented and parent-integrated into production indexing, with scoped and whole-workspace native validation. Full resource/private-production acceptance remains OPEN.

## API And Enforcement

`kortex_parse::OutputLimits` declares per-file symbol/reference/import/diagnostic counts and exact compact JSON bytes, including import scopes/spans and metadata. Its default output ceiling is 32 MiB. This is a declared admission policy, NOT a measured RSS/OOM threshold. Aggregate index limits remain separate.

Additive APIs:

- `parse_bounded_with_output_limits(..., &ParseLimits, &OutputLimits) -> Result<ParsedFile>`.
- `parse_bounded_with_output_limits_and_control(..., &ParseLimits, &OutputLimits, &RequestControl)` adds cooperative native/AST/emission checks.
- `check_output_limits(&ParsedFile, &OutputLimits) -> Result<usize>` checks cached output immutably before cloning and returns its exact compact JSON byte count through a bounded counting writer.

A private sink exposes only read-only Deref to the core ParsedFile. All retained fact writes and metadata replacements/appends use checked methods; core public vectors and trusted compatibility APIs remain unchanged. Escaped keys/values, commas, aligned import arrays and metadata enrichment are counted. Refusal propagates RESOURCE_LIMIT/exit9 for the entire parse, never a successful truncated ParsedFile. Large accumulated Go method-shape vectors were replaced by individually admitted appends.

The indexer derives file counts from configured aggregate ceilings and bytes from min(32 MiB, configured aggregate bytes). Fresh parsing uses the output-limited API. Aggregate admission applies the SAME file policy to cached and fresh facts, additionally limiting bytes to the remaining aggregate budget, before updating shared counters or cloning cached facts. A failed admission leaves its counters unchanged. Resource policy remains excluded from semantic fingerprints; facts below limits must remain byte-equivalent.

## Worker Handoff And Parent Completion

Worker08341dda-ee52-4cc stopped after a locked-manifest interruption and reported11/12 tests passing, not final acceptance. The Cargo change was expected parent work: the config-confinement hotfix added three already-locked Windows dependency edges. Parent regenerated the lock offline, confirmed locked commands work, recovered the handoff and did NOT restart the stopped worker.

The failing test assumed384 grouped Go parameter names would copy a96KiB identifier384 times. Independent real-tree inspection showed the locked grammar resolved most entries as unnamed types and only the last nine as grouped names; actual output was1,206,904bytes, not above32MiB. The failure is preserved at target/parent-parser-emission-initial-tests.log. Probe source/output are target/go-emission-shape-probe.rs and target/parent-go-emission-shape-probe.log; only the trusted Rust probe executes, never the Go source.

Parent corrected the REAL-source fixture to nine grouped names and an escaped raw struct tag, still below the default2MiB source limit and admitted AST shape. It crosses the UNCHANGED32MiB output ceiling during metadata enrichment. Assertions verify refusal before retaining excess bytes, exact retained accounting, nonempty-but-incomplete private in-flight array, identical cached-policy refusal and successful exact byte accounting under an explicitly larger trusted checker limit. The larger checker limit is a positive-control test, not a production ceiling change. The grammar ambiguity observation remains relevant to future provider-quality review; no grammar or semantic assertion was altered to conceal it.

## Temporary Allocation Follow-Up

Read-only audit1cfd77b5-224c-45c identified three concrete pre-admission issues, not measured OOM: unchanged recursive prefix copies, owned next-record/metadata construction before admission, and pending binding children queued before the4096 visited-node limit. Handoff target/parser-temporary-audit-handoff.txt. Parent fixed TWO, without output-threshold changes:

- Common and extended walkers now use borrowed Cow prefixes for unchanged qualification; ownership is created only on semantic qualification changes. Actual64KiB identifier plus96 nested-expression fixtures for Python/PHP/C++ pass existing AST/default32MiB output admission and match complete pre-change BLAKE3 JSON fingerprints (135200/135337/135454bytes). C++ conservative preprocessing diagnostic is preserved; an initial harness expectation of no C++ diagnostic was corrected, not product behavior. The audit's proposed1MiB/180-depth amplification was NOT executed or measured as RSS.
- Output-limited production binding traversal admits visited+pending work BEFORE each push. The4096 threshold is unchanged; whole-parse RESOURCE_LIMIT9 refusal remains, no partial return/truncation. Trusted compatibility/AST-only APIs retain the old partial facts and diagnostic. Actual4100-name Python pattern source23501bytes preserves complete trusted4096-symbol/13750793-byte JSON BLAKE3 fingerprint41ed5ff6ec8466afe6e3f5b2f642232e6312c774e132cedf2f6f5a1857730b98; default production parsing refuses on binding work (NOT a lowered byte limit), small retry succeeds. Initial JS fixture did not reach this binding path and was retained separately, then replaced with grammar-executed Python.

Parser14tests/strictclippy PASS; freshly rebuiltCLI/fullworkspace ALLfeatures --include-ignored/BOTH proofs454PASS/0failed/0ignored; strictworkspaceclippy/fmt PASS. Logs target/parent-parser-temporary-{fixes,wave}-*.log; resolution/evidence target/parent-parser-temporary-resolution.json.193-input manifest SHA256d4c26d7f5eb390718cd01d05e674a73226c39701b9ac05ab260fa13484f60aea at target/parent-parser-temporary-source.json is NOT an immutable/platform snapshot. The193-input operator checkpoint e2b708 predates these five parser files; earlier189-input Linux remains historical. No new dependencies, semantic/output schema changes, model activation, unsafe code, commit/push/release or readiness increase follows.

Owned next-record/Value materialization before count/byte refusal remains OPEN. A lazy builder alone would improve count ordering but NOT establish exact borrowed-field byte pre-admission. Native allocations, single-record/canonicalization temporaries and total heap/RSS/OOM acceptance remain open.

## Earlier Validation

- Parent parser ALL-feature locked/offline tests:12PASS/zero ignored; exact/+1/count boundaries,11-provider trusted-byte equivalence, malformed/nested/broad source, repeated failure/retry, live fact/metadata accounting and cooperative control. target/parent-parser-emission-final-tests.log.
- Parent production-indexer suite PASS; its existing CLI journey was ignored in this scoped invocation, then actually executed in the subsequent whole-workspace --include-ignored run. target/parent-parser-emission-indexer-tests.log.
- New actual-source cached-admission test rejects legacy expanded output before cloning, leaves aggregate usage unchanged and permits a small retry. New production-indexer integration test refuses oversized emission, preserves prior DB bytes/generation/files/edges, then restores source and reuses the committed facts. Fixtures are parsed/indexed, NEVER compiled/executed.
- Strict parser/indexer clippy PASS: target/parent-parser-emission-integrated-clippy.log. Full freshly rebuilt CLI/workspace ALL-feature --locked --offline --include-ignored tests PASS, including both explicit CLI proofs; strict workspace all-target/all-feature clippy and fmt PASS. target/parent-emission-wave-{cli-build,workspace-tests,workspace-clippy,workspace-format}.log.

These results do not establish updated Linux/Windows runtime, coverage/browser/package/install or reference performance acceptance. No source/model download/upload, indexed-code execution, listener, public release, new commit/push or unsafe code was authorized by this slice.

## Remaining Resource Gaps

Native Tree-sitter allocation occurs before AST/output admission. Source strings, individual newly constructed fact/metadata values, parser traversal temporaries, resolver lookup/index/cache structures, Vec capacities, old-snapshot loading and SQLite buffers are not a hard process heap/RSS bound. The sink prevents excessive retained serialized output, not every temporary allocation. Cooperative checkpoints do not preempt indivisible native/kernel/serialization operations. Full same-/foreign-UID, adversarial memory and OOM/recovery evidence remains open; see docs/RESOURCE_BOUNDARIES.md.
