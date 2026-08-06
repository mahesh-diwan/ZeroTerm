# ZeroTerm User Guide

GPU-accelerated terminal emulator written in Rust. This guide covers day-to-day
usage; see [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) for settings and
[ARCHITECTURE.md](ARCHITECTURE.md) for internals.

## Getting Started

### Requirements

- Rust stable (via [rustup](https://rustup.rs))
- Linux or macOS (Windows builds are not yet verified in CI)
- A GPU with Vulkan/Metal/DX12 support

### Install

```bash
# Linux/macOS one-liner (resolves the latest release, downloads a prebuilt
# binary when available, otherwise builds from source)
curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | sh

# From source
git clone https://github.com/mahesh-diwan/ZeroTerm.git
cd ZeroTerm
cargo run --release
```

> ZeroTerm is not yet published to crates.io, Homebrew, or Flathub; use the
> install script or build from source. Prebuilt binaries are attached to each
> [GitHub Release](https://github.com/mahesh-diwan/ZeroTerm/releases).

### First Run

ZeroTerm spawns your login shell (`zsh` on Unix, `cmd.exe` on Windows) in a PTY.
Config is read from `~/.config/zeroterm/config.toml` (or the platform equivalent
of the config dir) and overlaid with Lua from `.zeroterm.lua` in the working
directory. See the config reference.

## Features

- **GPU rendering** — wgpu instanced quads, dynamic glyph atlas, true font metrics
- **Tabs & splits** — tiling pane tree per session, keyboard-driven navigation,
  drag-resizable dividers, floating overlay pane
- **Block output tracking** — every prompt line starts a block; dividers show exit
  code and wall-clock duration, with one-click block copy
- **Scrollback** — 10,000-line buffer; Shift+PageUp/Home navigation with smooth
  velocity-based scrolling
- **Search overlay** — `Ctrl+Shift+F`, in-terminal regex search across the screen
- **Clipboard** — OSC 52 read/write, native copy/paste, copy-on-select
- **URL detection** — URLs are highlighted inline; hover/copy affordances
- **Local AI** — `Ctrl+Shift+I` asks an Ollama/LM Studio endpoint to explain the
  current screen
- **SSH sessions** — native SSH client, one key to connect to a configured host
- **Settings sync** — E2E encrypted (ChaCha20-Poly1305) push/pull of config
- **Plugins** — sandboxed WASM/WASI command plugins (see [PLUGIN_DEV_GUIDE.md](PLUGIN_DEV_GUIDE.md))
- **Session restore** — tabs/panes are re-spawned on next launch
- **Transparency** — cycle window opacity, persisted per config
- **Quake mode** — `F12` toggles a full-width drop-down window over your desktop

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
| `Ctrl+Shift+G`      | Float active pane (overlay)    |
| `Alt+Arrow`         | Focus adjacent pane            |
| `Ctrl+Shift+F`      | Toggle search overlay          |
| `Esc`               | Close search overlay           |
| `Ctrl+Shift+I`      | Ask local AI to explain screen |
| `Ctrl+Shift+O`      | Cycle window opacity           |
| `Ctrl+Shift+S`      | Connect SSH (config.ssh.host)  |
| `Ctrl+Shift+C`      | Copy selection                 |
| `Ctrl+Shift+V`      | Paste (honors bracketed paste) |
| `F12`               | Toggle quake mode              |
| `Ctrl+A` … `Ctrl+Z` | Send control char to shell     |
| `Ctrl+Space`        | Send NUL (0x00)                |
| `Shift+PageUp/Down` | Scroll back/forward 20 lines   |
| `Shift+Home`        | Jump to oldest scrollback      |
| `Shift+End`         | Jump to latest output          |

> **Scroll gotcha:** a _plain_ `PageUp`/`PageDown`/`Home`/`End` is forwarded to
> the shell as an escape sequence (for `less`, `vim`, etc.). Only the `Shift`
> variants scroll the ZeroTerm scrollback.

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
- **Jump between blocks** — `Ctrl+Shift+J` / `Ctrl+Shift+K` move to the next /
  previous block boundary.

## Search Overlay

`Ctrl+Shift+F` opens an in-terminal search box. Type a query; matches are
highlighted across the screen and scrollback, and the view jumps to the first
hit. `Enter` steps forward, `Shift+Enter` backward, `Esc` (or `Ctrl+Shift+F`
again) closes and returns to the shell.

## Selection & Copy

- **Copy-on-select** — dragging to select text copies it to the clipboard on
  mouse release. `Ctrl+Shift+C` re-copies the current selection.
- **Selection extend** — hold `Shift` while pressing arrow keys to extend the
  selection.
- Selection is disabled while the foreground app has mouse tracking (e.g.
  `vim`/`htop`); it copies whatever was selected beforehand.

## URLs

URLs (`http(s)://`, `ftp://`, `www.`) in terminal output are detected and
highlighted inline as you type, across both the visible screen and scrollback.
Highlighting only — there is no click-to-open yet; copy the URL with selection
and paste it into your browser.

## Tabs & Splits

- **Modern tab bar** — a GPU-rendered tab strip above the grid. Hovering a tab
  reveals a close (`×`) button; click it to close, click the tab body to switch.
- **Split panes** — `Ctrl+Shift+E` (vertical) and `Ctrl+Shift+D` (horizontal)
  split the active pane. Drag any divider to resize the adjacent panes.
- **Floating pane** — `Ctrl+Shift+G` pops the active pane out of the tree as a
  fullscreen overlay; pressing it again docks it back. One pane floats at a
  time.

## Quake Mode

`F12` toggles quake mode: the window snaps to a full-width, drop-down bar
anchored to the top of the screen, overlay-style. Press `F12` again to restore
the normal windowed position.

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
