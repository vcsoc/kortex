# Getting started (development 0.1.0)

Prerequisites: Rust 1.99.1 toolchain including Cargo, a native C compiler for bundled SQLite/Tree-sitter, and trusted Git on PATH. Runtime indexing requires no package manager or network. Public installation from crates.io/GitHub Releases is not available or approved yet.

On macOS/Linux, from this checkout:

```sh
cargo build --release --locked -p kortex-cli
./target/release/kortex version --json
demo=$(mktemp -d)
cp -R fixtures/polyglot/. "$demo/"
./target/release/kortex index "$demo" --verify --json
./target/release/kortex --repo "$demo" query py_check --json
./target/release/kortex --repo "$demo" context auth.py:py_check --json
./target/release/kortex --repo "$demo" impact auth.py:py_check --json
./target/release/kortex --repo "$demo" trace --from py_entry --to py_check --json
```

Note: fixtures inside this Git checkout are discovered at the enclosing repository root. To isolate the synthetic repository, copy `fixtures/polyglot` to a directory outside this checkout before indexing. No fixture code needs to be run.

On Windows (PowerShell with MSVC Build Tools installed):

```powershell
cargo build --release --locked -p kortex-cli
.\target\release\kortex.exe version --json
.\target\release\kortex.exe index C:\path\to\repository --verify --json
.\target\release\kortex.exe --repo C:\path\to\repository query check --json
```

Only local macOS has been exercised in this implementation session; Windows/Linux steps await CI evidence. Do not interpret the snippets as verified platform-release availability.

For your own repository, run `kortex init /path/to/repo` to write a new `.kortex.toml` (refuses overwrite), then `kortex index /path/to/repo --verify`. Kortex never edits `.gitignore`. Keep `.kortex/` private and out of version control. Use `kortex doctor --repo /path/to/repo --json` to inspect diagnostics and health.
