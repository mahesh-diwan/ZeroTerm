# UX, UI, Error Handling & Rendering — Best Practices + ZeroTerm Gap Analysis

Companion to `COMPETITOR_ANALYSIS.md` (how other terminals work) and
`FEATURE_OPPORTUNITIES.md` (what to add). This doc is the **how to make it
feel right**: design practices, robustness patterns, rendering quality, and
per-command behavior — each mapped to ZeroTerm's current state and the exact
file/line that would change.

Sources: Ghostty/WezTerm/Kitty/iTerm2/Warp/Alacritty docs and 2025-2026
terminal-engineering discourse (instanced-quad GPU rendering, OSC 133/7 shell
integration, SGR mouse, synchronized output, SDF vs bitmap glyphs, subpixel
AA). ZeroTerm state audited against the working tree at v0.3.10.

---

## 0. TL;DR — the five highest-leverage moves

| # | Move | Why it wins | Rough size |
| - | ---- | ----------- | ---------- |
| 1 | **Exit-code awareness** (OSC 133;D parse + PTY wait code) → gutter dot + status-bar badge | Turns invisible failures visible; the #1 requested terminal nicety | Small–Medium |
| 2 | **Shell spawn fallback + in-canvas error banner** (no silent dead pane) | Fixes the worst failure mode: blank/never-spawned pane with only a log line | Small |
| 3 | **OSC 133/7 shell integration** (block markers, cwd) | Unlocks block nav, prompt jumping, smart resize, `cd`-aware new tabs | Medium |
| 4 | **Confirm-on-close with running process** + exit-code-aware `[Process exited (code N)]` | Prevents data loss; the exit notice becomes actually informative | Small |
| 5 | **Renderer: cursor overlay cell-only invalidation + ligatures via swash shaping** | Visual polish + battery (blink redraws one cell, not the frame) | Medium |

---

## 1. UX / UI best practices

### 1.1 Tab bar — ZeroTerm is in good shape, keep pushing contrast

**Best practice (2025-26):** tabs must read as *separate buttons* even on
low-contrast displays; the active tab is the brightest, inactive tabs dim,
hover sits between. Badges (bell 🔔, unread, exit-failure) live on the tab.

**ZeroTerm today (`renderer.rs::draw_tab_bar`):** pills with an explicit
brightness ladder (inactive 0.42 → hover 0.52 → active 0.66 toward accent),
1-cell border-colored separators, close glyphs, bell badges, hover states.
This already implements the practice — the separator was specifically added
after the "tabs run together" regression.

**Next steps:**
- **Exit-failure badge on the tab** (pairs with §2.1): a red dot on the tab
  of any pane whose last command exited non-zero (cleared on focus). Reuses
  the existing `bell_rung` latch pattern (`main.rs` drain loop + `TabInfo`).
- **Unread-activity dot** for inactive tabs with output since last focus
  (already have the `bell_rung` machinery; generalize the latch).
- **Active-tab underline accent strip** (Ghostty/iTerm2 style) in addition to
  the pill brightness — an extra horizontal cue that survives any theme.

### 1.2 Search — per-screen search is fine; add match-count + block scope

**Best practice:** Ctrl+Shift+F opens an in-window find bar with match
count (`3/17`), Enter/Shift+Enter to cycle, Esc to dismiss, and the option to
scope to the current output block (OSC 133) once integration exists.

**ZeroTerm today:** per-screen search with scrollback, highlight, output-block
navigation. Good foundation.

**Next steps:**
- Match counter in the search overlay footer.
- "Search within current block" toggle once OSC 133 lands (§2.2) — cheap
  because `drain_pty` already parses block markers.

### 1.3 Command palette / quick actions

**Best practice (Warp `Cmd+P`, WezTerm launcher):** one fuzzy modal that
searches commands, history, settings, and actions.

**ZeroTerm today:** no palette; keybindings are discoverable only via docs.

**Next step (after the robustness items):** a `Ctrl+Shift+P` overlay listing
actions (new tab, split, search, toggle opacity, settings…) with fuzzy match
and the bound key shown per row. The overlay plumbing already exists
(`overlay.rs` ScreenScratch + settings/search/ai-style overlays were unified).
Low risk, high perceived polish.

### 1.4 Hinting — OSC 8 done; add WezTerm-style QuickSelect for *commands*

**Best practice:** WezTerm `QuickSelect` highlights patterns (git hashes,
file paths, URLs) with jump labels — keyboard targeting without the mouse.

**ZeroTerm today:** OSC 8 hyperlinks with hover-in-status-bar + click-to-open
(just shipped, v0.3.10). URL auto-detection exists in `highlight.rs`.

**Next step:** extend the highlight pass with a QuickSelect mode
(`Ctrl+Shift+Space`): regex-scan the visible screen for `\b[\w./-]+:\d+\b`
(file:line), `[0-9a-f]{7,40}` (hashes), and render jump-label overlays.
Moderate effort; reuses `highlight.rs` scan machinery.

### 1.5 Scrollback navigation — already strong; add jump-to-prompt

**Best practice:** trackpad momentum scroll, scrollbar thumb, `Shift+PageUp/Dn`,
and (with shell integration) jump between prompts/blocks.

**ZeroTerm today:** scrollback with search, block navigation, scrollbar with
thumb, wheel accumulation. Solid.

**Next steps:** once OSC 133 lands, bind `Ctrl+Shift+Up/Dn` to jump
prompt→prompt (block boundaries are already tracked for nav) and show a
"at top of scrollback" marker.

### 1.6 Split dividers — drag exists; add live-percentage + double-click maximize

**Best practice:** dragging shows a live percentage overlay (Ghostty);
double-clicking a divider splits; `Ctrl+Shift+Z` zooms a pane to full
(WezTerm).

**ZeroTerm today:** drag-to-resize with divider hit-testing and anchor-delta
math (`main.rs` CursorMoved/divider drag). Good.

**Next steps:**
- Percentage readout near the cursor while dragging (one text quad).
- Pane zoom/maximize (`Ctrl+Shift+Z` toggles): render only that pane, hide
  the others; the split tree already supports per-pane rects so this is a
  view-level change in `render()`.

### 1.7 Status bar — make it stateful, not decorative

**Best practice (WezTerm `set_right_status`):** the bar shows *live* state:
active pane cwd, git branch, shell-exit status, scroll position, notification
toggle.

**ZeroTerm today:** `status_left` = mode/overlay state; `status_right` =
scroll position or hovered URL. Already scroll-aware and now link-aware.

**Next steps:**
- Exit-status chip in the left side (`● last command: 0` / red `✗ 127`) from
  §2.1.
- OSC 7 cwd → show current directory (and seed new tabs from it, §2.2).
- Keep it compact — one row, left = mode+cwd, right = status+URL.

### 1.8 Themes & contrast

**Best practice:** light/dark following the OS; per-theme accent; kitty's
`text_fg_override_threshold` elevates low-contrast text.

**ZeroTerm today:** `tokyo-night`-flavored theme with accent/surface/border
tokens; config reload applies live (watcher + `apply_config_to_renderer`).

**Next steps:**
- Ship 3–4 built-in themes (`[colors] theme = "..."` already reserved in
  config docs) + `Ctrl+Shift+T` cycle. Nearly free since the theme token
  struct already centralizes colors.
- Optional OS light/dark detection via winit `ThemeChanged`.

### 1.9 Keyboard-driven navigation

**Best practice:** zero-mouse terminal workflows: vim-style copy mode,
leader-key palettes, platform-correct modifier defaults.

**ZeroTerm today:** vim_mode toggle, vi-style editing in the line editor,
rich `key_router` with per-action unit tests (+25 tests). Already strong.

**Next step:** vim-style scrollback copy mode (`Ctrl+Shift+C` → selection
motions in the buffer) — the selection controller already has pure math
extracted for testing.

---

## 2. Error handling & robustness

### 2.1 Exit status surfacing — the biggest single win ✅ shipped (v0.3.11)

**Best practice:** parse `OSC 133 ; D ; <code>` (and read the PTY wait code)
so the terminal knows whether the last command succeeded; surface as a gutter
dot, tab badge, and/or status chip.

**Shipped:**
- `parser.rs` `handle_osc133`: `A`/`B` open a block (clearing pending first),
  `C` writes the command onto the running block (`cmdline_url=` stripped),
  `D` sets the exit code + finalizes the block.
- `Screen`: sticky `last_exit()` (survives a new block opening),
  `has_running_block()`, `finalize_block()`, `set_running_block_command()`;
  `reset()` (RIS) clears all of it.
- `drain_pty` notice now carries the code: `[Process exited (code 127)] -
  exit to quit`.
- Status-bar chip (`✓ 0` / `✗ 127`) via `frame::status_left(…, exit)`.
- Tab failure dot: `TabInfo.failed` painted red (`close_red`) on the tab's
  left padding cell; failure outranks the bell dot.
- Shell snippet: the bash rcfile bootstrap sets a `PROMPT_COMMAND` hook and
  the zsh bootstrap prepends a `precmd_functions` hook (prepend matters —
  starship's own precmd runs `starship prompt`, which clobbers `$?` if our
  hook runs second).

**Known limitation:** `exit N` at the prompt does not run `PROMPT_COMMAND`,
so the notice shows the last *command's* code; the exact shell code needs the
PTY `wait()` path (a follow-up).

### 2.2 Shell integration — OSC 133/7 (FinalTerm block model) ✅ parsing shipped

**Best practice:** the shell emits `OSC 133;A` before the prompt,
`;B` before input, `;C` at command start (with `cmdline_url=`), `;D` at
finish (with exit code); `OSC 7` carries cwd. WezTerm/Kitty/Ghostty all
consume these.

**Shipped:**
1. Parse the four `133;X` markers + optional `;D;<code>` into Screen
   (`blocks`, `last_exit`, `has_running_block`).
2. Parse `OSC 7;file://host/path` → `Screen::cwd` (percent-decoded).
3. Shell snippet baked into the existing bash/zsh rcfile bootstrap
   (`session.rs starship_setup`) — works for every local pane automatically.

**Behavior gains that fall out for free:** prompt-jump keys, block-scoped
search (§1.2), cwd in status bar (§1.7), new-tab-seeded-from-cwd, and a
gutter marker for the running block — the accessors are all in place; each
feature is a small consumer on top.

### 2.3 PTY death & spawn failure — never a silent dead pane

**Best practice:**
- Spawn failure (shell binary missing): fall back to a safe default
  (`/bin/sh`), show an inline error card, don't loop.
- PTY death: keep the window, show a clear banner with recovery options.
- Resize (`TIOCSWINSZ`) failure: log quietly, keep last valid grid.

**ZeroTerm today:**
- Spawn errors surface only via `error!` logs (`main.rs` create_new_tab /
  split / ssh sites return `Result` and log). The pane would just be blank
  with a log line — the worst failure mode.
- Resize errors: `portable_pty` failures would propagate; grid stays at last
  valid size only by luck.

**Changes:**
1. `spawn_shell` helper: if configured shell fails to spawn, try `/bin/sh`,
   then display an in-canvas error banner (a row of red text through the
   parser, like the exit notice — one line: `Failed to spawn /bin/zsh:
   No such file or directory — using /bin/sh`).
2. Keep a `ResizeGuard` around `pane.parser.resize()` calls: on `Err`, log
   and reuse last-good `Size` instead of unwinding the frame.

### 2.4 Confirm-on-close with a running process

**Best practice:** if a pane has a live child (not `pty_dead`), the close
path should confirm (Ghostty/WezTerm prompt or config-gated auto-kill).

**ZeroTerm today:** `CloseRequested` kills every pane unconditionally
(`main.rs:2633-2637`) — `pkill zeroterm` loses all running commands.

**Changes:**
1. `close_tab`/`CloseRequested`: if any pane is alive, show an in-canvas
   confirm overlay (`Close tab — running processes will be killed  [Y/n]`)
   reusing `overlay.rs`; Esc cancels.
2. Config key `[session] confirm_on_close = true` (default true).

### 2.5 In-canvas status banners & diagnostics view

**Best practice:** subtle dismissible banner for non-fatal issues; a
`Ctrl+Shift+D` diagnostics tab listing recent parser warnings and shell
integration status — instead of modal popups.

**ZeroTerm today:** everything goes to `log` (env_logger) which users never
see. The `diag` probe (`renderer.rs` `self.diag`) already exists for
renderer internals.

**Changes:**
1. Route `warn!`/`error!` into an in-memory ring (e.g. `App::recent_events:
VecDeque<(time, level, msg)>`) and show the last error as a banner row until
dismissed (Esc).
2. `Ctrl+Shift+D` overlay lists recent events + which protocols each pane
   negotiated (mouse tracking, bracketed paste, kitty keyboard, sync output,
   OSC 8/133) — the data already lives on the parsers.

### 2.6 Panic-robustness audit

**Audit result (working tree v0.3.10):**
- `unwrap()/expect()` in production paths are concentrated in the renderer
  (`current_encoder` invariant — encoder exists iff a frame is active) and
  one `event_proxy` startup expect (`main.rs:241`, set before `run_app`) and
  `config_path.parent().unwrap()` (`main.rs:575`).
- No `panic!/todo!/unimplemented!` in main paths. `cargo clippy` is clean.

**Changes:**
1. `main.rs:575` parent path: replace with `unwrap_or(config_path.clone())`.
2. Add a top-level `catch_unwind` in `run_app` that on panic saves
   `~/.local/share/zeroterm/crash.log` (state snapshot is cheap; scrollback
   is already serialized for session restore) and shows a dialog — the
   "crash resilience" practice from the research.
3. Fuzz harness already exists (`zeroterm-core/fuzz`) — keep feeding it;
   the parser is the only untrusted input.

---

## 3. Rendering quality

### 3.1 Architecture — already the industry standard

**Best practice:** GPU instanced quads + glyph atlas + CPU-side grid. SDF is
for vector UI, not terminal text — bitmap atlases at fixed sizes are crisper.

**ZeroTerm today:** wgpu renderer, swash rasterization into a glyph atlas,
instanced draw (per the audit: `atlas.rs` caches glyph bitmaps + placement
offsets; `renderer.rs` batches quads). **Already conforms** — this is why
the "1:1 glyph placement, no stretch" fix landed crisply.

### 3.2 Antialiasing — grayscale, not subpixel

**Best practice:** grayscale AA is the modern standard (macOS dropped
subpixel; subpixel fringes with transparency/blur; high-DPI makes it moot).

**ZeroTerm today:** swash bitmap rasterization (grayscale coverage into the
atlas) — correct default. No action needed; **do not** "fix" to subpixel.

### 3.3 Font shaping & ligatures (the visual upgrade)

**Best practice:** shape runs with GSUB so `=>` `!=` `::` become ligatures;
continuation cells stay in the grid for selection/cursor coherence.

**ZeroTerm today:** swash is already a dependency and does shaping; the
renderer draws per-glyph bitmaps from the atlas — the atlas pipeline was
built for that. Ligatures are the natural next step (`FEATURE_OPPORTUNITIES`
#5): shape each attribute-run on demand, span the glyph across N columns,
mark continuation cells.

**Note:** this must be opt-in per theme and off by default if it complicates
the 1:1 cell model — Ghostty ships it on by default, WezTerm default-on;
either is defensible.

### 3.4 Cursor — line/bar/block all exist; fix the blink redraw cost

**Best practice:** DECSCUSR shapes (done), cursor drawn as an overlay (not a
cell mutation), blink invalidates only the cursor's cell.

**ZeroTerm today:** DECSCUSR → Block/Underline/Bar (`parser.rs:642-651`,
`screen.set_cursor_shape`), config-gated blink with an interval timer. The
nvim line-vs-block issue from earlier sessions is fixed by DECSCUSR.

**Next step:** the blink path currently re-arms a full redraw (`about_to_wait`
re-arm). Restrict the damaged rect to the cursor cell when only the blink
toggled — battery + avoids re-uploading the whole quad batch 2×/sec.

### 3.5 Underlines, strikes, undercurl

**Best practice:** procedural decorations (single/double/curly/dotted),
independent underline color (`SGR 58`).

**ZeroTerm today:** basic underline via cell attributes; OSC 8 links get an
accent underline (cell_batch). No curly/dashed/double.

**Next step:** add double/curly (`_` → `~`) as a fragment-shader function of
cell rect — moderate, high "premium" feel for nvim users who configure
undercurl. Lower priority than §3.3.

### 3.6 Fractional DPI & blur

**Best practice:** rasterize at the exact physical scale (never stretch the
presented texture); blur/opacity at the compositor, or a renderer pass.

**ZeroTerm today:** `estimate_cell_size` + `GlyphAtlas::set_font` take DPI
from the window; blur is a renderer Gaussian pass (documented in
ARCHITECTURE.md); `clear_color` was linearized so the sRGB surface matches
the background quad. The earlier "pixelated text" complaints trace to the
**stretch-to-cell bug that was fixed** — keep the 1:1 placement.

**Next step:** none required. If blur is ever reworked, prefer the
compositor protocol (`KDE blur`/GNOME) over an extra render pass.

### 3.7 Output floods — already handled, one gap

**Best practice:** read PTY on a thread, parse off-thread, coalesce to one
render per vsync, and honor synchronized output (DECSET 2026).

**ZeroTerm today:** threaded reader (`split_reader`, the deadlock fix), frame
coalescing, `sync_active` gating (`main.rs` drain loop, `sync_output()`
flag), dedupe of the sticky exit notice. **This is the correct design.**

**Gap:** when *not* synced and data floods, the render loop still drains as
fast as events arrive. Add a hard cap (e.g. one drain batch per
`request_redraw` frame, queue the rest) so `cat huge.log` doesn't starve the
UI thread. Small, worthwhile.

---

## 4. Behavior on specific commands

### 4.1 Full-screen TUIs (vim, nano, htop, fzf, tmux, mc)

**Best practice:** honor the alternate screen (DECSET 47/1049) with full
save/restore of the primary buffer; SGR mouse (1006) for fzf/ncurses; smooth
resize inside the app (SIGWINCH → reflow at the app's request); never scroll
the alt screen into scrollback.

**ZeroTerm today:** alt screen with buffer swap (`screen.rs:934`), mouse
tracking modes incl. SGR 1006 and any-event (`parser.rs:809-818`), resize
propagates `TIOCSWINSZ` via the PTY. **Conforms.**

**Gap:** alt-screen resize — when rows change while `use_alt_screen`, the
`resize()` path (`screen.rs:164`) only trims the *primary* scrollback;
verify the alt buffer reflows cleanly (add a test: enter alt, resize, exit,
assert primary scrollback intact).

### 4.2 Ctrl+C / SIGINT

**Best practice:** Ctrl+C sends `\x03` to the foreground process group (the
PTY does this — the emulator just writes the byte); SIGINT/SIGTERM to the
emulator itself should cascade to the child group on close.

**ZeroTerm today:** Ctrl+C is a plain `\x03` Pty write (`key_router` → PTY);
close kills panes (`PtyCommand::Kill`). Correct. The only gap is the §2.4
confirm-on-close.

### 4.3 Commands that exit immediately vs. long-running

**Best practice:** don't close the pane on command exit (only on *shell*
exit); keep output readable; mark exit with a status affordance.

**ZeroTerm today:** pane closes only when the *shell* exits (`pty_dead`),
not when a foreground command exits — correct. `[Process exited]` notice is
sticky-once. The improvement is §2.1 (show the code) and §2.3 (restart
hint).

### 4.4 Mouse reporting during scrollback

**Best practice:** when the user scrolls into scrollback, mouse reporting
should not hijack clicks meant for the buffer; click-to-focus still works.

**ZeroTerm today:** mouse clicks route to apps when mouse tracking is on
(even in scrollback). This is standard (kitty does the same) — leave as-is,
but make sure `Ctrl+click` on an OSC 8 link still wins (verify the link
handler runs before the mouse-report write in `main.rs`).

### 4.5 Paste behavior

**Best practice:** bracketed paste (2004) so readline/nvim insert literally;
plain fallback.

**ZeroTerm today:** `paste_clipboard` honors bracketed paste and falls back
to raw bytes (`main.rs:2568-2579`). **Conforms.**

---

## 5. Suggested implementation order

1. **Exit-code awareness** (§2.1) — small; every other UX item leans on it.
   Includes: parser `133;D` + PTY `wait()` code → Pane, exit notice with
   code, status chip, tab failure dot.
2. **Spawn-failure fallback + error banner** (§2.3.1, §2.5.1) — small,
   kills the worst failure mode. Ring buffer of recent events + banner.
3. **OSC 133/7 shell integration** (§2.2) — medium; unlocks block nav,
   cwd, smart resize, jump-to-prompt. Ship the rc snippet alongside the
   existing starship bootstrap.
4. **Confirm-on-close + R-to-restart** (§2.4, §2.1.3) — small, safety.
5. **Cursor cell-only blink invalidation** (§3.4) — small, battery.
6. **Ligatures via swash shaping** (§3.3) — medium, the visual upgrade.
7. **Command palette** (§1.3) and **QuickSelect hinting** (§1.4) — medium,
   both ride on existing overlay/highlight machinery.
8. **Pane zoom + divider percentage** (§1.6) — small-medium, multiplexer UX.

**Deliberately deferred / out of scope:** subpixel AA (wrong for modern
displays), SDF text (worse than bitmaps at fixed sizes), native OS title-bar
integration (Ghostty's approach — needs GTK/AppKit, not a rendering change),
AI chat UI (product decision: removed).
