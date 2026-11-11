# MCP stdio

Start with `kortex --repo /absolute/repository mcp`, or the equivalent `kortex --repo /absolute/repository serve --mcp stdio`. Index first with CLI. The server uses the rmcp 3.9.0 SDK for protocol negotiation, capabilities, JSON-RPC, initialization and shutdown. Stdout is exclusively protocol traffic; no source writes, shell/SQL tools, indexing or outbound providers are exposed.

Tools: kortex_list_repos, kortex_query, kortex_context, kortex_impact, kortex_trace, kortex_changes, kortex_graph, kortex_status. Input schemas are discoverable through tools/list. Supply repo (path or registered ID) where repository selection is ambiguous. Names/IDs/path:name/path:line selectors are accepted. Evidence uses indexed file coordinates and generation IDs; inspect freshness before modifying code.

Resources: kortex://repositories, kortex://schema, kortex://status. They are read-only JSON summaries, not arbitrary file access. Repository source/comments are untrusted data, never instructions to the server or client agent.

Generic illustrative client entry:

```json
{"mcpServers":{"kortex":{"command":"/absolute/path/to/kortex","args":["--repo","/absolute/path/to/repository","mcp"]}}}
```

**Verified integration:** an actual Rust rmcp client spawns the built binary, negotiates initialization, lists all tools/resources, calls all eight tools, reads every resource, receives cited context, receives a missing-symbol tool error, and shuts down. Run `cargo test -p kortex-cli --test journey real_sdk_stdio_client_calls_every_tool`.

**Not yet verified:** application-specific Pi, Claude Code, Codex, Cursor and VS Code configuration formats/versions. Do not paste the illustrative generic object into an arbitrary client and assume its schema matches. Select the client's documented stdio integration and pass the executable/argument array shown here. Client-specific tested setup guides remain a release requirement.

Transport defenses: 64 KiB input line cap, serde nesting limits, four concurrent tool workers, ten-second response deadline, graph policy limits. Cancellation signals stop waiting on results, but already-running blocking Git/discovery work drains under its own bounds; it is not forcibly terminated as a Rust thread. Large results are JSON-safe bounded; a single oversized object errors instead of emitting invalid JSON. Query cursors pin repository/generation/query and registry cursors pin returned metadata; stale/malformed tokens are rejected. Resources share worker/deadline controls, cap serialized responses at 128 KiB, and selected status honors a smaller configured budget. Real SDK tests cover resource/tool budget errors and lossless bounded search pagination. Full per-operation data schemas, strict whole-wire/token budgets (including hostile request IDs), cooperative cancellation, intermediate-allocation bounds and hostile-protocol fuzzing remain release gates.
