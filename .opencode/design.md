# ZeroTerm — Design Document

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    UI Thread (60–120 FPS)               │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │  wgpu    │  │  Input   │  │  Window  │  │  Config  │ │
│  │ Renderer │  │ Handler  │  │ Manager  │  │  Panel   │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘ │
│       └─────────────┴──────────────┴─────────────┘      │
│              Glyph Atlas (GPU texture)                  │
└─────────────────────────────────────────────────────────┘
                    ↑ (channel)
┌─────────────────────────────────────────────────────────┐
│                   Parser Thread (tokio)                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │  VT100   │  │  OSC/    │  │  Screen  │  │  Scroll- │ │
│  │  Parser  │  │  DCS     │  │  Buffer  │  │  back    │ │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘ │
└─────────────────────────────────────────────────────────┘
                    ↑ (channel)
┌─────────────────────────────────────────────────────────┐
│                   I/O Layer (tokio)                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │  PTY     │  │  SSH     │  │  Plugin  │  │  Sync    │ │
│  │ (portable-pty) │ (thrussh) │ (WASM)   │  │ (E2EE)   │ │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Thread Model

- **UI Thread**: wgpu rendering, input handling, window management. Never blocks.
- **Parser Thread**: VT100 state machine, screen buffer updates, scrollback management. Communicates with UI via lock-free channel.
- **I/O Threads**: PTY/SSH I/O, plugin IPC, sync daemon. All async via tokio.

### Why This Architecture

- **No GC pauses**: Rust ownership model eliminates garbage collection entirely
- **No blocking**: tokio async runtime handles all I/O without blocking the render thread
- **GPU-first**: wgpu provides native Metal/DX12/Vulkan with zero abstraction penalty
- **Separation of concerns**: Parser thread never touches GPU; UI thread never blocks on I/O

## Crate Structure

```
aether/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── zeroterm-core/        # VT parser, screen buffer, cell model
│   │   ├── src/
│   │   │   ├── parser.rs    # VT100 state machine
│   │   │   ├── screen.rs    # Screen buffer + scrollback
│   │   │   ├── cell.rs      # Cell model (char + attributes)
│   │   │   └── pty.rs       # PTY abstraction trait
│   │   └── tests/
│   │       └── unicode_test.rs  # Unicode terminal test suite
│   ├── zeroterm-render/      # wgpu renderer, glyph atlas, font handling
│   │   ├── src/
│   │   │   ├── renderer.rs  # wgpu render pipeline
│   │   │   ├── atlas.rs     # Dynamic glyph atlas
│   │   │   └── font.rs      # Font loading + shaping (swash)
│   ├── zeroterm-mux/         # Tabs, splits, session management
│   │   ├── src/
│   │   │   ├── tab.rs       # Tab model
│   │   │   ├── pane.rs      # Pane (split) model
│   │   │   └── session.rs   # Session lifecycle
│   ├── zeroterm-config/      # TOML parser, Lua runtime, GUI settings
│   │   ├── src/
│   │   │   ├── config.rs    # Config schema + TOML parsing
│   │   │   ├── lua.rs       # Lua runtime (optional)
│   │   │   └── gui.rs       # GUI settings panel
│   ├── zeroterm-ai/          # AI integration (optional)
│   │   └── src/
│   │       └── mod.rs       # Ollama/LM Studio client
│   ├── zeroterm-sync/        # E2E encrypted sync (optional)
│   │   └── src/
│   │       └── mod.rs       # Sync daemon + crypto
│   └── zeroterm/             # Main binary
│       └── src/
│           └── main.rs      # Entry point + app lifecycle
├── assets/                 # Icons, default fonts, shaders
├── docs/
│   ├── unicode-test-plan.md
│   └── escape-sequence-reference.md
└── scripts/
    └── ci.sh              # CI pipeline
```

## Data Model

### Cell

```rust
pub struct Cell {
    pub char: char,
    pub foreground: Color,
    pub background: Color,
    pub attributes: Attributes,
}

pub struct Attributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: UnderlineStyle,
    pub strikethrough: bool,
    pub dim: bool,
    pub blink: bool,
    pub reverse: bool,
    pub invisible: bool,
    pub hyperlink: Option<Hyperlink>,
}

pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
```

### Screen Buffer

```rust
pub struct ScreenBuffer {
    pub rows: VecDeque<Row>,     // Ring buffer for scrollback
    pub cursor: Cursor,
    pub size: Size,              // Columns x Rows
    pub scrollback_limit: usize, // Max scrollback lines (0 = unlimited)
}

pub struct Row {
    pub cells: Vec<Cell>,
    pub line_height: f32,        // For variable line height (wrapped lines)
}
```

### Session

```rust
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,       // Local(PTY) | Remote(SSH)
    pub screen: ScreenBuffer,
    pub title: String,
    pub working_dir: PathBuf,
    pub process_name: String,
    pub exit_code: Option<i32>,
    pub metadata: SessionMetadata,
}

pub enum SessionKind {
    Local { pty: Pty },
    Remote { ssh: SshSession },
}
```

### Block (Command Output)

```rust
pub struct Block {
    pub id: BlockId,
    pub command: String,
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub exit_code: Option<i32>,
    pub output_range: Range<usize>,  // Lines in screen buffer
    pub metadata: BlockMetadata,
}
```

## API Contracts

### Plugin API (WASM)

Plugins run in a WASM sandbox with a restricted API:

```rust
// Plugin host provides this interface
#[derive(Serialize, Deserialize)]
pub struct PluginApi {
    pub on_output: fn(block: &Block) -> PluginResult,
    pub on_command: fn(cmd: &str) -> PluginResult,
    pub get_config: fn() -> Config,
    pub set_config: fn(config: Config) -> PluginResult,
}
```

- Plugins are WASM modules compiled to `wasm32-wasi`
- Execution timeout: 100ms per call
- No filesystem access, no network access
- Communication via tokio channels

### Config Schema (TOML)

```toml
# ~/.config/aether/config.toml

[window]
width = 1200
height = 800
startup_position = "center"

[font]
family = "JetBrains Mono"
size = 14.0
line_height = 1.2

[colors]
# Base16 or custom
primary = { foreground = "#e0e0e0", background = "#1e1e1e" }

[keybindings]
"Ctrl+Shift+T" = "new_tab"
"Ctrl+Shift+D" = "split_vertical"
"Ctrl+Shift+%" = "split_horizontal"

[shell]
program = "zsh"
args = ["--login"]

[ai]
enabled = false
provider = "ollama"
endpoint = "http://localhost:11434"
model = "deepseek-coder"

[sync]
enabled = false
server = "https://sync.aether.run"
# Encryption key is generated locally, never sent to server
```

### AI Integration API

```rust
pub struct AiClient {
    pub provider: AiProvider,
    pub endpoint: String,
    pub model: String,
}

pub enum AiProvider {
    Ollama,
    LmStudio,
}

impl AiClient {
    pub async fn explain(&self, command_output: &str) -> Result<String>;
    pub async fn suggest(&self, prompt: &str) -> Result<String>;
}
```

## Security

### Threat Model

| Asset             | Threat                | Mitigation                                                |
| ----------------- | --------------------- | --------------------------------------------------------- |
| Shell credentials | Keylogger in terminal | OS-level input isolation (no plugin access to keystrokes) |
| Config sync data  | Server compromise     | E2E encryption (NaCl secretbox), server cannot decrypt    |
| Plugin code       | Malicious plugin      | WASM sandbox, no filesystem/network access, 100ms timeout |
| AI data           | Accidental data leak  | AI is local-only by default, no telemetry, no cloud calls |
| SSH keys          | Key theft             | OS keyring integration, keys never leave the machine      |

### E2E Encryption for Sync

- Encryption key generated locally on first run
- Key never sent to sync server
- Uses NaCl `crypto_secretbox` (XSalsa20-Poly1305)
- Server stores only encrypted blobs

### Plugin Sandbox

- WASM modules compiled to `wasm32-wasi`
- No filesystem access (WASI preopens are empty)
- No network access (no socket imports)
- Execution timeout: 100ms per call
- Memory limit: 64MB per plugin

## Testing Strategy

### VT Parser

- **Unicode Terminal Test Suite**: Full pass required before any release
- **Escape sequence conformance**: Test against xterm ctlseqs documentation
- **Fuzzing**: `cargo fuzz` on the parser with random byte streams
- **Regression tests**: Snapshot tests for known escape sequences

### Rendering

- **Pixel tests**: Compare rendered output against reference images
- **Performance tests**: 4K resolution, 120 FPS, measure frame time
- **Memory tests**: Verify < 50MB RAM at idle

### Cross-platform

- **CI matrix**: macOS, Windows, Ubuntu, Fedora
- **Wayland/X11**: Test both on Linux
- **ConPTY/WinPTY**: Test on Windows 10 and 11

## Deployment

### Native Installers

- **macOS**: `.dmg` + `.app` bundle, Sparkle auto-updates
- **Windows**: `.msi` installer, Squirrel auto-updates
- **Linux**: `.deb`, `.rpm`, Flatpak, AppImage

### Build System

```toml
# Cargo workspace
[workspace]
members = [
    "crates/zeroterm-core",
    "crates/zeroterm-render",
    "crates/zeroterm-mux",
    "crates/zeroterm-config",
    "crates/zeroterm-ai",
    "crates/zeroterm-sync",
    "crates/zeroterm",
]
```

### Dependencies (Phase 1 MVP)

```toml
[dependencies]
winit = "0.29"
wgpu = "21"
tokio = { version = "1", features = ["full"] }
portable-pty = "0.8"
swash = "0.2"
unicode-width = "0.1"
unicode-segmentation = "1"
toml = "0.8"
```
