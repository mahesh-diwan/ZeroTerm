# ZeroTerm — Product Specification

## Name

**ZeroTerm** (pronounced /ˈzɪəroʊ tɜːrm/)

### Why "ZeroTerm"

ZeroTerm = _zero_ + _terminal_ — the terminal with zero compromises.

This maps directly to what the terminal does:

- **Zero latency** — GPU-accelerated, 120 FPS, sub-frame input latency
- **Zero bloat** — < 50MB RAM, no Electron, no webview, native code
- **Zero config** — Works out of the box; TOML/Lua/GUI for when you want it
- **Zero cloud** — AI is local (Ollama/LM Studio); sync is self-hosted E2EE
- **Zero tools** — Native tabs/splits/session restore replaces tmux/screen

### Availability

- GitHub: `zeroterm` — available
- crates.io: `zeroterm` — available
- npm: `zeroterm` — available
- PyPI: placeholder only
- Domain: `zeroterm.com`, `zeroterm.run`, `zeroterm.dev` — available
- No conflicts in tech

### Alternatives Considered

| Name         | Pros                                                       | Cons                     |
| ------------ | ---------------------------------------------------------- | ------------------------ |
| **ZeroTerm** | Zero latency/bloat/config/cloud/tools — philosophy matches | None                     |
| SwiftTerm    | Directly says "fast terminal"                              | Less philosophical depth |
| ClearTerm    | Says "clear rendering"                                     | Narrower scope           |
| OneTerm      | One terminal for everything                                | "One" is overused        |
| TermV        | Short, CLI-friendly                                        | Too cryptic              |

---

## What It Is

A next-generation, GPU-accelerated terminal emulator built in Rust.

ZeroTerm synthesizes the best features from every modern terminal into a single, coherent experience — without forcing users to choose between speed and features, or to configure a dozen plugins to get basic functionality.

## What It Does

| Category           | Feature                                                                       |
| ------------------ | ----------------------------------------------------------------------------- |
| **Rendering**      | GPU-accelerated via wgpu (Metal/DX12/Vulkan), 120 FPS, sub-50MB RAM           |
| **Multiplexing**   | Native tabs, splits (tiling tree), floating windows, session restore          |
| **Remote**         | SSH integration with daemon mode (disconnects don't kill sessions)            |
| **Graphics**       | Kitty + Sixel + iTerm2 inline image protocols, rendered to GPU textures       |
| **Modern UX**      | Block-based output with command metadata (exit code, duration, timestamp)     |
| **Input**          | Multi-line prompt editor with syntax highlighting, proper selection           |
| **AI**             | Optional, local-only (Ollama/LM Studio). No cloud required.                   |
| **Sync**           | End-to-end encrypted, self-hostable settings/hosts sync                       |
| **Config**         | GUI settings panel (beginners) + TOML (power users) + Lua (hackers)           |
| **Cross-platform** | macOS (Metal + native titlebar), Windows (DX12 + ConPTY), Linux (Wayland/X11) |

## What It's Really For

### Primary Audience

**Developers who refuse to choose.** Today's terminal landscape forces a tradeoff:

- Alacritty: Fast but no tabs/splits, no graphics, no sync
- WezTerm: Feature-rich but complex config, slower startup
- Kitty: Great graphics but no native tabs, Linux-focused
- Warp: Modern UX but cloud-locked, macOS-only
- Ghostty: Native feel but no Windows, no sync

ZeroTerm eliminates this tradeoff. You get Alacritty's speed, WezTerm's features, Kitty's graphics, Warp's UX, and Ghostty's native feel — all in one app, all cross-platform, all with zero-config defaults.

### Secondary Audiences

- **Teams** that need settings/hosts sync but want it self-hostable and E2E encrypted (not gated behind a paywall)
- **Sysadmins** who work across macOS/Windows/Linux and want consistent behavior
- **AI-curious developers** who want optional local AI assistance without cloud dependencies
- **Terminal minimalists** who want modern features without bloat (everything optional, everything fast)

## Positioning Statement

> ZeroTerm is the terminal that doesn't make you choose. It's the fastest terminal emulator with the most features, built for developers who work across platforms and demand both speed and modern UX — without cloud lock-in.

## Non-Goals

- **Not a webview app** — no Electron, no Tauri, no web rendering layer
- **Not cloud-first** — AI and sync are optional and self-hostable
- **Not a framework** — this is an end-user terminal application, not a library for building terminals
- **Not a reimplementation of tmux** — multiplexing is built-in but simpler and more integrated
- **Not a toy** — production-grade from day one, tested against Unicode terminal test suites

## Success Metrics

| Metric                | Target                                                     |
| --------------------- | ---------------------------------------------------------- |
| Startup time          | < 200ms (cold), < 50ms (warm)                              |
| Memory usage          | < 50MB idle                                                |
| Frame rate            | 120 FPS sustained at 4K                                    |
| Unicode test suite    | 100% pass rate                                             |
| Cross-platform parity | Identical behavior on macOS/Windows/Linux                  |
| Config complexity     | 0 steps for basic use, < 5 lines for common customizations |

## User Stories

1. **As a** developer switching between macOS and Linux, **I want** identical terminal behavior on both platforms, **so that** my muscle memory works everywhere.
2. **As a** developer who uses tmux, **I want** native tabs and splits that don't require a separate tool, **so that** I can manage sessions without learning tmux.
3. **As a** developer who wants AI assistance, **I want** optional local AI via Ollama, **so that** I get help without sending my code to the cloud.
4. **As a** team lead, **I want** end-to-end encrypted settings sync that I can self-host, **so that** my team's configs stay consistent without trusting a third party.
5. **As a** power user, **I want** a GUI settings panel for common options and TOML/Lua for advanced config, **so that** I can choose my level of complexity.

## Edge Cases

- **Windows 10 (pre-20H2)**: ConPTY not available — fall back to WinPTY with degraded but functional experience
- **Linux without GPU**: Fall back to software rendering via wgpu's CPU backend
- **macOS without Metal**: Fall back to OpenGL (deprecated but functional on older Macs)
- **Network partitions**: Local config always works; sync resumes when connectivity restored
- **AI service down**: Terminal continues to work normally; AI features simply unavailable
- **Large scrollback**: Compressed (zstd) to disk, hot region in RAM — no OOM crashes

## License

MIT — permissive, patent-safe, encourages community contribution and commercial adoption.
