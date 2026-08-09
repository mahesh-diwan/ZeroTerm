# Feature Opportunities — What to ADD Next

Derived from the competitor landscape in [COMPETITOR_ANALYSIS.md](COMPETITOR_ANALYSIS.md).
This list is about **new capabilities**, not polishing existing ones. Items are
ranked by (impact × feasibility) for a GPU-accelerated Rust terminal with
ZeroTerm's existing architecture.

## Tier 1 — High impact, fits current architecture

### 1. Kitty keyboard protocol (CSI u) — `SEND`
**What:** Report full key/modifier state (`ESC [ <modifier>;<key> u`) so apps
like Neovim can disambiguate Ctrl+I vs Tab, Shift+letter, etc.
**Why:** The single most-requested protocol gap in modern terminals (Kitty,
WezTerm, Ghostty, foot, Contour all support it). ZeroTerm already has the
kitty graphics protocol, so this completes the kitty pair.
**How:** Parser flag + a key-sequence encoder in `key_router.rs` that switches
to CSI-u reporting when the app requests `CSI > 1 u`; app must advertise with
`CSI ? u`. Pure encoder + parser tests — same pattern as `key_sequence`.

### 2. OSC 8 hyperlinks — click to open
**What:** Render OSC 8 hyperlinks (`ESC ]8;;URL ESC \`) as hoverable,
clickable anchors; `Ctrl+Click` opens in the browser, hover shows the URL.
**Why:** Modern `ls --hyperlink`, `grep`, and language servers emit OSC 8.
ZeroTerm already **detects** URLs; this makes them actionable. Small delta on
existing infra: store link range in the cell, hit-test in the mouse arm, open
via `xdg-open`/`open`.
**Also:** clickable **file paths** → open in `$EDITOR` (WezTerm/Kitty do this
with shell integration).

### 3. Desktop notifications — OSC 9 / OSC 777
**What:** When a pane is in the background and a long-running command finishes,
fire a native notification (`ESC ]9;message ESC \`). Config threshold
(e.g. after 5s of silence), opt-in.
**Why:** Universal in serious terminals (Kitty `notify-on-completion`, Warp,
iTerm2). Cheap to add: detect block exit in `drain_pty` → spawn
`notify-send`/`osascript` — no new crates.

### 4. OSC 133 shell integration (semantic blocks)
**What:** Adopt the standard shell-integration protocol (OSC 133 A/B/C) so
command boundaries, exit status, and cwd are **exact** instead of heuristic.
Ship a shell snippet (like starship_setup already does) that emits the
sequences; parser consumes them to fix block boundaries, enable cwd-aware
links, and power per-block metadata.
**Why:** ZeroTerm's block detection is prompt-sigil heuristics today; OSC 133
is the industry answer (foot, WezTerm, iTerm2, Warp). It directly hardens an
existing differentiator.

### 5. Ligatures + font shaping
**What:** Use `swash`'s shaping (already a dependency!) to render ligatures
(→, ⇒, `!=`) and correct combining marks. ZeroTerm's atlas currently
rasterizes single glyphs; swash's `ShapeContext` gives run-level shaping.
**Why:** Table stakes for a "modern" terminal (Kitty, WezTerm, Ghostty,
Alacritty ≥0.13). Improves perceived quality more than anything else visual.
**Risk:** Medium — shaping changes cell-width accounting; gate behind a
`[font] ligatures = true` config defaulting to on.

### 6. Tab overview grid (Ghostty-style)
**What:** A hotkey (`Ctrl+Shift+O` conflict — pick `Ctrl+Shift+Tab` long-press
or `Alt+O`) that shows all tabs as a tiled grid of miniature screens; arrows
navigate, Enter selects. ZeroTerm already has a floating-pane overlay + a
`ScreenScratch` snapshot mechanism — the overview can reuse it.
**Why:** Ghostty's most-praised navigation feature; direct fit for ZeroTerm's
GPU renderer (miniature screens are just scaled-down `render_screen` calls).

## Tier 2 — Medium impact, more work

### 7. Single-instance daemon + remote control (`zeroterm msg`)
**What:** One background instance owns the session; `zeroterm msg new-tab`
opens in it (alacritty/kitty model). Enables `zeroterm msg dump-screen`,
`set-opacity`, split by CLI.
**Why:** Scriptability (CI, editor integrations) is a real differentiator; a
slim `--socket` + JSON command protocol reuses `SessionLayout` and the
existing command routing. This is also the stepping stone to Tier-1
multiplexing.

### 8. Mux server / client (WezTerm-style detach)
**What:** Move PTY ownership into a `zeroterm-muxd` background process; the
GUI becomes a client that can disconnect and reattach (`zeroterm attach`).
**Why:** The most architecturally significant upgrade available — closes the
window, keeps the session, reattach anywhere. Reuses the existing
`SessionLayout` restore work (which already round-trips tabs/splits).
**Risk:** Large. Recommend building #7 first and evolving it.

### 9. Per-pane working directory + cwd-aware features
**What:** Track cwd per pane via OSC 7 (`ESC ]7;file://host/path`) and show
it in the tab/status bar; enable "open file here" and per-tab cwd restore.
**Why:** WezTerm/Warp sell this hard; OSC 7 is trivial to emit from the shell
snippet in #4. Medium work (parser OSC 7 handling + per-pane cwd field +
UI).

### 10. Command palette (Warp-style, terminal-native)
**What:** `Ctrl+Shift+P` (or a new chord) opens a fuzzy-searchable palette of
**actions** (new tab, split, copy, settings, plugins) plus **history**
(run any previous command). Purely in-app — no cloud.
**Why:** Warp's most-loved feature is the palette; a Rust fuzzy filter
over the existing `GlobalAction` enum + command history is very tractable.

### 11. SGR mouse + hover URL tooltip, and kitty keyboard for mouse too
**What:** Extend mouse reporting to the kitty protocol so apps get precise
events; show URL tooltips on hover for OSC 8 links.
**Why:** Rounds out protocol support; small incremental work over #1/#2.

## Tier 3 — Nice-to-have, larger scope

### 12. Font fallback chains + emoji
**What:** Per-glyph fallback across installed fonts (WezTerm `font: with_fallbacks`)
and color emoji (COLR/SBIX) rendering.
**Why:** Cosmetic but commonly requested; needs a font-matching layer over the
atlas.

### 13. True scrollback search across sessions + global search
**What:** Search across all tabs at once; jump-to-match switches tabs.
**Why:** iTerm2/WezTerm have it; ZeroTerm's per-screen search is a natural
stepping stone.

### 14. Split-pane rotation, zoom, and reordering
**What:** WezTerm-style pane zoom (maximize/restore), rotate axes, drag to
reorder tabs, drag dividers with live percentage overlay (Ghostty).
**Why:** Multiplexing UX polish; the tree model already supports it.

### 15. Trigger / alert system (iTerm2-style)
**What:** Configurable regex triggers that flash the tab, notify, or run a
command when matched in output (build failures, "tests passed").
**Why:** Power-user feature; pairs with #3 notifications.

### 16. Remote shell integration for SSH sessions
**What:** Run the OSC 133/7 shell snippet on the remote side of `connect_ssh`
so blocks and cwd work over SSH too.
**Why:** SSH is a ZeroTerm differentiator; shell integration currently only
works locally.

## Suggested order

1. **OSC 8 hyperlinks + click-to-open** (small, high value)
2. **Kitty keyboard protocol** (protocol completeness, pairs with existing
   kitty graphics)
3. **OSC 9 notifications** (small, delight)
4. **OSC 133 + OSC 7 shell integration** (hardens blocks, unlocks cwd)
5. **Ligatures via swash shaping** (visual quality)
6. **Tab overview grid** (navigation differentiator)
7. **Command palette** (productivity)
8. **Single-instance daemon → mux client** (architecture)

## What we deliberately do NOT add

- **AI/LLM features** — removed from the product by decision. Warp's AI is
  proprietary/cloud-bound; the terminal-native alternatives (blocks, palette,
  notifications) deliver the productivity without a network dependency.
- **Cloud sync/team features** — Warp Drive-style features require a service;
  out of scope for a local-first tool.
