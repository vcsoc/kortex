# Kortex

**Understand the code. Trace the connections. Change with confidence.**

Kortex is an independently implemented, local-first Rust code intelligence engine: a native CLI, embeddable workspace libraries, persistent SQLite knowledge graph and read-only MCP stdio server. Indexing requires no account, LLM, embedding service, package install or source execution.

## Development status

Version 0.1.0 is functional development software, **not a completed P0 production release**. The authoritative requirements are in [KORTEX_SPECIFICATION.md](KORTEX_SPECIFICATION.md). See [implementation and remaining gates](docs/IMPLEMENTATION_PLAN.md) and [executed tests](docs/TEST_RESULTS.md). No affiliation with GitNexus and no restricted implementation code copied.

Implemented and locally tested:

- Real Tree-sitter Rust, Python, TypeScript/JavaScript and Go parsers, source spans, lexical/import facts, richer declarations, conservative calls, identifier references and named type uses.
- SQLite WAL/FTS5 with atomic committed generations, advisory writer locks, read-only pinned readers and tested process-crash rollback.
- Exact/prefix/path/keyword search, cited context, bounded graph export, upstream/downstream impact, shortest dependency trace and Git hunk mapping.
- Cached-fact incremental updates and native file events with debounce/reconciliation for edits, atomic saves, renames and deletions.
- Eight MCP tools and three resources, generation-pinned MCP cursors and response budgets, verified with an actual rmcp SDK client.
- 97 passing local tests; measured line coverage of 90.92% overall and 92.30% for repository/storage sources.
- Strict TOML/environment policy, confined discovery, offline defaults, automated regression tests and gated release-preparation scripts.

Not complete: comprehensive references/type/trait/receiver inference, all language/module constructs, large-scale quality/performance gates, platform release validation and security/provenance approval. Extended languages, embeddings, HTTP/explorer, route mapping and architecture rules are not implemented. Unsupported behavior is not represented by synthetic success.

## Build and use

Prerequisites: Rust 1.99.1, native C compiler/build tools, trusted Git on PATH. Dependencies may need network during the first build; indexing is offline.

```sh
cargo build --release --locked -p kortex-cli
./target/release/kortex --help
./target/release/kortex index /path/to/repository --verify --json
./target/release/kortex --repo /path/to/repository query check --json
./target/release/kortex --repo /path/to/repository context src/auth.py:check --json
./target/release/kortex --repo /path/to/repository impact src/auth.py:check --depth 3 --json
./target/release/kortex --repo /path/to/repository mcp
```

Windows binary: `target\release\kortex.exe`. Only local macOS has been exercised; Linux and Windows CI configuration is provided but not claimed passed. See [Getting started](docs/GETTING_STARTED.md), [CLI](docs/CLI.md), [MCP](docs/MCP.md), [configuration](docs/CONFIGURATION.md), [architecture](docs/ARCHITECTURE.md) and [language support](docs/LANGUAGE_SUPPORT.md).

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
python3 benches/smoke.py
python3 packaging/sbom.py
```

The benchmark is deliberately a small smoke workload, not proof of reference-scale performance. [Security](docs/SECURITY.md), [privacy](docs/PRIVACY.md), [recovery](docs/TROUBLESHOOTING.md) and [dependency provenance](docs/DEPENDENCIES.md) document current safeguards and outstanding acceptance gates.

## License and releases

Kortex is **proprietary, closed source, and private**. No permission is granted to anyone else for any use until the project owner explicitly changes that policy. See [LICENSE](LICENSE) and [private policy](LICENSE_PENDING.md). Workspace crates are `publish = false`; public packaging is unconditionally refused, including when legacy approval variables are set. Private validation archives grant no use or distribution rights. No source, artifact, package, website, or release has been published or pushed. Hosted workflows require a private repository; native artifact validation also requires separate owner authorization. Third-party materials retain their own licenses.
