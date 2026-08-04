# ZeroTerm Architecture

GPU-accelerated terminal emulator in Rust. Workspace of 9 crates under
`crates/`, a single binary in `crates/zeroterm`.

## Crate Layout

```
crates/
├── zeroterm-core      — VT100/ANSI parser, screen buffer, cell model, PTY abstraction
├── zeroterm-render    — wgpu renderer, glyph atlas, block divider overlays
├── zeroterm-mux       — tab / pane / split-tree model
├── zeroterm-config    — TOML + Lua config loading
├── zeroterm-ai        — Ollama/LM Studio client (explain)
├── zeroterm-sync      — E2E encrypted settings sync daemon
├── zeroterm-ssh       — native SSH client (libssh2)
├── zeroterm-plugin    — wasmtime/WASI sandboxed plugin runtime (stdio ABI)
└── zeroterm           — main binary: event loop, PTY threads, wiring
```

Dependency direction is one-way: `zeroterm` imports everything; `render`
imports `core` and `config`; nothing depends on `zeroterm`.

## Data Flow

```
PTY child process
   │  read() 4096-byte chunks
   ▼
spawn_pty_process thread (core::pty)   ── mpsc::channel ──►  main.rs drain_pty()
                                                                   │
   spawn_ssh_process thread (zeroterm-ssh)  ── mpsc ──────────────┤
                                                                   ▼
                                             Parser::parse(bytes)
                                                   │  ESC/CSI/OSC state machine
                                                   ▼
                                             Screen (buffer + scrollback)
                                                   │
                                             Renderer::render(screen)
                                                   │  update_cell_data()
                                                   ▼
                                             wgpu storage buffer (CellData[])
                                                   │
                                             vs_main/fs_main (WGSL)
                                                   │
                                             swapchain surface
```

- **PTY/SSH threads** own the blocking `read` loop and the `PtyCommand` receiver
  (Write / Resize / Kill). The GUI thread never blocks on I/O.
- **`drain_pty`** drains every pane's channel each redraw/keypress, feeding bytes
  straight into the parser.
- **Parser → Screen** is a single-pass hand-written state machine; only visible
  buffer and scrollback cells are stored.

## Instanced Rendering

`zeroterm-render` draws the whole grid with one draw call: 6 quad vertices ×
`cols*rows` instances. The vertex shader derives `col = ii % cols`,
`row = ii / cols` and looks up per-cell data from a GPU storage buffer
(`CellData`, POD via `bytemuck`):

- glyph UVs + size (from the atlas), fg/bg colors, and a `u32 attrs` bitmask.

`update_cell_data` rebuilds the buffer every frame (all cells dirty; ~<200KB
writes are free). The shader blends the glyph atlas alpha over the background,
then applies attribute effects:

| Bit   | Effect              |
| ----- | ------------------- |
| 0x1   | bold (fg ×1.2)      |
| 0x2   | italic (reserved)   |
| 0x4   | underline           |
| 0x8   | strikethrough       |
| 0x10  | dim (fg ×0.7)       |
| 0x20  | blink (reserved)    |
| 0x40  | reverse video       |
| 0x80  | invisible           |
| 0x100 | bar cursor          |
| 0x200 | selection highlight |
| 0x400 | kitty image blit    |
| 0x800 | block divider tint  |

Glyphs are packed into a 1024² atlas on demand (`swash` rasterizer); ASCII is
pre-packed at init. Cell metrics come from font ascent/descent/leading.

## Rendering Pipeline

`zeroterm-render` draws the frame as a sequence of passes, each ending in a
`end_frame` submit/present. The GUI thread calls these in order every redraw:

```
draw_background(color)   — clear the whole surface to the background color
render_screen(screen, …) — the instanced cell grid (the pass above)
draw_tab_bar(tabs)       — tab strip with active/hover states + close buttons
draw_status_bar(left, right) — bottom status line
draw_scrollbar(screen, …) — overlay scrollbar thumb/track
end_frame()              — submit the pass command buffers and present
```

Every pass is a separate wgpu render pass with its own shader bindings, so the
bar passes (tab/status/scrollbar) reuse the same glyph pipeline with their own
vertex data. Each pass sets a per-pass `viewport_origin` uniform (`[f32; 2]`,
`renderer.rs`); for the grid pass it is the scrolled window's origin in cell
space plus padding, letting `render_screen` draw only the visible window of the
scrollback. `render_screen` is the only pass that reads `Screen` state; the UI
bar passes take plain slices (`&[TabInfo]`, `&str`).

## Block Tracking

The parser recognizes command boundaries: when a line starts with a prompt
sigil (`$ % # >`) after a newline, `Screen::mark_block_boundary()` closes the
previous block (setting `end_line` and `duration_ms` from its timestamp) and
opens a new `CommandBlock`.

```
CommandBlock { id, start_line, end_line, command, exit_code, timestamp, duration_ms }
```

- `command` is captured from the prompt line; `set_block_exit_code()` is the
  hook for shell integration to record status.
- `Screen::block_metadata()` renders `exit:0 · 123ms`.

The renderer builds a `HashSet` of `start_line` rows each frame; those rows get
`ATTR_BLOCK_DIVIDER`, a subtle lighten of the background, and an overlay:
right-aligned `[copy]` affordance with metadata beside it. The overlay replaces
only the drawn glyphs, never the buffer cells. Clicking a `[copy]` marker
(`main.rs::copy_block_output`) copies that block's buffer rows to the clipboard
via `arboard`.

> `start_line` is buffer-local, so dividers align with view rows only while
> `scroll_offset == 0`; scrolled dividers are best-effort.

## Split Tree

`zeroterm-mux::split::SplitNode` is a recursive tiling tree:

```rust
enum SplitNode {
    Leaf(pane_id),
    Split { dir: Vertical | Horizontal, children: Vec<SplitNode>, ratio: f32 },
}
```

`insert_leaf` / `remove_leaf` rebuild the tree (removal collapses single-child
splits); `compute_rects` distributes normalized rectangles via a single split-
step algorithm. Rectangles power `Alt+Arrow` focus navigation. Currently the
active pane renders fullscreen (`main.rs` keeps a per-pane viewport TODO); the
rects are used for focus, not yet per-pane viewports.

## Session Restore

`zeroterm/src/session.rs` persists pane titles and commands to `session.json`
(next to the config) on close. On init, the first pane is always the configured
shell; remaining saved panes are re-spawned as new PTYs. Restore failure is
non-fatal (warn and continue).

## Auxiliary Crates

- **zeroterm-config** — loads TOML, then evaluates `.zeroterm.lua` in a
  sandboxed VM (`io`/`require`/`package`/`debug` stripped, safe `os`) and applies
  `set(key, value)` overrides. Hot-reloaded via a `notify` watcher thread.
- **zeroterm-ai** — `POST /api/generate` to Ollama-compatible endpoints with
  `model = llama3.2`, streaming disabled. `Ctrl+Shift+I` feeds the raw screen
  text to `explain()` on a background tokio runtime and writes the reply to the
  PTY.
- **zeroterm-sync** — `SyncDaemon` marks config dirty every 300 redraws; a tokio
  task pulls `GET /api/sync/latest` and pushes `POST /api/sync`, encrypting the
  serialized config with a per-launch ChaCha20-Poly1305 key (nonce-prefixed).
- **zeroterm-ssh** — libssh2 (`ssh2`) session; password, key-file, or agent
  auth; execs `$SHELL` on a channel and bridges reads/writes/resizes to the same
  `PtyCommand` protocol as local PTYs.
- **zeroterm-plugin** — wasmtime + WASI (`wasm32-wasip1`) command modules. A
  plugin imports `wasi_snapshot_preview1`, exports `_start`, reads input from
  WASI stdin, writes its result to WASI stdout — no linear-memory pointer ABI.
  `PluginHost` owns the shared engine; each `.wasm` loads once and runs in a
  fresh `Store` per call (stateless). Bounds: `max_memory` (16 MiB), fixed fuel
  budget per call (infinite loops trap), optional read-only `wasi_dir` preopen,
  `max_output` (1 MiB) stdout cap. Loaded plugins run via `Ctrl+Shift+B`.
  See [PLUGIN_DEV_GUIDE.md](PLUGIN_DEV_GUIDE.md).
