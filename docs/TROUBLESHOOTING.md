# Troubleshooting and recovery

- REPO_NOT_FOUND/INDEX_NOT_FOUND: select an absolute root with --repo, then run `kortex index /root --verify`. A subdirectory in a Git checkout resolves to its Git top level.
- AMBIGUOUS_SYMBOL: use a logical symbol ID or `relative/path:name`; query first to see names and paths. No guessed target is selected.
- stale: source inventory, hashes, Git HEAD, config/parser fingerprint or resolution inputs differ. Reindex with the same persistent configuration. Citations refer to the indexed generation until then.
- WRITER_BUSY: another process holds the OS advisory writer lock. Do not delete its lock file while it runs. If the process dies, the OS releases the lock; no manual stale-PID override is needed.
- FILES_CHANGED: the defensive pre-commit validation caught edits while indexing. The prior committed generation is intact. Retry or use watch.
- partial/truncated: inspect diagnostics; syntax errors, skipped/limited files, unsupported dynamic calls, node/depth and response budgets can limit analysis. Absence of an edge is not proof of no runtime dependency.
- Git errors: install trusted Git on PATH; validate refs with Git. Kortex does not run hooks, text conversion or external diff drivers. Git reads are bounded to five seconds and 8 MiB output; huge diffs must be scoped externally.
- Corrupt/newer index: `kortex doctor --json` reports an error. `--repair` can fully rebuild a readable stale index, but deliberately does not overwrite a corrupt/newer DB. Stop Kortex processes, back up `.kortex/` for diagnosis, move the entire `.kortex/` index directory aside (preserve `.kortex.toml`), and run `kortex index --full --verify`. Do not delete source or user configuration.
- Disk full/interrupted indexing: SQLite transaction rollback should retain the prior generation. Free space, check with doctor and retry. Actual killed transaction recovery is tested; injected disk-full acceptance remains pending.
- MCP startup: index first; use an absolute executable path and --repo. stdout must contain only MCP JSON-RPC. Client-specific config formats must be checked against that client version.

Only schema v1 is implemented. There are no destructive schema upgrades. Future migrations must back up the old database and add failure-injection tests before shipping.
