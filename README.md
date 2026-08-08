# ZeroTerm

[![CI](https://github.com/mahesh-diwan/ZeroTerm/actions/workflows/ci.yml/badge.svg)](https://github.com/mahesh-diwan/ZeroTerm/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/mahesh-diwan/ZeroTerm)](https://github.com/mahesh-diwan/ZeroTerm/releases)
![Rust](https://img.shields.io/badge/language-Rust-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

ZeroTerm is a GPU-accelerated terminal emulator written in Rust. Rendering is
done with [wgpu](https://wgpu.rs) (Metal on macOS, DX12/Vulkan on Windows and
Linux), and the project is organized as a Cargo workspace of nine crates.

## Status

- **Linux and macOS** are the supported platforms and run the full CI suite
  (build, tests, clippy, fmt).
- **Windows** has a packaging target in the release pipeline, but the Windows
  app build is **not yet verified in CI** and should be treated as
  experimental.
- There is **no crates.io package, Homebrew formula, or Flathub listing yet** —
  `cargo install zeroterm`, `brew install zeroterm`, and `flatpak install …`
  will not work today. The one supported installation path is the install
  script below — no AppImage/DEB/RPM/brew artifacts are published separately.

## Features

- ⚡ GPU-accelerated rendering via wgpu (Metal / DX12 / Vulkan)
- 📑 Tabs and tiled split panes, plus a floating-pane overlay
- 🔭 Scrollback with search, output-block navigation, and syntax highlighting
- 🖼️ Kitty, Sixel, and iTerm2 inline image protocols
- 🖥️ Native SSH client with persistent sessions (Unix, feature-gated)
- 🤖 Optional local AI integration (Ollama / LM Studio) for explain & suggest
- 🔒 End-to-end encrypted settings sync (ChaCha20-Poly1305)
- 🔌 WASM plugin sandbox (wasmtime)
- ⚙️ TOML configuration with optional Lua scripting
- ⌨️ Readline-style multi-line line editor with history

## Install

### Install script (Linux / macOS) — the one supported way

```bash
curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash
```

The installer resolves the **latest tagged release**, downloads a prebuilt
package for your OS/architecture when one is published, and otherwise builds
from source at that exact tag — so what you run always matches the tagged
source. Re-run the same command (or `zeroterm upgrade`) to update to the latest
release. Windows is experimental through the same script.

This single curl command is the only supported installation channel: no
AppImage/DEB/RPM/brew taps are published separately.

### Build from source

```bash
git clone https://github.com/mahesh-diwan/ZeroTerm.git
cd ZeroTerm
cargo run --release
```

Requires a stable Rust toolchain (via [rustup](https://rustup.rs)) and the
platform prerequisites for wgpu (e.g. Vulkan drivers on Linux). Lua 5.4 is
bundled (mlua's vendored build), so no system Lua package is needed:

```bash
# Debian/Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libx11-dev \
     libxrandr-dev libxi-dev libgl-dev libssl-dev pkg-config

# macOS (Homebrew)
brew install cmake pkg-config
```

## Configuration

ZeroTerm reads `~/.config/zeroterm/config.toml` for core settings and an
optional `~/.zeroterm.lua` Lua script for advanced customization.

- [Config reference](docs/CONFIG_REFERENCE.md)
- [User guide](docs/USER_GUIDE.md)
- [Plugin development guide](docs/PLUGIN_DEV_GUIDE.md)
- [Architecture](docs/ARCHITECTURE.md)

## Keyboard Shortcuts

| Shortcut                 | Action                                |
| ------------------------ | ------------------------------------- |
| `Ctrl+Shift+T`           | New tab                               |
| `Ctrl+Shift+W`           | Close active tab                      |
| `Ctrl+Tab`               | Next tab                              |
| `Ctrl+Shift+Tab`         | Previous tab                          |
| `Alt+1` … `Alt+9`        | Switch to tab 1-9                     |
| `Ctrl+Shift+E`           | Split pane vertically                 |
| `Ctrl+Shift+D`           | Split pane horizontally               |
| `Ctrl+Shift+G`           | Float active pane (overlay)           |
| `Alt+Arrow`              | Focus adjacent pane                   |
| `Ctrl+Shift+F`           | Toggle search overlay                 |
| `Esc`                    | Close search overlay                  |
| `Ctrl+Shift+J`/`K`       | Jump to next/previous output block    |
| `Ctrl+Shift+I`           | Ask local AI to explain screen (`ai`) |
| `Ctrl+Shift+P`           | Open settings overlay                 |
| `Ctrl+Shift+O`           | Cycle window opacity                  |
| `Ctrl+Shift+S`           | Connect SSH (`ssh` feature)           |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy selection / paste         |
| `F12`                    | Toggle quake (drop-down) mode         |
| `Ctrl+A` … `Ctrl+Z`      | Send control character to the shell   |
| `Ctrl+Space`             | Send NUL (`0x00`)                     |
| `Shift+PageUp/Down`      | Scroll back/forward                   |
| `Shift+Home` / `Shift+End` | Jump to oldest / newest scrollback  |

> A plain `PageUp`/`PageDown`/`Home`/`End` is forwarded to the shell (for
> `less`, `vim`, etc.); only the `Shift` variants scroll the scrollback.

## Project Structure

```
├── crates/
│   ├── zeroterm-core      — VT100/ANSI parser, screen buffer, cell model, images
│   ├── zeroterm-render    — wgpu GPU renderer, glyph atlas, themes
│   ├── zeroterm-mux       — tabs, split tree, session management
│   ├── zeroterm-config    — TOML + Lua configuration
│   ├── zeroterm-ai        — local AI integration (Ollama / LM Studio)
│   ├── zeroterm-sync      — E2E-encrypted settings sync
│   ├── zeroterm-ssh       — native SSH client
│   ├── zeroterm-plugin    — WASM plugin sandbox
│   └── zeroterm           — the main application binary
├── scripts/               — installer, packaging, CI helpers
├── docs/                  — architecture, config, user and plugin guides
└── landing/               — marketing site (Next.js)
```

## Development

```bash
cargo build            # debug build
cargo build --release  # release build
cargo test             # run all tests
cargo clippy           # lint
cargo fmt --check      # check formatting
```

## License

MIT
