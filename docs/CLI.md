# CLI 0.1.0

Run `kortex --help` and `kortex COMMAND --help` for the actual Clap contract. Global `--repo PATH|ID`, `--config PATH`, and `--json` work before or after subcommands. Without `--repo`, the nearest indexed current-directory ancestor is used; otherwise a single registered repository can be selected. Multiple registry entries require explicit selection.

| Command | Shipped behavior |
|---|---|
| init [path] | Create default TOML without overwriting or changing Git ignore files |
| index/analyze [path] | Discover, parse/cache, resolve and atomically commit |
| watch [path] | Native events, default 500 ms debounce, two-second reconciliation; Ctrl-C shutdown |
| status [path], doctor | Freshness, graph counts, diagnostics and SQLite integrity |
| doctor --repair | Full rebuild of readable index; corruption recovery is manual |
| repos list | Registered repositories, paths, generations and freshness |
| repos remove ID | Unregister; `--delete-index --yes` deletes validated index DB files only |
| query TEXT | Exact/prefix/path/FTS ranking; `--limit 1..500`, `--cursor N` |
| context SELECTOR | Declaration, owner, incoming/outgoing calls, direct tests |
| impact SELECTOR | `--direction upstream|downstream|both`, depth 0..10 |
| trace --from A --to B | Shortest dependency path within node/depth policy |
| changes | Read-only Git hunks to indexed symbols and likely dependents |
| graph SELECTOR | Bounded neighborhood; `--format json|dot|graphml` |
| mcp | SDK stdio transport; not an HTTP service |
| config show/validate/path | Effective config, validation, config/registry paths |
| completion SHELL | bash, zsh, fish, powershell, elvish via clap_complete |
| version | Version/index format and development status |
| serve | `--mcp stdio` aliases the read-only stdio server; plain HTTP remains NOT_IMPLEMENTED P1 |

Selectors: symbol ID, exact qualified name, name (ambiguous names error), `path:name`, or `path:line`. Coordinates are 1-based UTF-8 bytes. No rename or source editing operation exists.

Index flags: `--full`/`--incremental`, `--verify` (re-scan inventory), `--dry-run` (no index/registry write), `--index-only` (normal behavior, no post-index remote work), `--include`, `--exclude`, `--include-hidden`, `--follow-symlinks`, `--max-file-size`, `--max-files`, `--languages rust,python,typescript,javascript,go`. Default exclusions cannot be overridden to scan `.git`/`.kortex`/build/vendor caches. Changing discovery flags changes the config fingerprint; use a persistent TOML config for consistent freshness checks.

`changes --unstaged` (also default) compares worktree to staging; `--staged` compares staging to HEAD; `--base REF [--head REF]` selects revisions. Git is invoked with explicit arguments, external diff/text conversion disabled. Indexed citations may refer to older spans; heed staleness warnings.

Exit codes: 0 success; 2 usage/config; 3 missing repo/index/symbol; 4 ambiguity; 5 explicitly required freshness failure (MCP/API); 6 storage/corruption; 7 partial/limits/content changed; 8 security refusal; 9 timeout/cancellation; 1 unexpected I/O/application failure. Architecture exit 10 is reserved, not shipped. Human output is currently pretty JSON; `--json` is single-line machine output. Watch emits JSON Lines. MCP stdout is protocol-only.
