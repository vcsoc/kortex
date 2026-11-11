# Semantic graph crew evidence

## Production wave: integration handoff (not full P0/P1 acceptance)

**Owner policy: CLOSED SOURCE / PRIVATE. No publishing, source uploads, public release or permission for third-party use.** All three owned first-party manifests retain `publish.workspace = true` (workspace value false) and now add `license-file.workspace = true` pointing to the parent-owned restrictive LICENSE. MIT statements below apply ONLY to third-party grammars.

### Cross-owner contracts / immediate integration needs

1. **Repository owner: add discovery mappings** java→java, cs→csharp, c/h→c, cc/cpp/cxx/hpp/hh/hxx→cpp, rb/rake/gemspec→ruby, php/phtml→php. Actual parser provider IDs are `java`, `csharp`, `c`, `cpp`, `ruby`, `php`. Discovery still lacks these mappings at this handoff. No peer-owned repository source was edited. P1 fixtures currently verify real parse → resolve → SQLite graph/FTS persistence, but do not claim discovery/CLI acceptance.
2. **Parent: bump `PARSER_VERSION` beyond version 6** before integration acceptance. Parsed facts, Go method qualified identities, CommonJS metadata and providers changed. No core struct/schema/parser constant was edited. Existing `Symbol.attributes` and `Reference.kind` suffice.
3. **Application/transport owners:** `index(path, config, options)` selects Git root, then applies `config.resolved_root` ONCE and configured storage. New `index_at_root(effective_root, config, options)` deliberately does NOT expand a selected subdirectory to Git ancestor or apply relocation again. Call this helper when Service/CLI has already selected the effective root. Both use `Store::open_with_storage`; dry-run checks the configured cache, not `.kortex` unconditionally.
4. Public `resolve(snapshot, go_module)` remains compatible. Additive `ResolutionContext` / `resolve_with_context` accept explicit Go module/root tsconfig inputs without filesystem access. Indexer uses confined reads. Every generation globally rebuilds resolution from immutable facts: explicit **safe full-resolution fallback**, not an unproven selective invalidation optimization. Resolution-input fingerprints are rechecked before commit, including when inventory verification is off.
5. Cargo updated the lockfile as tooling; no manual root manifest/lockfile edits. One `--locked` run encountered concurrent peer dependency changes; `cargo test ... --offline` reconciled the lock (including peer Candle/tokenizer dependencies), then scoped strict clippy passed. Parent owns final lockfile/advisory/SBOM reconciliation.

### Implemented vertical slices

- CommonJS namespace/destructured/renamed `require` imports; named/object/default exports; direct function/arrow export assignments. Locally shadowed `require`/`module`/`exports`, missing/private exports and unknown receivers do not receive guessed exact targets. ESM behavior preserved.
- Root tsconfig/jsconfig `baseUrl` and one-wildcard/exact `paths`, JSONC comments/trailing commas, bounded target expansion and root-confined normalization. Multiple matching physical targets remain candidates, not guessed winners. Changing aliases reuses unchanged parsed files but globally re-resolves; incremental/full graphs match. Inherited `extends` is explicitly refused with a diagnostic rather than guessed inherited baseUrl semantics.
- Rust file/inline modules, basic glob imports and bounded public reexport traversal; explicit concrete receiver annotations and generic/dyn trait implementation candidates. Macro calls are omitted rather than confused with same-name functions; cfg declarations/module descendants are inferred, not unconditional exact. Private types and instance fields are still not falsely resolved as public/associated items.
- Go method receiver-qualified identities, explicit pointer/value annotations, cross-file same-package methods and interface candidate sets. Known incompatible method signatures/missing interface methods are rejected. Even a singleton interface candidate retains no exact target.
- Actual `CONTAINS`, `EXPORTS`, `DEPENDS_ON`, `READS`, `WRITES`, `INHERITS`, `IMPLEMENTS`, explicit-marker `OVERRIDES`, alongside existing DECLARES/IMPORTS/REFERENCES/CALLS/USES_TYPE. Interface bases are classified after resolving the declared type. No fabricated route/tool/process/test edges; those require peer evidence.
- Six independently integrated P1 Tree-sitter grammars, with `LanguageProvider` IDs/extensions and real declaration/span/modifier/signature/owner/type/value/call facts. Java packages/imports/static calls and overload sets; C# namespaces/using/file-scoped namespaces/partial class overload sets; C/C++ local includes/namespaces/function declarations/definitions; Ruby modules/classes/singleton methods/require_relative; PHP namespaces/use aliases/static methods/functions, including multiple semicolon namespaces. Dynamic receivers remain unresolved. C/C++ macro names block same-name guesses and conditional declarations remain inferred; no preprocessing/build/compilation-database execution.
- Configured effective-root and shared-cache publication/reuse/dry-run integration, including an isolated child-process cache environment (no unsafe process-global environment mutations).

### Dependency provenance / licenses

Exact grammar pins, no optional features and no runtime grammar downloads:

| Crate | Version | Reviewed license |
|---|---|---|
| tree-sitter-java | 0.23.5 | MIT, Ayman Nadeem 2017 |
| tree-sitter-c-sharp | 0.23.5 | MIT, Brunsfeld/Guard/Qureshi/contributors 2014–2023 |
| tree-sitter-c | 0.24.2 | MIT, Max Brunsfeld 2014 |
| tree-sitter-cpp | 0.23.4 | MIT, Max Brunsfeld 2014 |
| tree-sitter-ruby | 0.23.1 | MIT, Rob Rix 2016 |
| tree-sitter-php | 0.25.1 | MIT, Josh Vera/GitHub 2017; Brunsfeld/Qureshi/Frøystad/White 2019 |

Reviewed crates.io metadata for all six and complete LICENSE texts. C#/C/PHP license files are in downloaded crate archives. Java/C++/Ruby archives omit LICENSE files; their complete MIT notices were checked at the official tree-sitter repositories' exact `v<version>/LICENSE` tags (not GitNexus). Parent must include required third-party notices in private artifacts. `cargo deny check advisories licenses sources` was attempted but is unavailable in this worker environment (`no such command: deny`); this is NOT an advisory-pass claim. Parent's current audit remains required.

### Exact verification (local macOS, worker checks)

Latest full owned-crate run (including JSONC inheritance/traversal negatives):

```text
cargo test -p kortex-parse -p kortex-languages -p kortex-indexer --offline
```

**49 passed** (4 indexer unit, 11 P1 provider/persistence, 12 P0 completion/cache/root entries including one guarded child entrypoint, 3 existing resolution, 7 existing graph, 8 existing scope, 2 languages, 2 parse). The parent cache test actually invokes the guarded child with a private isolated cache directory. All doc tests pass. Initial peer config/repo compilation/lint overlap failures were retried after stabilization and are not final results.

```text
cargo clippy -p kortex-parse -p kortex-languages -p kortex-indexer --all-targets --offline -- -D warnings
cargo fmt -p kortex-parse -p kortex-languages -p kortex-indexer -- --check
```

Both **PASS** after the last full test run. The new test module was moved below production items after clippy flagged its placement, then both clippy/fmt and `cargo test -p kortex-languages --offline` passed again. Latest `--locked` retry was prevented by concurrent peer manifest/lock changes; `--offline` passed without network. Metadata verification also confirmed `publish=[]`, `license=null`, `license_file=../../LICENSE` for all three owned packages. No `cargo fmt --all`. GitNexus impact attempted before existing function/method edits; repository unindexed throughout. Direct trace/review and persisted fixtures are fallback evidence, not graph-tool analysis.

New P1 cases explicitly assert: one known exact target per provider; missing names remain unresolved despite unrelated same-name declarations; two overload/conflicting/redefinition candidates with no target; malformed syntax diagnostics for EACH grammar; actual source span/name containment; unknown Java/Ruby/PHP receiver calls unresolved; namespace owner preservation; explicit bases/overrides; C/C++ macro/cfg uncertainty; immutable serialized facts re-resolve deterministically; SQLite graph/files/FTS roundtrip. C has conflicting declarations (not invented C overload support); Ruby/PHP ambiguity fixtures are redefinitions/conflicting declarations, not a claim those languages support Java-style overloads.

### Remaining criteria / do not claim complete

- P1 discovery mappings and full discovery→index→CLI/Service verification; P1 edit/rename/delete incremental/full equivalence through the actual index orchestrator after discovery integration. Current P1 persistence tests do not substitute for those gates.
- Broad labeled precision/recall datasets, malformed-input property/fuzz campaigns, parser/fact extraction resource stress, 10k-file/100k-symbol p95/RSS, cross-platform and refreshed coverage. No new percentage/performance claim.
- Root-only tsconfig aliases: inheritance/project references/package exports/node_modules/package-manager semantics are not resolved. Multiple target fallbacks conservatively overapproximate rather than emulate every compiler precedence rule.
- Rust workspace crate-root/Cargo feature graphs, complex `#[path]`/restricted visibility, cfg evaluation, trait defaults/associated types/generic solving remain conservative/incomplete. Public reexports are bounded to 32 hops; dynamic type propagation is not claimed.
- Go interface constraints/embedded generic type sets/alias-equivalence and full pointer method-set semantics are not a compiler. Candidates are uncertain, not proof of runtime dispatch.
- P1 overload argument-type selection, virtual dispatch/receiver propagation, full compiler visibility/generic scopes, C/C++ preprocessing/linker/build flags and function-pointer inference, Ruby load paths/metaprogramming, PHP Composer/autoload/grouped use and dynamic dispatch are not complete. Unsupported syntax may lose recall; no generic regex stand-in was substituted.
- Full graph domain node completeness/identity continuity and peer-owned route/process/intelligence contracts remain integrated acceptance work. No release or permission for third-party use is authorized.

### Owned files this wave

`kortex-parse/{Cargo.toml,src/lib.rs,src/semantic.rs,src/p0.rs,src/extended.rs}`; `kortex-languages/{Cargo.toml,src/lib.rs,src/context.rs,src/receivers.rs,src/rust_modules.rs,src/extended.rs,src/relationships.rs}`; `kortex-indexer/{Cargo.toml,src/lib.rs,tests/p0_completion.rs,tests/extended_providers.rs}`; this handoff. No delegation, commits, resets, cleanup, worktrees, publishing or indexed-code execution.

## Historical previous-wave runtime / ownership

Production changes are limited to:

- `crates/kortex-parse/src/lib.rs`
- `crates/kortex-parse/src/semantic.rs` (new)
- `crates/kortex-languages/src/lib.rs`
- `crates/kortex-indexer/tests/semantic_graph_crew.rs` (new, 7 tests)
- `crates/kortex-indexer/tests/semantic_scope_crew.rs` (new, 8 tests)
- this evidence document.

`crates/kortex-indexer/tests/resolution.rs` was preserved unchanged and rerun. No dependencies, manifests, core contracts, parser-version constant, indexer production code, peer-owned files, commits, resets, or cleanup operations were changed by this worker. No delegation was launched.

Read `AGENTS.md`, `docs/IMPLEMENTATION_PLAN.md`, and specification §§5–6. GitNexus impact was attempted for `parse`, `walk`, `record_binding`, `declaration`, `resolve`, and `module_candidates`; each returned that the repository is not indexed. Direct fallback tracing covered `index -> parse -> walk -> semantic facts -> resolve -> Store::commit`, with real persisted-graph fixtures below. The untracked checkout has no baseline diff to use as a substitute for that review.

## Implemented behavior

- Actual AST identifier and qualified-property/path sites now produce `REFERENCES`; named annotations, returns, type arguments and supported bounds produce `USES_TYPE`.
- Declaration names, imports/package names, explicit global/nonlocal declarations, property keys and duplicate callee identifiers are excluded. Function values are references rather than calls. TypeScript `typeof` uses the value namespace.
- Named Rust/Go fields, JS/TS class/interface properties, Python class assignments, locals/parameters, common destructuring bindings and Go constants become real searchable declarations. Go struct/interface type specs retain their actual kind.
- Additive `Symbol.attributes` retain binding/name/scope ranges, generic blockers, mutation uncertainty, and Rust runtime-receiver syntax. Public/explicit private/protected/restricted visibility is retained where supported by the AST. Existing serialized structs are unchanged.
- Resolution filters candidates by `reference.kind`, including separate type/value namespaces. Local parameter/block bindings shadow imports and unrelated names; JS `var` is function-scoped. Python future assignments block fallback to globals/imports, and repeated assignments do not duplicate declarations. Generic parameters do not accidentally bind to unrelated concrete types.
- Rust cross-file private declarations are not treated as public; named ESM exports and Go export rules remain enforced. Rust runtime `receiver.member` syntax is not confused with associated `Type::member` syntax. Unknown runtime receivers/callable assignments remain unresolved.
- Exact, ambiguous (candidate IDs but no guessed target), unresolved (no target), and existing syntax-error/limit inference behavior remain explicit with source spans and evidence.

## Exact local verification

Final run, after the reassignment guard and runtime-receiver regression were added:

```text
cargo test -p kortex-parse -p kortex-languages -p kortex-indexer --locked
```

PASS: **25 tests**, including 15 new integration tests; zero failures. Existing call-resolution, reader-generation, rename/delete and incremental/full-equality regressions all pass. New coverage includes Rust/Python/TS/JS/Go positive local facts; cross-file type/value imports; aliases and namespace properties; function values; wrong-kind/private/missing targets; ambiguity; generic/type-value separation; block/parameter/closure/var scope; future Python binding; destructuring defaults; explicit global uncertainty; reassignment; static-vs-runtime Rust syntax; and cached semantic facts followed by edit/rename/delete versus full rebuild.

```text
cargo clippy -p kortex-parse -p kortex-languages -p kortex-indexer --all-targets --locked -- -D warnings
```

PASS, including dependency checks. The earlier `semantic.rs:226` collapsible-if lint was fixed. Final clippy completed successfully in 0.62 seconds after the final test run.

```text
rustfmt --edition 2024 --check crates/kortex-parse/src/lib.rs crates/kortex-parse/src/semantic.rs crates/kortex-languages/src/lib.rs crates/kortex-indexer/tests/resolution.rs crates/kortex-indexer/tests/semantic_graph_crew.rs crates/kortex-indexer/tests/semantic_scope_crew.rs
```

PASS. Only owned files were formatted, never `cargo fmt --all` by this worker.

These are focused worker checks, not independent full-workspace acceptance. Parent reported passing integrated workspace tests/fmt/clippy before the final small reassignment guard; parent should rerun final integrated checks against the completed snapshot.

## Required parent action

**Bump `kortex_core::PARSER_VERSION` from `tree-sitter-0.25-kortex-5` to a fresh value (for example `tree-sitter-0.25-kortex-6`) before integration acceptance.** Cached parsed facts now include new references/declarations and different scope metadata. Without invalidating old facts, an unchanged indexed repository can retain the old incomplete semantic graph. The worker deliberately did not edit the parent-owned constant.

## Residual gaps / not claimed

- AST lexical facts are not compiler resolution. There is no receiver/type propagation, dynamic dispatch, inferred instance-field ownership, trait candidate solving, full control-flow/definite-assignment analysis, or complete language scope semantics.
- Rich match/if-let/pattern/comprehension/loop forms and all declaration varieties are not complete. Computed/dynamic member expressions and decorators/macros can be omitted rather than receiving a fabricated dependency; READS/WRITES and inheritance/implementation edges are not added here.
- Generic parameter targets and Python global/nonlocal bindings are explicitly unresolved. Callable locals/assignments are conservative even when a compiler could infer them.
- Rust restricted visibility (`pub(crate)`, `pub(super)`, etc.) is recorded as restricted but cross-file resolution currently only accepts clearly public declarations; this intentionally loses recall. Rust glob/reexports/inline module subtleties, CommonJS/tsconfig aliases, Python star imports and richer package layouts remain outside this slice.
- No broad precision/recall benchmark, large-repository performance/coverage claim, platform CI, or release acceptance was performed by this worker. Parent owns integrated acceptance, coverage, shared documentation and artifacts.
