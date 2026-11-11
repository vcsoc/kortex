# Privacy and data lifecycle

Default runtime functionality is local and offline. No account, telemetry, cloud service, model or embedding provider exists in the shipped implementation. Building dependencies and CI supply-chain auditing may require network; these are not indexing behavior.

`.kortex/index.db` contains repository root metadata, generations/Git HEAD, file hashes, parsed symbols, bounded signatures/snippets/comments, references, graph edges and FTS terms. The user registry stores repository IDs, roots and active generations. No source upload occurs. Search/context results can reveal secrets present in indexed snippets; callers must protect tool results just as they protect source.

`kortex repos list --json` identifies registered entries. `kortex repos remove ID` unregisters without deleting the index. `kortex repos remove ID --delete-index --yes` validates ownership and deletes the index DB/WAL/SHM only, not source/configuration. A writer lock file and empty `.kortex/` directory may remain intentionally. Removing from the registry does not prevent explicitly selecting an existing local index by path.

SQLite/WAL and backups may retain data until cleaned; this is not secure erasure. Do not commit `.kortex/`, registry files or sensitive query output. Repository-local config is independent of index data and must be preserved during rebuild/recovery.
