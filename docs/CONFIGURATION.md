# Configuration

Use `kortex init .` to generate the exact supported defaults. Every TOML structure rejects unknown fields. Resolution: index CLI flags > supported environment overrides > explicit `--config` or repo `.kortex.toml` > user config > defaults. `config show` outputs effective settings. Explicit config also applies to CLI queries and MCP.

User config is `$XDG_CONFIG_HOME/kortex/config.toml`, `%APPDATA%/kortex/config.toml`, or `$HOME/.config/kortex/config.toml`, in that order. Registry is `$KORTEX_REGISTRY` if supplied; otherwise under XDG_DATA_HOME, LOCALAPPDATA, or `$HOME/.local/share/kortex/registry.db`.

Every existing config field supports a nested environment override such as `KORTEX_ANALYSIS__MAX_NODES=100` or `KORTEX_INDEX__WORKERS=2`. Strings are literal; booleans/numbers/arrays use TOML values. Unknown nested fields error. Flat compatibility aliases `KORTEX_MAX_FILE_SIZE_BYTES`, `KORTEX_MAX_FILES`, `KORTEX_INCLUDE_HIDDEN` are also supported and win over their nested equivalents. `KORTEX_REGISTRY` selects the user registry path. Secrets are never accepted in repo TOML.

```toml
schema_version = 1
[repository]
include_hidden = false
follow_symlinks = false
max_file_size_bytes = 2097152
max_total_bytes = 268435456
max_files = 100000
exclude = ["**/generated/**"]
languages = ["rust", "python", "typescript", "javascript", "go"]
[index]
mode = "incremental"
workers = 0
watch_debounce_ms = 500
storage = "repository"
[analysis]
default_depth = 3
max_depth = 10
max_nodes = 500
query_timeout_ms = 10000
[search]
backend = "sqlite_fts5"
embeddings = "disabled"
[mcp]
read_only = true
max_response_bytes = 262144
```

Defaults: 2 MiB/file, 256 MiB total source, 100,000 discovery entries; hard maxima 64 MiB/file, 1 GiB total and 1,000,000 entries. Parser pool uses up to eight available CPUs; configured workers are clamped to that ceiling. Tree-sitter parse deadline is five seconds/file and extraction depth is capped at 256. Graph depth <=10, max nodes <=10,000, timeout <=60,000 ms; MCP applies tighter schema bounds.

Only repository-local storage is supported. Shared user-cache placement, manifest workspaces, config schema migration, configurable repository-root relocation and embeddings are not shipped. `repository.root` is reserved and must be `.`; other values error. Select root with CLI path. HTTP enablement, non-read-only MCP, and embedding enablement are refused by policy. No repository configuration can turn on remote execution or source uploads.
