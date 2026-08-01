# Windows Packaging Notes

## ConPTY status

ZeroTerm uses `portable-pty` (workspace dependency, see `crates/zeroterm-core/Cargo.toml`),
which wraps the Windows ConPTY API (`pseudo_console` / `winpty` fallback) under the hood.
No source changes are required for Windows: the terminal runs against ConPTY out of the box.

`zeroterm-ssh` is the only crate gated to unix (`[target.'cfg(unix)'.dependencies]` in
`crates/zeroterm/Cargo.toml`); everything else is cross-platform.

## Building on Windows (native, MSVC)

1. Install Rust with the MSVC toolchain:
   `rustup toolchain install stable-x86_64-pc-windows-msvc` (default via rustup on Windows)
2. Build: `cargo build --release -p zeroterm`
3. Binary: `target/release/zeroterm.exe`

## Cross-compiling from Linux/macOS

Not recommended for the MSVC target: linking needs the MSVC runtime libraries. Use a
native Windows host or GitHub Actions (`windows-latest`). The `x86_64-pc-windows-gnu`
target can cross-compile with mingw but ConPTY integration should be tested natively.

## Packaging: zip + optional WiX MSI

`scripts/package_windows.bat` (run on Windows, cmd.exe or Git Bash) does:

1. `cargo build --release -p zeroterm`
2. `dist/zeroterm-<version>-windows-x86_64.zip` (binary + README, via PowerShell)
3. If WiX Toolset v3 (`candle`/`light` on PATH): builds `dist/zeroterm-<version>-x86_64.msi`
   from `scripts/windows_installer.wxs`

Install WiX: `winget install --id WiXToolset.WiXToolset.3` (or download from https://wixtoolset.org).

The `.bat` detects whether it runs inside Git Bash/MSYS via `where uname` (the `uname`
guard) so it can print a cross-compile hint instead of silently doing the wrong thing.

## Installation on Windows

- The `install.sh` installer targets unix-ish shells (MSYS/Git Bash). For PowerShell use
  `scripts/install.ps1`.
- The MSI installs `zeroterm.exe` to `Program Files\ZeroTerm` and adds it to PATH.
