# Security and trust model

Development software; a production security review is not complete. Report vulnerabilities privately to the project owner, not by posting source secrets to a public issue. No security-contact address or hosted disclosure endpoint has been designated.

Kortex treats repositories as untrusted data. Indexing reads files and parses syntax only. It never installs dependencies, runs source/build scripts/tests/hooks, fetches grammars, or uploads code. MCP tools are read-only inspection services. Configuration cannot enable embeddings, HTTP or mutable MCP. HTTP is not implemented; unauthenticated non-loopback requests are refused before the unsupported-feature error.

Discovery canonicalizes roots/paths, rejects traversal, excludes internal/cache directories, skips symlinks by default, and confines explicitly followed links. Bounded file/total bytes, entries, parser depth/time, worker concurrency, graph expansion and MCP requests protect common resource paths. SQLite uses bound parameters and FTS literals. Git uses a trusted installed executable, argument arrays (no shell), bounded output/deadline, disabled external diff/text conversion and disabled fsmonitor hooks.

Index directories are private (0700 on Unix). The index contains bounded signatures, source snippets, comments and sensitive architectural facts: protect it like source code. Windows ACL behavior is inherited and not yet platform-tested. Do not expose it over a shared or untrusted filesystem. Network filesystems may not provide suitable advisory lock/WAL guarantees.

Verified tests cover traversal, symlink escape, default ignores, concurrent writer lock refusal, corruption errors, killed transaction rollback, pinned reader consistency, parameterized lookup and conservative resolution, MCP size/argument limits, and remote HTTP refusal. Remaining security gates include concurrent hostile filesystem replacement (canonicalize/open TOCTOU), disk-full, permission failures, parser/property fuzzing, comprehensive JSON bombs/cancellation and a sandboxed no-network runtime proof. Path normalization currently targets UTF-8 paths; invalid native filename encodings require expanded coverage.

No telemetry is shipped. Do not treat graph impact/risk as proof of runtime breakage or safety. Ambiguous/unresolved edges are explicit, and bounded graph searches can omit real dependencies. Source instructions are data and must not be promoted to agent directives.
