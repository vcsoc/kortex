# Windows validation evidence and remaining gates

Windows is NOT accepted for private production use. Local cross-compilation is not Windows execution, installation, ACL enforcement, filesystem locking or crash-safety evidence.

## Genuine local cross-compilation

The persistence worker implemented real Windows handle-relative source reads using cap-std/cap-fs-ext and real Windows junction/stream/device fixtures. Its config/repository Windows check and clippy passed using the official rustup toolchain. The bundled SQLite C build initially blocked storage validation because no MinGW compiler was installed.

Parent installed a development-only Zig 0.16.0 compiler under `target/tools`, not system-wide. Official archive:

- URL: `https://ziglang.org/download/0.16.0/zig-aarch64-macos-0.16.0.tar.xz`
- Published archive bytes: 52,238,004
- SHA-256: `b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489`

The download was size-bounded, checked against that digest, and extracted only after rejecting escaping/nonregular archive members. `target/tools/zig-provenance.json` records the tool source and scope. This is local tool integrity evidence, not an independent publisher/signature audit or a runtime dependency of the installed Kortex application.

`packaging/zig_windows_cc.py` is a minimal real compiler driver for this host. It translates Rust's vendor-qualified `x86_64-pc-windows-gnu` target to Zig's `x86_64-windows-gnu`, then execs the actual Zig/Clang compiler. It does not generate dummy objects, suppress diagnostics, emulate Windows or replace a failed compilation with success. Six unit tests cover argument preservation/mapping. A real deliberately invalid C probe exited 1 with its actual preprocessor diagnostic and produced no object. LLVM inspection of the actual compiled SQLite object reports `COFF-x86-64` and `IMAGE_FILE_MACHINE_AMD64`. Evidence: `target/windows-compiler-evidence.json`.

Reproduction on this aarch64 macOS development host, after obtaining/checking the exact tool archive and installing the official Rust Windows GNU standard library:

```sh
rustup target add --toolchain 1.99.1 x86_64-pc-windows-gnu
chmod 700 packaging/zig_windows_cc.py
toolchain="$(dirname "$(rustup which --toolchain 1.99.1 rustc)")"
export PATH="$toolchain:$PATH"
export RUSTC="$toolchain/rustc"
export CARGO_TARGET_DIR="$PWD/target/parent-windows-check"
export ZIG_GLOBAL_CACHE_DIR="$PWD/target/tools/zig-cache"
export CC_x86_64_pc_windows_gnu="$PWD/packaging/zig_windows_cc.py"
export AR_x86_64_pc_windows_gnu="$(dirname "$toolchain")/lib/rustlib/aarch64-apple-darwin/bin/llvm-ar"
cargo check -p kortex-storage --all-targets --all-features --target x86_64-pc-windows-gnu --locked
cargo clippy -p kortex-storage --all-targets --all-features --target x86_64-pc-windows-gnu --locked -- -D warnings
```

Both storage checks PASS locally. Cross-clippy exposed two Windows-only `unused_mut` errors in directory builders; parent fixed their cfg-specific bindings and reran the strict check successfully. Logs: `target/parent-storage-windows-check.log`, `target/parent-storage-windows-clippy.log`. The normal macOS persistence suite separately passes 67 scoped tests. None of these commands executes a Windows test or proves that all workspace crates link into a Windows executable.

## Unresolved production criteria

- Current Windows storage does not enforce or verify current-owner/private DACLs. Unix 0700/0600 branches are not Windows privacy enforcement. Need a genuinely audited safe implementation, owner-only creation before derived-source writes, inherited/permissive/foreign-owner rejection fixtures, and real Windows execution. Do not treat the current Windows index/cache as verified private storage.
- SQLite, writer locking and repair still resolve ambient parent paths. NOFOLLOW on the final DB and no-replace renames do not solve parent replacement races. Need a handle-bound VFS/parent-pinning design and descriptor-relative preservation/inventory, or a justified fail-closed placement policy.
- Root-handle acquisition ancestors remain a trust boundary, even though subsequent source traversal is confined by real handles.
- Windows fixture execution, NTFS behavior, symlink privilege paths, native installation, watcher behavior, ACL privacy, file sharing/locks, restart/crash and actual ENOSPC/fsync evidence remain missing.
- No remote runner, VM, publication or upload was started. Hosted workflows require both private repository visibility and explicit owner approval; configured jobs are not execution evidence.
