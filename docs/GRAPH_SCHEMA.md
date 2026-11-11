# Graph and JSON schema v1.0 (development)

Symbols expose `symbol_id` (logical identity), `symbol_revision_id` (content revision), repo, kind, language, name/qualified name, relative path, signature, visibility, attributes, hash, optional doc comment and lexical owner. All public paths use `/`. Line/columns are 1-based UTF-8 byte coordinates, not UTF-16 editor offsets. Byte spans are zero-based half-open Tree-sitter offsets.

Repository IDs hash canonical local root with a domain tag. Symbol IDs hash delimited repo/language/path/kind/qualified-name/ordinal fields. Moving or renaming can change identity; continuity aliases are not yet implemented. Generation IDs are unique publication identifiers, not content hashes.

Implemented edge kinds: DECLARES, IMPORTS, CALLS, REFERENCES, USES_TYPE. Edges include source/optional target, source span, resolution, confidence, resolver version, evidence, and explicit candidate IDs. Ambiguous and unresolved edges have no guessed target. Traversal excludes unresolved/candidate edges; ambiguous counts are informational. Confidence is a heuristic and paths aggregate minimum edge confidence.

Responses from analysis services contain `schema_version`, `request_id`, `repo_id`, `generation_id`, `index_status`, `partial`, `truncated`, `warnings`, `data`, `page.next_cursor`, `timing_ms`. Repository listing/index commands have specialized envelopes. Errors use `{schema_version:"1.0",error:{code,message,exit_code}}`. MCP wraps these in text and structuredContent according to the SDK protocol.

Runtime MCP tool input schemas are returned by tools/list and `kortex_mcp::schema`. Output schemas currently constrain only the envelope; complete operation-specific stable output schemas remain a release gate. CLI/shared-Service query cursors remain numeric offsets, so reindexing between pages can change results. MCP query cursors pin repository/generation/query; registry cursors pin returned metadata and reject stale tokens. Graph exports are bounded; do not interpret absent truncated relationships as proof of no dependencies.

Storage schema version 1 lives in kortex-storage and includes schema_migrations, repositories, generations, files, nodes, edges, aliases (reserved), index_state and generation-filtered FTS5. Newer schema versions are refused rather than overwritten. Existing v1 indexes are disposable and rebuildable; configuration is separate and must not be deleted for recovery.
