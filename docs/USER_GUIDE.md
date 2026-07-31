# ZeroTerm User Guide

GPU-accelerated terminal emulator written in Rust. This guide covers day-to-day
usage; see [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) for settings and
[ARCHITECTURE.md](ARCHITECTURE.md) for internals.

## Getting Started

### Requirements

- Rust 1.79+
- Linux / macOS / Windows (wgpu backend: Vulkan / Metal / DX12)
- A GPU with Vulkan/Metal/DX12 support

### Install

```bash
# Linux/macOS one-liner
curl -fsSL https://zeroterm.dev/install.sh | sh

# Flatpak
flatpak install flathub com.zeroterm.ZeroTerm

# From source
git clone https://github.com/zeroterm/zeroterm.git
cd zeroterm
cargo run --release
```

### First Run

ZeroTerm spawns your login shell (`zsh` on Unix, `cmd.exe` on Windows) in a PTY.
Config is read from `~/.config/zeroterm/config.toml` (or the platform equivalent
of the config dir) and overlaid with Lua from `.zeroterm.lua` in the working
directory. See the config reference.

## Features

- **GPU rendering** — wgpu instanced quads, dynamic glyph atlas, true font metrics
- **Tabs & splits** — tiling pane tree per session, keyboard-driven navigation
- **Block output tracking** — every prompt line starts a block; dividers show exit
  code and wall-clock duration, with one-click block copy
- **Scrollback** — 10,000-line buffer, Shift+PageUp/Home navigation
- **Clipboard** — OSC 52 read/write, native copy/paste, selection copy
- **Local AI** — `Ctrl+Shift+I` asks an Ollama/LM Studio endpoint to explain the
  current screen
- **SSH sessions** — native SSH client, one key to connect to a configured host
- **Settings sync** — E2E encrypted (ChaCha20-Poly1305) push/pull of config
- **Session restore** — tabs/panes are re-spawned on next launch
- **Transparency** — cycle window opacity, persisted per config

## Keyboard Shortcuts

| Shortcut            | Action                         |
| ------------------- | ------------------------------ |
| `Ctrl+Shift+T`      | New tab                        |
| `Ctrl+Shift+W`      | Close active tab               |
| `Ctrl+Tab`          | Next tab                       |
| `Ctrl+Shift+Tab`    | Previous tab                   |
| `Alt+1` … `Alt+9`   | Switch to tab 1-9              |
| `Ctrl+Shift+E`      | Split pane vertically          |
| `Ctrl+Shift+D`      | Split pane horizontally        |
| `Alt+Arrow`         | Focus adjacent pane            |
| `Ctrl+Shift+I`      | Ask local AI to explain screen |
| `Ctrl+Shift+O`      | Cycle window opacity           |
| `Ctrl+Shift+S`      | Connect SSH (config.ssh.host)  |
| `Ctrl+Shift+C`      | Copy selection                 |
| `Ctrl+Shift+V`      | Paste (honors bracketed paste) |
| `Ctrl+A` … `Ctrl+Z` | Send control char to shell     |
| `Ctrl+Space`        | Send NUL (0x00)                |
| `Shift+PageUp/Down` | Scroll back/forward 20 lines   |
| `Shift+Home`        | Jump to oldest scrollback      |
| `Shift+End`         | Jump to latest output          |

> `Ctrl+Shift+Z` (fullscreen) and zoom are listed in README but not yet bound.
> Move-tab bindings are also pending.

## Working with Blocks

Each line beginning with a prompt sigil (`$`, `%`, `#`, `>`) marks a block
boundary. The boundary row renders as a dim divider carrying the previous
block's metadata:

```
$ cargo build
...output...
$ ·────────────── [copy]exit:0 · 3421ms
```

- **Metadata** — exit code and duration of the block above, right-aligned.
- **Copy block** — click the `[copy]` marker at the far right of a divider to
  copy that block's command + output to the clipboard.

## FAQ

**Where is the config file?**
`~/.config/zeroterm/config.toml` (or `dirs::config_dir()/zeroterm/config.toml`).
It is watched and hot-reloaded.

**Why does my partial config get ignored?**
The TOML file is deserialized as a whole. Missing a non-optional key rejects
the file, and ZeroTerm falls back to defaults. Use the full template in
`CONFIG_REFERENCE.md`.

**How do I set up the AI explain feature?**
Point `[ai] endpoint` at an Ollama or LM Studio server, e.g. `http://localhost:11434`,
then press `Ctrl+Shift+I`. The hardcoded model is `llama3.2`.

**How do I SSH from ZeroTerm?**
Fill in `[ssh]` in config.toml (host/user/port/key_path) and press
`Ctrl+Shift+S`. Auth order: password (not currently wired from config), key
file, then SSH agent.

**Why doesn't my mouse select work in `vim`/`htop`?**
Applications that enable mouse tracking take over mouse events (SGR 1000-1003).
Selection is disabled while they're active; `Ctrl+Shift+C` copies whatever was
selected before.

**Where is the session restored from?**
`session.json` lives next to `config.toml`. Tabs/panes are saved on exit and
re-spawned on launch. The first saved pane is always the primary shell.

**How do I reset everything?**
Delete `~/.config/zeroterm/config.toml` and `~/.config/zeroterm/session.json`.
Also remove `.zeroterm.lua` if you rely on it.
