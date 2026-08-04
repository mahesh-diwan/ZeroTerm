# ZeroTerm Execution Roadmap — Stage Tracker

Source: user's visual-overhaul plan (m2247) + 5-track roadmap (m2306) + exact execution manual (m2358, Phases 1-5).

## Baseline (measured 2026-08-02)

- App RUNS, executes commands (ls/fastfetch verified via pty + grim screenshot analysis).
- Boot time: process start → renderer ready ≈ 420ms (window created at 47.465, renderer init done at 47.875).
- All 4 deadlock/buffer/surface fixes DONE (b3). GPU adapter fallback DONE (b9/b11). sRGB double-gamma shader fix DONE (b11/b13). Content padding 16px DONE (b14/b15).
- Binary: debug target only, no release audit yet.

## Phase 1 — Architecture Refactor (kill the god struct)

Target: `crates/zeroterm/src/app/{mod,session,chrome,input,extensions}.rs` (note: manual lists `render_context.rs` too; keep only files we actually fill).

- [x] 1.1 create `src/app/` module structure — DONE (mod.rs re-exports chrome/extensions/input/session)
- [x] 1.2 `session.rs` — move PTY/spawn logic + `PaneState`/`PtyCommand` out of main.rs — DONE. Ported the two-thread split_reader design VERBATIM (reader thread → sync_channel(4) + wake.send_event; command thread blocks on recv dispatching Write/Resize/Kill). Also moved starship_setup (include_str! path changed `../assets/`→`../../assets/`), spawn_ssh_process (set_timeout(50) poll loop). SessionManager + tab/split logic NOT moved yet (1.7 remainder).
- [x] 1.3 `input.rs` — EditingState + word_left/word_right/is_word_char moved — DONE. EditingState gained pub methods: `from_line(line)` (replaces new(), cursor at end), `is_empty()`, `truncate_to_cursor()` (C-k). Fields private.
- [x] 1.4 `chrome.rs` — HostPicker moved — DONE (`open` kept pub, main.rs writes `self.host_picker.open = false` directly).
- [x] 1.5 `extensions.rs` — block_output_text + load_plugins moved — DONE (OnceCell containers for ai/sync/plugins still in main.rs App fields, not yet gated).
- [x] 1.6 `mod.rs` — re-exports created — DONE (pub use for all moved items). AppState struct NOT yet created.
- [x] 1.7 refactor `main.rs` `App` to compose `{state, session, input, chrome, ext}` — DONE. SessionManager extracted into app/session.rs (owns panes/active_pane/next_pane_id/tabs/split_root/floating/dragging_divider/divider_anchor/scroll_offset + pure nav/tree methods: new/active_pane/active_pane_mut/pane/pane_mut/pane_ids/compute_split_rects/next_tab/previous_tab/switch_to_tab/focus_adjacent_pane/max_scroll_offset/scroll_up/scroll_down — nav methods return bool changed). App struct gained `session: SessionManager` field; 163 `self.X`→`self.session.X` refs sed-rewritten; App navigation methods are thin delegates calling `if self.session.next_tab() { self.redraw(); }`. App keeps all renderer/window-orchestration methods (render, drain_pty, tab/split/close/floating, resize, hit-testing, window_event, jump_to_block). Added `fn redraw(&self)` helper. Fixed multiline refs sed missed (`.panes`/`.split_root`/`.active_pane()` on continuation lines at ~864/1220/2093/2128/2186 + test at 2388-2423). Removed `#[derive(Default)]` from SessionManager (SplitNode has no Default; new() covers it). Main.rs 2521 lines.
- Verify: `cargo check -p zeroterm` — PASSED. cargo build 0 errors (only pre-existing unused `dir` warn create_split_pane main.rs:364), 209 tests pass (21 suites), clippy clean. App relaunched PID 1185406: "ZeroTerm initialized: 90x53 (bash)", "GPU renderer ready: 81x40", input via /dev/pts/1 accepted, grim shows content (109k bright px).

## Phase 2 — Binary Size Reduction

- [x] 2.1 root Cargo.toml: add `once_cell = "1.19"`, `which = "6.0"` to workspace deps — SKIPPED (already present via workspace deps)
- [x] 2.2 `crates/zeroterm/Cargo.toml`: features `default=["gpu","themes"]`, `gpu/ai/sync/ssh/plugins`; heavy deps optional (reqwest, wasmtime, wasmtime-wasi, chacha20poly1305, chacha20, poly1305) — DONE (also zeroterm-ai/sync/plugin/tokio optional behind ai/sync/plugins; zeroterm-ssh optional in `[target.'cfg(unix)'.dependencies]` behind ssh)
- [x] 2.3 release profile: `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip="symbols"`, `debug=false` — DONE
- [x] 2.4 `#[cfg(feature=...)]` gate ai/sync/plugins in extensions.rs — DONE + full wiring: main.rs imports/fields/init/keybinds gated (E0658 cfg-on-expression fixed via block-wrapping at 280/282/297; spawn_ssh_process `#[cfg(all(unix,feature="ssh"))]`; open_host_picker/pick_host/HostPicker::{open,selected,save_screen} + host_picker test gated; cfg'd imports moved into `#[cfg(feature="plugins")]` blocks in extensions.rs; removed unused `std::path::Path` in session.rs)
- Verify: build 0 errs, clippy --workspace --all-targets -D warnings CLEAN, fmt clean, 208 tests pass (host_picker test gated behind ssh feature, hence 209→208), app relaunched and verified (GPU renderer ready 81x40, ls via pts input, grim dark-bg+text pixels)

## Phase 3 — Boot Speed (<200ms cold)

- [x] 3.1 deferred GPU init: window + config + PTY spawn immediately (cols/rows estimate), Renderer::new on bg thread via mpsc, `check_renderer_ready()` in render() polls rx.try_recv, then resizes PTY to real cell metrics — DONE (verified: boot 382ms to "ZeroTerm initialized", renderer fills async "GPU renderer ready: 81x48")
- [x] 3.2 bundled font: Nerd Font path detection FIRST (JetBrainsMonoNerdFont-Regular.ttf) then LiberationMono/GeistMono/DejaVu + embedded DejaVuSansMono fallback — DONE (no JetBrains Mono plain on system, so bundled asset skipped; Nerd Font loaded from /usr/share/fonts/TTF/. Cells 81x48→81x40 due to Nerd Font metrics). `include = [...]` in Cargo.toml NOT needed (default package includes .ttf)
- Verify: release build boots to visible window < 100ms, renderer fills in

## Phase 4 — Themes, Fonts & Starship

- [x] D1 curated Theme pack — DONE (theme.rs: 8 presets via const fns + by_name + map_cell_color; renderer.rs: `theme` field, reload_config/set_theme derive clear_color; config ColorConfig.theme default "tokyo-night"; update_cell_data maps fg/bg; draw_tab_bar uses theme colors). Build green, clippy clean.
      NOTE manual's 4.1 Theme struct differs from mine (mine has no fg_muted/fg_dim/accent_secondary/success/warning/error/selection_fg). Only extend if a feature needs them.
- [x] 4.2 wire remaining hardcoded colors → theme.* — DONE (clear_color init #1a1b26 renderer.rs:881, highlight_color(idx,&Theme)→theme.ansi[6]/[3]/[5]/[8])
- [x] 4.3 `crates/zeroterm/assets/starship.toml` default config — DONE
- [x] 4.4 auto-inject Starship on shell spawn — DONE (PtyBackend::spawn 4-arg with env; spawn_pty_process env param; starship_setup() wraps bash/zsh via `eval "$(starship init bash)"; exec bash -l`; verified prompt renders)

## Phase 5 — Solid UX

- [x] 5.1 shell-spawn error recovery: fake channel delivering ANSI error message on spawn failure — DONE in app/session.rs: `spawn_pty_process` never returns Err now; each fallible step (backend new/spawn/split_reader/resize) degrades to `spawn_err_channels()` which returns a fake (pty_rx, pty_tx): one sync_channel pre-loaded with `\x1b[31m[zeroterm] failed to spawn shell 'X': err\x1b[0m\r\n` then closed, plus a command channel with dropped receiver (all sends fail). All call sites (`?` in init/new tab/split) work unchanged. Test `spawn_err_channels_delivers_ansi_error_and_swallows_commands` added.
- [x] 5.2 URL detection: `url: bool` on Attributes + regex pass + theme.accent/underline render (needs regex/lazy_static — check deps first) — DONE, no new deps. highlight.rs: `HL_URL=5` + `url_len_at()` hand-rolled scanner (matches `http(s)://`, `ftp://`, `www.`, strips trailing `.,:;!?`, stops at whitespace/quotes/`<>[]{}|`) + a second pass in `highlight_line` that overrides earlier classes (URL wins over keyword/comment). renderer.rs: `highlight_color` maps HL_URL → `theme.accent`; URL cells forced to `UnderlineStyle::Single` via a `cell_attrs` copy. Tests: `urls_are_detected`, `www_and_trailing_punctuation`, `url_overrides_keyword_and_comment`, `no_url_without_scheme`.
- [x] 5.3 copy-on-select (end_selection → copy_selection) — DONE in main.rs MouseInput release handler: `let dragged = self.selecting; self.end_selection(); if dragged && selection spans >1 cell { self.copy_selection(); }`.
- [x] 5.4 search overlay skeleton: SearchState{match list, current}, Ctrl+Shift+F toggle, scrollback+visible scan — DONE in new crates/zeroterm/src/search.rs (SearchState{open,query,matches,current,saved_cells,saved_cursor}, case-insensitive find over scrollback+visible excluding last row, next/prev wrap, bottom-row bar via overlay_bytes CSI, save/restore_screen). main.rs: `mod search`, `search: SearchState` field, toggle_search/close_search/draw_search_overlay/search_apply/search_step/search_jump; search-mode key interception (Escape/Backspace/Enter/Arrows/append); Ctrl+Shift+F binding added, floating-pane moved to Ctrl+Shift+G. Unit tests: find_scans_visible_buffer_and_tracks_matches, empty_query_and_no_match.

## Phase 5+ fixes (post-5.4 hardening, from rendering review)

- Fix: bracketed-paste probe `\x1b[?2004h` DELETED from spawn path (main.rs:218). Writing it pre-readline lands in the pty line discipline: bash echoes it back AND later reads it as buffered input, consuming `\x1b[?` and leaking literal `2004h` as typed text (`[prompt]$ 2004h`). Proven via python pty.fork repro: bare bash and the exact starship wrapper both advertise `\x1b[?2004h` themselves with NO leak. Parser handles bash's own `\x1b[?2004h` (bracketed_paste=true).
- Fix: cell grid-line seams — renderer.rs `draw_background()` (opaque full-window theme.bg quad via LoadOp::Clear, sets needs_clear=false, mirrors draw_tab_bar bind-group pattern) called first in main.rs render() after begin_frame; shader.wgsl vs_main inflates each instance quad by 0.5 device px (`clip_xy += (local*2-1)/uniforms.screen_size`) + tex_coord clamped to [0,1]. Mirrors alacritty background-pass + ghostty grid_padding (13-source web research: alacritty #5499/text.v.glsl/gles2 pass0 BlendFunc(ONE,ZERO), flutter_alacritty isAntiAlias=false, ghostty #9432 ceil→round, xterm.js #6015 floor, bevy #10537). Visual seam verification INCONCLUSIVE — Hyprland global decoration:active_opacity=0.85 composites the window (measured bg (65,68,86) ≠ theme.bg (26,27,38)), grim captures only active workspace, setprop not supported; screenshots always show content so no pure-bg rows to scan. Fix accepted on research basis.
- Fix: async-renderer lost-Resized-event race — check_renderer_ready (main.rs:839-855) now calls `renderer.resize(size.width,size.height)` before storing renderer (window may already be resized larger than the renderer's initial capacity, truncating visible rows).

## Track C remaining (outside manual Phase 3)

- [x] C2 async config loading (defaults instantly, hydrate from disk) — Config::load_async() returns defaults + mpsc Receiver; main.rs polls rx in render(), swaps hydrated config, re-applies to renderer. Test: load_async_returns_defaults_then_hydrates_from_file.
- [x] C4 shader pipeline cache (wgpu 23+) — wgpu 23 has NO `cache` feature (that's 25+); real API: Features::PIPELINE_CACHE + unsafe device.create_pipeline_cache(fallback:true) + cache.get_data() persisted to ~/.cache/zeroterm/wgpu_pipeline_cache_vulkan_<vendor>_<device> (19.1K verified). dirs dep added to zeroterm-render.

## Track B — Binary Size

- [x] B1 cargo bloat audit (after Phase 2) — 9.0M stripped (13.0M unstripped, .text 9.2M). wgpu+naga 1.9M, winit 1.1M, font pipeline 0.9M, regex 0.23M (tracing env-filter), image 0.3M, misc ~0.3M. Cheap wins: winit `wayland-csd-adwaita` off (−0.25M) + `webp` off (−0.16M). ai/sync/plugins/ssh feature-gating already avoids +8-15M.
- [x] B3 zstd release packaging — scripts/package-zstd.sh → dist/zeroterm-v<VERSION>-<ARCH>.tar.zst (gzip fallback), verified 3.2M
- [x] B4 thiserror in zeroterm-render hot path — RendererError enum (CreateSurface/Device/NoAdapter/FontNotFound, thiserror derive), `pub type Result<T>` alias; anyhow dep removed from zeroterm-render/Cargo.toml

## Track E remaining (supersedes some manual Phase 5)

- [x] E3 smooth velocity scroll — PixelDelta accumulates fractional lines (`scroll_fraction`, |rem|<1) and applies only whole lines, so trackpad scrolling glides; LineDelta unchanged; renderer untouched (still integer offsets). Also fixed pre-existing renderer.rs:538 breakage (`?` on anyhow; now uses existing `RendererError::NoAdapter`).
- [x] E6 quake mode (global F12) — F12 intercepted (verified: no \x1b[24~ leak to pty); toggle_quake uses set_visible which is a NO-OP on Wayland in winit 0.30 (and set_minimized(false) is too — unminimize ignored), so on Wayland F12 is currently a safe no-op. True quake dropdown needs Wayland global-shortcut portal / layer-shell (documented in code comment).

## Step 2 visual (from m2247, partially superseded by Track D)

- [x] 2.1 content padding (16px)
- [x] 2.2 theme system → D1 (done)
- [x] 2.3 modern tab bar (pill, accent line, close btn on hover) — DONE (TabInfo+{hovered,close_hovered}; active pill = surface_highlight tinted 30%→accent, 1-cell accent line, × close glyph on hover; hover/close hit-test via tab_bar_hover in main.rs, close_tab by id)
- [x] 2.4 command-block separators (ATTR_BLOCK_TOP_BORDER) — already implemented (renderer.rs ATTR_BLOCK_DIVIDER=0x800 + shader 0x800 tint + [copy] marker + screen.rs mark_block_boundary)

## Step 4 polish (from m2247)

- [x] animated cursor (blink) — CPU-side phase toggle + timer, NOT shader (alacritty model; blink_visible gates is_cursor_cell). config cursor.blink=true / blink_interval_ms=530
- [x] status bar (bottom row, viewport_origin=[0,win_h-cell_h]) — pane title left, [N%] scroll right; content_h shrunk by status bar height in ALL layout sites
- [x] scrollbar overlay (right 2 cols of active pane, thumb=theme.accent, only when max_scroll>0)
- [x] rounded selection corners — REJECTED (zero precedent in any terminal; conflicts with instanced-quad model + half-px seam fix)
- [~] window transparency + blur — PARTIAL: opacity already existed (config.window.opacity, cycle_opacity); blur/blur_radius config fields added (defaults off) but renderer has NO blur pass yet

## Execution order chosen

1. Track D1 themes — DONE
2. Phase 4.2-4.4 (theme stragglers + Starship) — DONE → Phase 3 boot (3.1/3.2) — DONE → Phase 1 arch refactor → Phase 2 size → Phase 5 UX
3. Manual's "Start with Phase 1.1→1.2→1.7" is the arch refactor; roadmap's own priority was C/D first. Both recorded; current focus per roadmap = finish Phase 4, then Phase 3 boot.

## Final build checklist (manual)

```bash
cargo check --all
cargo build --release -p zeroterm
ls -lh target/release/zeroterm
./target/release/zeroterm   # test ls, echo hello, git status, starship prompt, theme, padding, tab bar
strip target/release/zeroterm
```
