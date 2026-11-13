# Meet Kortex: local code intelligence for people and AI agents

**Status: private development software, not a public release.** This article describes the current development checkout, not a production-readiness certification. Kortex is proprietary; the owner has not authorized public release or third-party use. See [LICENSE](../LICENSE) and [production readiness](PRODUCTION_READINESS.md).

An AI coding assistant can read a file and still miss the function that calls it, the test that exercises it, or a dependency several directories away. Ordinary text search remains essential, but it does not by itself describe how program entities relate to one another.

Kortex is a local-first code-intelligence engine that builds a persistent, source-derived knowledge graph and makes it available through a command-line interface and a read-only Model Context Protocol (MCP) server. It is written in Rust and is designed to help a developer or agent find evidence, trace relationships and assess the possible impact of a change.

It is an aid to investigation, not a compiler's complete model of a program. A missing graph edge is not proof that a dependency does not exist, and an indexed citation must be checked against the source, especially after changes.

## From source files to a useful graph

Kortex discovers repository files under configured limits, parses supported languages with Tree-sitter, extracts declarations and references, and attempts to resolve relationships across files. It stores source-derived facts and search indexes in SQLite. Published generations are transactional, so readers can work against a consistent committed snapshot while an update is prepared.

The default parser families are Rust, Python, JavaScript/TypeScript and Go. Additional provider implementations include Java, C#, C, C++, Ruby and PHP; language support and cross-file resolution vary by construct. Dynamic behavior, receiver and type inference, macros, module systems and other language features can leave relationships uncertain or unresolved. See [language support](LANGUAGE_SUPPORT.md) and the [implementation ledger](IMPLEMENTATION_PLAN.md).

Indexing is local and does not require an LLM, account, embedding service or execution of repository code. An index contains sensitive source-derived information and should be kept private. Existing limitations include an open default ignore-policy confinement issue documented in [production readiness](PRODUCTION_READINESS.md); do not treat the current build as production-safe for untrusted repositories.

## What can you ask?

The CLI and shared services provide several complementary views:

- **Query and context:** find symbols and inspect declarations, locations and indexed relationships.
- **Impact and trace:** explore bounded upstream or downstream dependencies and find a path between symbols.
- **Changes:** associate Git changes with indexed symbols and likely affected code, subject to explicit uncertainty.
- **Graph:** inspect a bounded neighborhood, rather than attempting to draw an entire large repository at once.
- **Routes and rules:** development-stage analysis can identify certain framework-evidenced routes and evaluate configured architecture rules; coverage is not exhaustive.

Results include evidence, freshness or generation information, and partial/truncation indicators where applicable. Consult the current executable's `--help` and actual response rather than assuming every relationship is supported.

## Working with an AI agent

Kortex exposes read-only MCP tools for repository selection, status, query, context, impact, trace, changes and graph exploration. An agent should check index freshness, locate the relevant symbols, examine context and impact before nontrivial edits, and then read the actual source and tests. After changes, it should run the project's checks and refresh the index when permitted. MCP does not execute code, edit files or build indexes.

`kortex init` in this development checkout creates default settings when absent, installs a versioned Kortex instruction section in the project's `AGENTS.md`, and builds a verified index if the selected configuration has no committed index. Existing project instructions outside that section are preserved. Older managed sections require interactive confirmation before replacement; this does not automatically configure an MCP client or install a Pi slash command. See [the agent guide](AI_AGENT_GUIDE.md) and [CLI documentation](CLI.md).

## The private web explorer

An optional, locally served web explorer can show indexed files, search results, symbol details, bounded graph neighborhoods and other analysis. The current development workflow requires an HTTP-enabled Kortex build, explicitly supplied local web assets, a bearer token and authorized repository IDs. These steps protect local source-derived data; they are not yet the one-button folder-selection experience requested for a future dashboard. See [the explorer guide](../web/README.md) and the [proposed one-button design](ONE_BUTTON_EXPLORER_DESIGN.md).

## Search and model policy

Default search uses SQLite full-text and structural evidence; it does not require a model download. An experimental SDK-only local embedding path exists, but automatic model activation and pretrained relevance are not accepted for release. Kortex does not silently upload source or enable remote embeddings. The graph and search results should not be marketed as complete, universally accurate, or proven to improve any particular model's performance.

## How Kortex compares

These tools overlap, but they solve different problems. The comparison below reflects maintainer documentation consulted on 2026-11-12, not an independent accuracy or performance benchmark. A language list or the presence of a graph does not establish equivalent coverage.

| | Kortex | GitNexus | Sourcegraph + SCIP | Joern | GitHub CodeQL |
|---|---|---|---|---|---|
| Primary focus | Local code intelligence and agent context | Agent-oriented repository knowledge graph | Precise code navigation across indexed code | Code-property-graph analysis, especially security research | Query-based code and security analysis |
| Main interfaces | CLI, read-only MCP; optional local HTTP/explorer | CLI, MCP, web explorer | Sourcegraph web/API and language-specific SCIP indexers | CLI and Scala-based graph-query DSL | CLI, query libraries and VS Code integration |
| Relationships | Symbols, references and bounded dependency/impact traversal; incomplete resolution is explicit | Documents calls, dependencies, impact, processes and graph search; construct support varies | Definition, reference and implementation navigation; SCIP itself is an indexing protocol | Code property graphs and programmable traversals; language frontend determines coverage | Language-specific databases and queries; analysis depends on supported extractor and queries |
| Agent integration | Read-only MCP; `init` installs project instructions; client setup is separate | MCP and agent integrations; `analyze` documents integration setup, with an index-only option | Navigation/index integration; the cited SCIP sources do not establish equivalent agent workflows | Primarily query-driven static analysis | Primarily security scanning and query development |
| Local operation | Local indexing, SQLite and offline default search; no model required | Local CLI indexing; optional provider-backed features | Sourcegraph consumes generated indexes; its documented precise-navigation workflow is an Enterprise feature | Local analysis with language-specific frontends and a JDK | CLI creates and analyzes databases; build/extraction requirements vary |
| Visual exploration | Optional local explorer for bounded graph neighborhoods; setup currently manual | Web graph explorer documented | Code navigation in Sourcegraph UI | Graph queries and visualizations; not the same agent dashboard | VS Code/code-scanning result interfaces, not the same agent dashboard |
| Licensing | Proprietary/private; no third-party use authorized | PolyForm Noncommercial 1.0.0, with commercial licensing available | Verify Sourcegraph product and each SCIP indexer's terms separately | Verify Joern and frontend terms for the intended use | Query repository is MIT; the separately licensed CLI/engine has different terms, including restrictions on closed-source analysis |

Sourcegraph precise navigation is not a standalone substitute for every Kortex graph operation; Joern and CodeQL are particularly relevant when programmable static/security analysis is the goal. The cited sources do not establish that any product finds every call or dependency. Compare tools on the same representative repositories and measure both missed and incorrect relationships before choosing one.

**Sources:** [GitNexus README and license](https://github.com/abhigyanpatwari/GitNexus), [GitNexus language inventories](https://github.com/abhigyanpatwari/GitNexus/blob/main/docs/languages/README.md), [SCIP](https://github.com/scip-code/scip), [Sourcegraph precise navigation](https://docs.sourcegraph.com/code_navigation/explanations/precise_code_navigation), [Joern](https://github.com/joernio/joern) and [frontends](https://docs.joern.io/frontends/), [CodeQL](https://github.com/github/codeql) and [GitHub CodeQL documentation](https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-code-scanning). The requested [MarkTechPost GitNexus article](https://www.marktechpost.com/2026/04/24/meet-gitnexus-an-open-source-mcp-native-knowledge-graph-engine-that-gives-claude-code-and-cursor-full-codebase-structural-awareness/) inspired the explanatory format; its broad completeness claims are not independently established here.

## A practical workflow

```sh
kortex init /path/to/project
kortex --repo /path/to/project status --json
kortex --repo /path/to/project query MySymbol --json
kortex --repo /path/to/project context src/example.rs:MySymbol --json
kortex --repo /path/to/project impact src/example.rs:MySymbol --json
```

Use the actual trusted Kortex executable available on the machine. Initialization builds a missing index and writes local index and registry data; it leaves an existing index unchanged. Run `kortex index /path/to/project --verify --json` to refresh it. Check freshness, warnings and incomplete results, and fall back to ordinary search and source inspection when Kortex is unavailable or inadequate. The project-specific policy still governs which tests and edits are authorized.

## Current boundaries

Kortex is not yet approved for public release or full production use. Correctness across all language constructs, hostile filesystem scenarios, reference-scale performance, full cross-platform behavior and other acceptance gates remain open. Historical benchmark and platform results are evidence only for the exact snapshots tested, not guarantees about this checkout. For current limits, see [production readiness](PRODUCTION_READINESS.md), [security](SECURITY.md), and [test results](TEST_RESULTS.md).
