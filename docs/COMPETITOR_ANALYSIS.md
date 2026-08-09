# Competitor Analysis — Terminal Emulator Landscape (2024–2026)

Research into how the major modern terminal emulators are designed: their tab
layout, rendering model, core architecture, and signature features. Written to
inform ZeroTerm's direction — see [FEATURE_OPPORTUNITIES.md](FEATURE_OPPORTUNITIES.md)
for the concrete "what to build next" list derived from this.

## Comparison at a Glance

| Terminal | Lang / Backend | Tab model | Multiplexing | Renderer | Signature |
| --- | --- | --- | --- | --- | --- |
| **Kitty** | C, OpenGL | In-window tab strip | Server/client daemon | GPU (OpenGL) | Kitty graphics + keyboard protocols, hyperlinked kittens, remote control (`kitty @`) |
| **WezTerm** | Rust, wgpu | In-window tab strip | **Mux server/client** (tmux-style detach) | GPU (OpenGL/WebGPU/Metal) | Domain model (local/SSH/WSL/TLS), local echo, per-pane working dirs |
| **Ghostty** | Zig, Metal/OpenGL | In-window tabs + **tab overview grid** | None built-in (tmux adapter) | GPU | Platform-native UI, runtime config reload, tab overview, minimal config |
| **Alacritty** | Rust, OpenGL/WebGPU | **No tabs** (relies on WM) | None | GPU | Speed + simplicity; `alacritty msg` single-instance control |
| **Rio** | Rust, WebGPU | In-window tabs | Server/client | GPU | Focus on aesthetics, themes, kitty protocols |
| **Contour** | C++, OpenGL | In-window tabs | Optional terminal multiplexer | GPU | Terminal multiplexer (`contour terminal`), kitty protocols, sixel |
| **foot** | C, Wayland | **No tabs** (WM-managed) | None | GPU (shm) | Wayland-native, minimal, fast; OSC 133 shell integration |
| **Warp** | Rust, GPU | In-window tabs | None (proprietary cloud) | GPU | **Blocks** (command/output grouping), command palette, workflows |
| **iTerm2** | ObjC, Metal | In-window tabs | tmux integration | GPU | Tmux integration, image protocols, triggers, hotkey windows |

## Layout & Design Elements

### Tab Bars

- **Kitty / WezTerm / iTerm2 / Rio / Contour / Warp** draw their own tab strip
  **inside** the window, above the terminal grid. Tabs are separate windows in
  the underlying toolkit; the strip is a chrome element layered over the
  surface.
- **Ghostty** also draws in-window tabs but adds a **tab overview** (an
  Exposé-style grid of all tabs/splits, opened with a hotkey) — the standout
  navigation innovation of the current generation.
- **Alacritty / foot** deliberately ship **no tab bar**; tabs are delegated to
  the window manager (sway/awesome/i3 tabs). This keeps the emulator surface
  pure but pushes UX onto the WM.
- Common strip anatomy: leftmost tab = current position indicator, each tab is
  a **pill/label** (icon or title), active tab highlighted, hover shows a
  close button. WezTerm and Kitty allow tab **reordering** by drag; Warp
  colors tabs by status (running/error).

### Split Panes

- All tiling terminals (Kitty, WezTerm, Ghostty, iTerm2, Contour) use a
  **recursive binary split tree**: each pane is a leaf, splits combine two
  subtrees horizontally or vertically. ZeroTerm's `SplitNode` is the same
  model.
- **WezTerm** extends panes with **per-pane working directory, zoom (maximize
  a pane), and rotation** (rotates split axis).
- **Ghostty** adds a split **resize overlay** (hold modifier, see live
  percentages) and **split navigation rings** — improvements to the resize
  divider UX ZeroTerm currently has.
- Warp and iTerm2 make splits *visual*: Warp animates pane boundaries,
  iTerm2 draws a **tint + border** around the focused pane.

### Status Line / Chrome

- **Warp** has a persistent bottom **status bar** with shell integration info
  (git branch, cwd) — the closest analog to ZeroTerm's status bar.
- **WezTerm** shows per-pane status via optional **left/right status
  components** (battery, git, time) through Lua.
- **iTerm2** bottom status bar shows badges; Kitty shows a **tab bar only**.
- Most terminals show scroll position implicitly via the scrollbar or a
  percentage in the tab strip; ZeroTerm's `[n%]` right-aligned status text
  matches this.

### Cursor, Fonts, and Text

- **Cursor**: block, beam (I-bar), and underline are universal; most now
  support cursor **blink intervals** and **per-mode shapes** (beam in insert,
  block in normal — Kitty/WezTerm/Ghostty all do this).
- **Fonts**: ligatures (FiraCode, JetBrains Mono) via **font shaping** are
  table stakes (Kitty, WezTerm, Ghostty, Alacritty ≥ 0.13, Rio). ZeroTerm
  currently renders monospace bitmaps with no shaping.
- **Ghostty** does *ligature-aware* shaping, **WezTerm** adds per-font
  fallback chains, **Kitty** has box-drawing/unicode-width correctness as a
  design goal.

### Themes & Appearance

- **Kitty** ships a theme repo + `kitten themes` picker; **WezTerm**
  `wezterm ls-fonts`/schemes; **Rio** markets itself on aesthetics
  (integrated themes, color adjustments); **Warp** ships a polished default
  theme set. All support **opacity + background blur** (Kitty & WezTerm:
  `background_opacity`, Ghostty: `background-opacity`). ZeroTerm already has
  opacity cycling; blur exists as a renderer pass.

## Core Structures & Architecture

### Rendering

- **GPU instanced quads** are the industry standard: Kitty (OpenGL), WezTerm
  (OpenGL/WebGPU/Metal), Rio/ZeroTerm (wgpu), Ghostty (Metal/OpenGL),
  Alacritty (OpenGL/WebGPU). Glyphs are rasterized to an **atlas texture**
  and each cell is a quad with UV + fg/bg + attribute flags. ZeroTerm's
  renderer is architecturally in the mainstream; **font shaping** is the main
  gap.
- **Terminal multiplexing**: WezTerm is the architectural leader — a
  background **mux server** owns all PTYs, the GUI is a *client* that can
  detach and reattach. Kitty has a similar single-instance daemon (`kitty @`).
  Alacritty and Ghostty run one PTY per process; Alacritty adds a control
  socket (`alacritty msg`). ZeroTerm is currently single-process (matches
  Ghostty/Alacritty); a mux daemon is the largest structural upgrade
  available.

### Protocols

| Protocol | What it does | Status in ZeroTerm |
| --- | --- | --- |
| **Kitty graphics** (`ESC _ G…`) | Streaming images as z-indexed placements; GPU-cached | ✅ implemented |
| **Sixel** | Legacy sixel bitmap images | ✅ implemented |
| **iTerm2 inline images** (OSC 1337) | Inline images | ✅ implemented |
| **Kitty keyboard** (CSI u) | Full modifier/key reporting | ❌ not implemented |
| **OSC 8 hyperlinks** | Clickable links in output | ⚠️ detection only, no click |
| **OSC 9 / OSC 777** | Desktop notifications | ❌ not implemented |
| **OSC 133 shell integration** | Semantic command boundaries, cwd, prompt marks | ⚠️ partial (block detection is heuristic, not OSC 133) |
| **Bracketed paste** | Readline-style paste protection | ✅ implemented |
| **Mouse reporting** (SGR 1000–1003) | App mouse tracking | ✅ implemented |

## Behavior Notes

- **Focus-follow / hover**: WezTerm and Kitty have hover-focus options;
  Ghostty focuses panes on hover. ZeroTerm has focus-follow-on-hover gated by
  config.
- **Session persistence**: WezTerm (mux), iTerm2 (tmux integration), Kitty
  (`kitty @` session save) persist full layouts. ZeroTerm's `layout.json`
  opt-in restore is the same idea without a daemon.
- **Scrollback**: 10k lines is common (ZeroTerm). WezTerm allows unlimited;
  Kitty keeps a per-window ring. **Search in scrollback** with regex
  highlighting is universal (ZeroTerm has it); **jump-to-block** (Warp's
  semantic navigation) is a differentiator ZeroTerm already has via
  `Ctrl+Shift+J/K`.
- **Notifications**: Warp, Kitty (`notify-on-completion`), and iTerm2
  (alerts) notify when a long-running command finishes. ZeroTerm lacks this.

## Sources

- Kitty docs: https://sw.kovidgoyal.net/kitty/
- WezTerm docs: https://wezterm.org/
- Ghostty: https://ghostty.org/
- Alacritty: https://alacritty.org/
- foot: https://codeberg.org/dnkl/foot
- Warp: https://www.warp.dev/
- iTerm2: https://iterm2.com/
