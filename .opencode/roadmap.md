# ZeroTerm — Implementation Roadmap

> **Project**: ZeroTerm — GPU-accelerated terminal emulator in Rust
> **Goal**: Build the fastest terminal with the most features, without compromise
> **License**: MIT
> **Status**: Phase 1 (MVP)

---

## Phase 1: The Engine (Months 1–3)

**Goal**: A Rust terminal that opens a shell, renders text via wgpu at 120 FPS, and uses less than 50 MB RAM.

This is the hardest part. Get this right first — everything else is application logic on top.

### Milestone 1.1: Project Scaffold (Week 1)

- [ ] Create Cargo workspace with crate structure
- [ ] Set up CI (GitHub Actions: build + test on macOS/Windows/Linux)
- [ ] Add basic logging (tracing crate)
- [ ] Create `.opencode/` directory with product.md + design.md

### Milestone 1.2: PTY Integration (Week 2)

- [ ] Integrate `portable-pty` for cross-platform PTY spawning
- [ ] Spawn a shell (zsh/bash) in a PTY
- [ ] Read/write bytes to/from PTY
- [ ] Handle PTY resize events

### Milestone 1.3: VT100 Parser (Weeks 3–4)

- [ ] Hand-written VT100 state machine (not regex)
- [ ] Handle: cursor movement, colors, attributes, scrolling
- [ ] Handle: UTF-8, grapheme clusters, wide characters
- [ ] Handle: OSC/DCS sequences (title, clipboard, etc.)
- [ ] **Test**: Pass Unicode Terminal Test Suite (100% required)
- [ ] **Test**: Fuzz with `cargo fuzz` on random byte streams

### Milestone 1.4: Screen Buffer (Week 4)

- [ ] Ring buffer of rows for scrollback
- [ ] Copy-on-write for scrollback efficiency
- [ ] Cell model (char + attributes + colors)
- [ ] Cursor model (position, visibility, shape)

### Milestone 1.5: wgpu Rendering (Weeks 5–6)

- [ ] winit window + wgpu surface
- [ ] Dynamic glyph atlas (rasterize on-demand to GPU texture)
- [ ] Batch all glyphs into single instanced draw call per frame
- [ ] Dirty-region tracking (only re-render changed cells)
- [ ] Subpixel anti-aliasing on LCD displays
- [ ] **Test**: 120 FPS at 4K resolution
- [ ] **Test**: < 50MB RAM at idle

### Milestone 1.6: Input Handling (Week 6)

- [ ] Keyboard input (all keys, modifiers, dead keys)
- [ ] Mouse input (click, scroll, selection)
- [ ] Clipboard integration (copy/paste)
- [ ] Terminal resize (SIGWINCH)

### Phase 1 Deliverable

> A Rust terminal that opens a shell, renders text via wgpu at 120 FPS, and uses less than 50 MB RAM.

---

## Phase 2: Multiplexing (Months 3–4)

**Goal**: Native tabs, splits, and session management — no tmux required.

### Milestone 2.1: Tab System

- [ ] Tab bar (rendered in GPU)
- [ ] Create/destroy tabs
- [ ] Switch between tabs (keyboard shortcuts)
- [ ] Tab titles (from OSC sequences)
- [ ] Session restore (serialize layout on quit, restore on launch)

### Milestone 2.2: Splits (Tiling Tree)

- [ ] Horizontal/vertical splits
- [ ] Resize splits (drag with mouse)
- [ ] Focus following (mouse hover)
- [ ] Layout persistence (save/restore split layout)
- [ ] Floating windows (overlay on top of splits)

### Milestone 2.3: SSH Integration

- [ ] SSH client integration (thrussh crate)
- [ ] SSH daemon mode (run on remote hosts, disconnect without killing sessions)
- [ ] SSH config parsing (~/.ssh/config)
- [ ] Agent forwarding

### Phase 2 Deliverable

> Native multiplexing that replaces tmux for most use cases.

---

## Phase 3: Modern UX (Months 4–6)

**Goal**: Block-based output, modern input, AI integration, graphics protocols.

### Milestone 3.1: Block-Based Output

- [ ] Command UUID tracking
- [ ] Subtle separator between command outputs
- [ ] Copy button per block
- [ ] Metadata display (exit code, duration, timestamp)
- [ ] Block navigation (jump between commands)

### Milestone 3.2: Modern Input Editor

- [ ] Multi-line prompt support
- [ ] Proper text selection (shift+arrow, mouse drag)
- [ ] Syntax highlighting in prompt (optional)
- [ ] History navigation (Ctrl+P/N, up/down)
- [ ] Vi/Emacs keybinding modes

### Milestone 3.3: Graphics Protocols

- [ ] Kitty graphics protocol
- [ ] Sixel protocol
- [ ] iTerm2 inline images
- [ ] Render images to GPU textures
- [ ] Animated image support (GIF, WebP)

### Milestone 3.4: AI Integration (Optional)

- [ ] Ollama/LM Studio client
- [ ] Local-only by default (no network calls)
- [ ] Explain command output
- [ ] Suggest commands
- [ ] Code completion in prompt
- [ ] **Security**: No data leaves the machine

### Milestone 3.5: GUI Settings Panel

- [ ] Native dialogs (GTK4 on Linux, Win32 on Windows, Cocoa on macOS)
- [ ] Common settings: font, colors, keybindings
- [ ] Live preview (change font size, see immediately)
- [ ] Import/export config

### Phase 3 Deliverable

> A terminal with modern UX features that rival Warp, but with everything optional and local-first.

---

## Phase 4: Cross-Platform Polish (Months 6–8)

**Goal**: Native feel on every platform, with platform-specific integrations.

### Milestone 4.1: macOS

- [ ] Metal via wgpu (native)
- [ ] Transparent/blur titlebar
- [ ] Cmd+keybindings
- [ ] .app bundle
- [ ] Sparkle auto-updates
- [ ] Notarized builds

### Milestone 4.2: Windows

- [ ] DirectX 12 via wgpu (native)
- [ ] ConPTY API for shell integration
- [ ] Win32 acrylic/mica titlebar
- [ ] .msi installer
- [ ] Squirrel auto-updates
- [ ] Proper UNC path handling

### Milestone 4.3: Linux

- [ ] Wayland + X11 (via winit)
- [ ] GTK4 file picker
- [ ] Portal integration
- [ ] .deb and .rpm packages
- [ ] Flatpak manifest
- [ ] systemd unit for daemon

### Milestone 4.4: Sync (Optional)

- [ ] End-to-end encrypted settings/hosts sync
- [ ] Self-hostable sync server
- [ ] Conflict resolution
- [ ] Offline support (local config always works)

### Phase 4 Deliverable

> A terminal that feels native on macOS, Windows, and Linux — with optional encrypted sync.

---

## Phase 5: Ecosystem (Months 8–12)

**Goal**: Plugin system, documentation, community, v1.0 release.

### Milestone 5.1: Plugin System

- [ ] WASM plugin sandbox
- [ ] Plugin API (on_output, on_command, get_config, set_config)
- [ ] Plugin marketplace (GitHub-based)
- [ ] Plugin manager (install/update/remove)
- [ ] **Security**: No filesystem/network access, 100ms timeout, 64MB memory limit

### Milestone 5.2: Documentation

- [ ] User guide (getting started, features, FAQ)
- [ ] Configuration reference (all TOML options)
- [ ] Plugin development guide
- [ ] Architecture documentation

### Milestone 5.3: v1.0 Release

- [ ] All Phase 1–4 milestones complete
- [ ] 100% Unicode test suite pass
- [ ] 120 FPS at 4K on all platforms
- [ ] < 50MB RAM at idle
- [ ] Native installers for all platforms
- [ ] Auto-update system working
- [ ] Community feedback incorporated

---

## Critical Path

The critical path is **Phase 1** — specifically the VT100 parser and wgpu rendering. These are the hardest parts and everything else depends on them.

```
Phase 1 (3 months)
  ├── Project scaffold (1 week)
  ├── PTY integration (1 week)
  ├── VT100 parser (2 weeks) ← CRITICAL
  ├── Screen buffer (1 week)
  ├── wgpu rendering (2 weeks) ← CRITICAL
  └── Input handling (1 week)
```

If the VT100 parser or wgpu rendering proves intractable in the first 2 weeks, the recommendation is to contribute to an existing project (WezTerm or Alacritty) to learn the internals first.

## Resource Requirements

| Resource             | Requirement                                                |
| -------------------- | ---------------------------------------------------------- |
| **Developer time**   | 1 FTE for 8–14 months, or 2 FTE for 4–7 months             |
| **CI/CD**            | GitHub Actions (free for open source)                      |
| **Testing hardware** | macOS, Windows, Linux machines for cross-platform testing  |
| **Code signing**     | Apple Developer ID, Windows Code Signing Certificate       |
| **Domain**           | aether.run or aether.dev                                   |
| **Budget**           | $0–$500/month (domain, code signing, CI minutes if needed) |

## Success Criteria

| Metric            | Target                          | Measurement                 |
| ----------------- | ------------------------------- | --------------------------- |
| Startup time      | < 200ms cold, < 50ms warm       | `hyperfine`                 |
| Memory usage      | < 50MB idle                     | `ps` / Activity Monitor     |
| Frame rate        | 120 FPS at 4K                   | `wgpu` profiler             |
| Unicode tests     | 100% pass                       | Unicode Terminal Test Suite |
| Cross-platform    | Identical behavior              | Automated test matrix       |
| Config complexity | 0 steps basic, < 5 lines common | User testing                |
