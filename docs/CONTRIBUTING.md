# Contributing

Read AGENTS.md, KORTEX_SPECIFICATION.md, DECISIONS.md and IMPLEMENTATION_PLAN.md. Kortex is an independent implementation; do not copy restricted GitNexus source/tests/schemas/assets. License selection is pending owner approval; do not assume permission to publish a release under MIT or Apache-2.0.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo build --release --locked -p kortex-cli
```

Add executable regression assertions for every semantic/storage/security fix. Test positive, missing, ambiguous and failure outcomes, actual source coordinates, incremental/full equality, and JSON/protocol stdout purity. Avoid mock-success implementations. Keep imports and lexical scopes conservative: same-name matching across unrelated files is not evidence.

All transports must call shared analysis services. Parse facts are immutable and global resolution is separate. New dependencies/grammars require provenance/license documentation and lockfile review. Preserve offline defaults and disallow repository configuration from enabling network or execution. Benchmarks must record hardware/data and measured observations, not unverified performance goals.

Record commands/outcomes and remaining gaps in TEST_RESULTS.md. Do not mark a milestone verified from build success alone. Cross-platform CI, security audits, coverage, full labeled correctness datasets and public release gates are still pending.
