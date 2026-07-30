# ZeroTerm

[![CI](https://github.com/zeroterm/zeroterm/actions/workflows/ci.yml/badge.svg)](https://github.com/zeroterm/zeroterm/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/rust-1.79%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

GPU-accelerated terminal emulator written in Rust.

## Features

- ⚡ GPU-accelerated rendering via wgpu (Metal/DX12/Vulkan)
- 📑 Tab management with split panes
- 🤖 Local AI integration (Ollama/LM Studio)
- 🔒 E2E encrypted settings sync
- 🐧 Cross-platform (macOS, Linux, Windows)
- 📜 Full VT100/ANSI escape sequence support
- 🎨 Dynamic glyph atlas with font metrics
- 🔌 SSH session support via portable-pty

## Quick Start

### Install via script (Linux/macOS)

```bash
curl -fsSL https://zeroterm.dev/install.sh | sh
```

### Build from source

```bash
git clone https://github.com/zeroterm/zeroterm.git
cd zeroterm
cargo run --release
```

### Flatpak

```bash
flatpak install flathub com.zeroterm.ZeroTerm
```

![ZeroTerm Screenshot](docs/screenshot.png)

## Configuration

ZeroTerm reads `~/.config/zeroterm/config.toml` for core settings and supports Lua scripting via `~/.zeroterm.lua` for advanced customization.

See [docs/configuration.md](docs/configuration.md) for full reference.

## Keyboard Shortcuts

| Shortcut                | Action                  |
| ----------------------- | ----------------------- |
| `Ctrl+Shift+T`          | New tab                 |
| `Ctrl+Shift+W`          | Close tab               |
| `Ctrl+Tab`              | Next tab                |
| `Ctrl+Shift+Tab`        | Previous tab            |
| `Ctrl+Shift+Left/Right` | Move tab                |
| `Ctrl+Shift+D`          | Split pane horizontally |
| `Ctrl+Shift+E`          | Split pane vertically   |
| `Ctrl+Shift+Arrow`      | Navigate panes          |
| `Ctrl+Shift+Z`          | Toggle fullscreen       |
| `Ctrl++` / `Ctrl+-`     | Zoom in/out             |
| `Ctrl+Shift+C`          | Copy                    |
| `Ctrl+Shift+V`          | Paste                   |

## Project Structure

```
├── crates/
│   ├── zeroterm-core      — VT100 parser, screen buffer, cell model
│   ├── zeroterm-render    — wgpu GPU renderer, glyph atlas
│   ├── zeroterm-mux       — Tab/split/session management
│   ├── zeroterm-config    — TOML + Lua config
│   ├── zeroterm-ai        — Local AI integration (Ollama/LM Studio)
│   ├── zeroterm-sync      — E2E encrypted settings sync
│   └── zeroterm           — Main binary
└── Cargo.toml             — Workspace root (7 crates)
```

## Development

```bash
cargo build           # debug build
cargo build --release # release build
cargo test            # run tests
cargo clippy          # lint
```

## License

MIT
