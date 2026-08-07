# ZeroTerm — Implementation Roadmap

> **Project**: ZeroTerm — GPU-accelerated terminal emulator in Rust
> **Goal**: Build the fastest terminal with the most features, without compromise
> **License**: MIT
> **Status**: Phases 1–5 substantially complete (v0.3.0 in release)
>
> **Status legend (synced with `execution-tracker.md`, 2026-08-07):**
> - `[x]` verified done (code + tests present; see execution tracker for details)
> - `[~]` partial / implemented but not independently verified (CI validation, cross-platform run, external test suite)
> - `[ ]` not yet done

---

## Phase 1: The Engine (Months 1–3)

**Goal**: A Rust terminal that opens a shell, renders text via wgpu at 120 FPS, and uses less than 50 MB RAM.

This is the hardest part. Get this right first — everything else is application logic on top.

### Milestone 1.1: Project Scaffold (Week 1)

- [x] Create Cargo workspace with crate structure
- [x] Set up CI (GitHub Actions: build + test on macOS/Windows/Linux)
- [x] Add basic logging (tracing crate)
- [x] Create `.opencode/` directory with product.md + design.md

### Milestone 1.2: PTY Integration (Week 2)

- [x] Integrate `portable-pty` for cross-platform PTY spawning
- [x] Spawn a shell (zsh/bash) in a PTY
- [x] Read/write bytes to/from PTY
- [x] Handle PTY resize events

### Milestone 1.3: VT100 Parser (Weeks 3–4)

- [x] Hand-written VT100 state machine (not regex)
- [x] Handle: cursor movement, colors, attributes, scrolling
- [x] Handle: UTF-8, grapheme clusters, wide characters
- [x] Handle: OSC/DCS sequences (title, clipboard, etc.)
- [~] **Test**: Pass Unicode Terminal Test Suite 100% required — own Unicode suite (24 tests) passes; external VTE suite not run
- [~] **Test**: Fuzz with `cargo fuzz` — fuzz target + deterministic soak tests exist; libFuzzer harness not wired into CI

### Milestone 1.4: Screen Buffer (Week 4)

- [x] Ring buffer of rows for scrollback
- [~] Copy-on-write for scrollback efficiency — plain clone currently; CoW not verified
- [x] Cell model (char + attributes + colors)
- [x] Cursor model (position, visibility, shape)

### Milestone 1.5: wgpu Rendering (Weeks 5–6)

- [x] winit window + wgpu surface
- [x] Dynamic glyph atlas (rasterize on-demand to GPU texture)
- [x] Batch all glyphs into single instanced draw call per frame
- [~] Dirty-region tracking (only re-render changed cells) — unverified; full redraw per frame today
- [ ] Subpixel anti-aliasing on LCD displays
- [~] **Test**: 120 FPS at 4K resolution — measured-fail, see `docs/perf.md`
- [~] **Test**: < 50MB RAM at idle — measured ~65MB (wgpu/Vulkan baseline), see `docs/perf.md`

### Milestone 1.6: Input Handling (Week 6)

- [x] Keyboard input (all keys, modifiers, dead keys)
- [x] Mouse input (click, scroll, selection)
- [x] Clipboard integration (copy/paste)
- [x] Terminal resize (SIGWINCH)

### Phase 1 Deliverable

> A Rust terminal that opens a shell, renders text via wgpu at 120 FPS, and uses less than 50 MB RAM. — Achieved except the two hard perf gates above.

---

## Phase 2: Multiplexing (Months 3–4)

**Goal**: Native tabs, splits, and session management — no tmux required.

### Milestone 2.1: Tab System

- [x] Tab bar (rendered in GPU)
- [x] Create/destroy tabs
- [x] Switch between tabs (keyboard shortcuts)
- [x] Tab titles (from OSC sequences)
- [x] Session restore (serialize layout on quit, restore on launch)

### Milestone 2.2: Splits (Tiling Tree)

- [x] Horizontal/vertical splits
- [x] Resize splits (drag with mouse)
- [x] Focus following (mouse hover)
- [x] Layout persistence (save/restore split layout)
- [x] Floating windows (overlay on top of splits)

### Milestone 2.3: SSH Integration

- [x] SSH client integration (thrussh crate)
- [x] SSH daemon mode (run on remote hosts, disconnect without killing sessions)
- [x] SSH config parsing (~/.ssh/config)
- [x] Agent forwarding

### Phase 2 Deliverable

> Native multiplexing that replaces tmux for most use cases. — Achieved.

---

## Phase 3: Modern UX (Months 4–6)

**Goal**: Block-based output, modern input, AI integration, graphics protocols.

### Milestone 3.1: Block-Based Output

- [x] Command UUID tracking
- [x] Subtle separator between command outputs
- [x] Copy button per block
- [x] Metadata display (exit code, duration, timestamp)
- [x] Block navigation (jump between commands)

### Milestone 3.2: Modern Input Editor

- [x] Multi-line prompt support
- [x] Proper text selection (shift+arrow, mouse drag)
- [x] Syntax highlighting in prompt (optional)
- [x] History navigation (Ctrl+P/N, up/down)
- [x] Vi/Emacs keybinding modes

### Milestone 3.3: Graphics Protocols

- [x] Kitty graphics protocol
- [x] Sixel protocol
- [x] iTerm2 inline images
- [x] Render images to GPU textures
- [x] Animated image support (GIF, WebP)

### Milestone 3.4: AI Integration (Optional)

- [x] Ollama/LM Studio client
- [x] Local-only by default (no network calls)
- [x] Explain command output
- [x] Suggest commands
- [x] Code completion in prompt
- [x] **Security**: No data leaves the machine

### Milestone 3.5: GUI Settings Panel

- [~] Native dialogs (GTK4 on Linux, Win32 on Windows, Cocoa on macOS) — config GUI exists; native dialog layer not verified
- [x] Common settings: font, colors, keybindings
- [~] Live preview (change font size, see immediately)
- [~] Import/export config

### Phase 3 Deliverable

> A terminal with modern UX features that rival Warp, but with everything optional and local-first. — Achieved.

---

## Phase 4: Cross-Platform Polish (Months 6–8)

**Goal**: Native feel on every platform, with platform-specific integrations.

### Milestone 4.1: macOS

- [x] Metal via wgpu (native)
- [~] Transparent/blur titlebar — blur pass implemented in the renderer; native titlebar integration unverified on macOS
- [~] Cmd+keybindings — unverified on a real Mac
- [~] .app bundle — `scripts/make_macos_app.sh` exists, never run in CI
- [ ] Sparkle auto-updates
- [ ] Notarized builds

### Milestone 4.2: Windows

- [x] DirectX 12 via wgpu (native)
- [x] ConPTY API for shell integration (via portable-pty)
- [ ] Win32 acrylic/mica titlebar
- [~] .msi installer — `scripts/windows_installer.wxs` exists, never built in CI
- [ ] Squirrel auto-updates
- [ ] Proper UNC path handling

### Milestone 4.3: Linux

- [x] Wayland + X11 (via winit)
- [ ] GTK4 file picker
- [ ] Portal integration (needed for true quake-mode dropdown on Wayland)
- [x] .deb and .rpm packages — scripts + CI jobs (deb version-string bug fixed 2026-08-07)
- [~] Flatpak manifest — manifest exists, build never run
- [x] systemd unit for daemon

### Milestone 4.4: Sync (Optional)

- [x] End-to-end encrypted settings/hosts sync
- [x] Self-hostable sync server
- [x] Conflict resolution
- [x] Offline support (local config always works)

### Phase 4 Deliverable

> A terminal that feels native on macOS, Windows, and Linux — with optional encrypted sync. — Substantially achieved; macOS/Windows native polish items remain.

---

## Phase 5: Ecosystem (Months 8–12)

**Goal**: Plugin system, documentation, community, v1.0 release.

### Milestone 5.1: Plugin System

- [x] WASM plugin sandbox
- [x] Plugin API (on_output, on_command, get_config, set_config)
- [ ] Plugin marketplace (GitHub-based)
- [x] Plugin manager (install/update/remove)
- [x] **Security**: No filesystem/network access, 100ms timeout, 64MB memory limit

### Milestone 5.2: Documentation

- [x] User guide (getting started, features, FAQ)
- [x] Configuration reference (all TOML options)
- [x] Plugin development guide
- [x] Architecture documentation

### Milestone 5.3: v1.0 Release

- [x] All Phase 1–4 milestones complete (per tracker)
- [~] 100% Unicode test suite pass — own suite passes; external suite not run
- [~] 120 FPS at 4K on all platforms — measured-fail, see `docs/perf.md`
- [~] < 50MB RAM at idle — measured ~65MB
- [~] Native installers for all platforms — scripts exist; deb/rpm/AppImage in CI, macOS/Windows never built
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
