# Crew evidence: Git change analysis

## Scope and completion

Implementation worker completed the scoped changes; parent/integrated acceptance is separate. Read `AGENTS.md`, `docs/IMPLEMENTATION_PLAN.md`, and specification §§7.4/8. Only `crates/kortex-analysis/**` and this document were edited. No dependency/manifest/lockfile changes, delegation, commits, checkout/reset/clean/discard operations, or workspace-wide formatting.

Files: `src/lib.rs`, `src/changes.rs`, `src/changes_tests.rs`, `src/changes_safety_tests.rs` under `crates/kortex-analysis/`.

Pre-edit `gitnexus_impact` was attempted for `Service::changes`, `Service::execute`, and subsequently modified analysis/test helpers. Every attempt reported this project is unindexed. Fallback was direct source inspection: CLI/MCP invoke `Service::execute`, which invokes the unchanged-signature private `changes` method; graph impact still uses existing upstream traversal. No graph-derived call/process evidence was invented.

## Behavior implemented

- Old/new hunk starts and counts remain separate. Zero-count ranges describe gaps and never become adjacent changed lines.
- Mapping selects only a path-and-content-hash-verified indexed diff side, independently per file. Matching `git_head` alone is never proof that indexed content matches either side.
- Unmatched generations/files retain path/hunk evidence, `mapping_status: unavailable`, `partial: true`, warnings, and no inferred changed symbols. Indexed file IDs remain separately labeled evidence.
- No-overlap, empty-file, metadata-only rename, and zero-count-side cases use the indexed file symbol rather than claiming every declaration changed.
- Staged reads index blobs, unstaged reads index/worktree, base reads commit/worktree, and base/head reads two resolved commits. Unborn staged additions work. Added/deleted files and pure/edited renames are covered.
- Existing signatures and response keys remain; additive fields include `hunks`, old/new ranges, indexed sides, mapping evidence, resolved revisions and explicit side kinds. `line_ranges` now contains only verified indexed-side coordinates rather than the former mixed-side union.
- Change-analysis partial/warning/truncation evidence propagates to the public service envelope, including when the index is fresh for the worktree but represents neither requested commit.
- Literal pathspecs, NUL-delimited statuses, forced patch prefixes, Git C-quoted header decoding, and exact old/new patch header pairing prevent hostile paths or unrelated patch sections from selecting wrong coordinates.
- All Git reads retain existing bounded `kortex_repo::git` behavior. External diff and textconv remain disabled. Configured clean/process commands and required-filter flags are overridden during diff, so Git does not execute repository-configured filters. Encoding-attributed worktree coordinates and symlink replacements are refused rather than guessed.
- Affected symbols and likely tests come only from existing indexed graph traversal/kinds; no routes, processes, or test associations are synthesized.

## Final verification (local macOS)

Commands run after final source changes:

1. `rustfmt --edition 2024` on the four owned Rust files — passed.
2. `cargo test -p kortex-analysis` — **16 passed, 0 failed**; 0 doctests; test runtime 0.74s, build 1.15s.
3. `cargo clippy -p kortex-analysis --all-targets -- -D warnings` — **passed**, 0.32s.
4. `cargo test -p kortex-cli --test journey complete_cli_core_journey` — **1 passed, 0 failed, 4 filtered**; test runtime 1.53s, build 1.78s. This exercises the actual indexer/parser/CLI consumer without editing peer-owned files.
5. `rustfmt --edition 2024 --check` on the four owned Rust files — **passed**.

The 16 analysis tests use actual temporary Git repositories. Coverage includes insertion before declarations on both indexed sides; pure deletion without adjacent-symbol attribution; pure/edited staged rename; added/deleted/empty files; unborn branch; same-HEAD mismatched indexed content; fresh worktree index versus an unrepresented commit pair; staged/index/worktree separation; commit-pair and base/worktree modes; invalid modes and bounds; public truncation propagation; leading-option/pathspec-magic/bracket/space/tab/newline/quote/Unicode/backslash filenames; external/textconv refusal; clean/process-filter execution markers remaining absent; symlink replacement; and explicit encoding uncertainty. Single-line fixture symbol spans are defined exactly from fixture text, without fabricated graph links; the separate CLI journey verifies real parser/graph integration.

Intermediate regressions caught and fixed before the final passing run: Git appends a tab to unquoted space-containing patch headers; temporary macOS repository roots require canonicalization when constructing a freshness fixture.

## Remaining gaps / parent acceptance

- Optional untracked-file support is not implemented; no shared helper/API extension was required.
- Whole-workspace fmt/tests/clippy, MCP protocol acceptance, Linux/Windows behavior, coverage/fuzzing and performance remain parent gates; this worker ran only the focused commands above.
- Repository/index/worktree reads are bounded but not a transactional Git snapshot. Hostile concurrent replacement/ABA mutation across separate Git and filesystem reads needs broader shared-repository hardening; no atomic-snapshot guarantee is claimed.
- Encoding-attributed worktree sides are deliberately conservative (even explicitly UTF-8-attributed files): old-side content-verified mappings can still be returned, otherwise mapping is partial. Custom clean/process conversions are intentionally disabled, so analysis does not execute or emulate their semantics.
- Gap-only changes fall back to a file symbol rather than guessing a containing or adjacent declaration. Deleted/added declarations not present in the chosen indexed side cannot gain fabricated symbol IDs.
- Route/process mapping remains absent when those graph facts are absent. Test recommendations require actual existing graph reachability and test symbol classification.
