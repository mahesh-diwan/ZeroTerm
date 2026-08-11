# Kitty Design System Study

Research into the kitty terminal emulator's design system and UI/UX elements,
mapped to ZeroTerm. Sources: kitty's official config reference
(`kitty.conf`, sw.kovidgoyal.net/kitty/conf/) and kitty.app philosophy.

## The philosophy (why kitty looks the way it does)

- **Keyboard-first, chrome-minimal.** kitty draws almost no UI of its own: no
  menu, no settings dialog (config is a file opened with `ctrl+shift+f2`),
  no status bar, no toolbar. Everything a power user does is a keybinding or
  a config line. The terminal content is the interface.
- **Colors are configuration tokens.** Every chrome color (tabs, selection,
  cursor, marks, scrollbar) is a named option that themes can set. Nothing is
  hardcoded in the drawing code.
- **Chrome appears only when it earns its pixels.** The tab bar is hidden
  with a single tab (`tab_bar_min_tabs`, default 2). Window borders render
  only when more than one window is visible (`window_border_width`,
  `draw_minimal_borders`). The scrollbar track is fully transparent
  (`scrollbar_track_opacity 0`); only the handle shows, and only once
  scrolling starts (`scrollbar scrolled`).
- **Feedback without interruption.** Bells flash the screen rather than
  stealing focus (`visual_bell_duration`, `visual_bell_color` → defaults to
  the selection color); a bell in an unfocused window marks its tab with
  `bell_on_tab` ("🔔 "); the OS taskbar flashes via `window_alert_on_bell`.

## Tab bar (`tabs.html` / kitty.conf)

- `tab_bar_style` — `fade` (default), `slant`, `powerline`, `hidden`. The
  iconic kitty look is powerline/slant separators between tabs; `fade` is a
  simple colored block per tab.
- `tab_bar_min_tabs 2` — **no tab bar until the second tab exists.** This is
  the single biggest chrome saver in kitty.
- Active vs inactive tabs have explicit colors (`active_tab_foreground/
  background`, `inactive_tab_foreground/background`); kitty's default active
  tab is a solid, saturated block (blue) so the focused tab is unmistakable.
- `tab_separator` (default " "), `tab_bar_edge` (top), `tab_bar_align`,
  `tab_bar_margin_width` — geometry is tunable but the defaults are tight.
- `tab_title_template` can include `{bell_symbol}`, `{activity_symbol}`,
  `{index}` — the bell/activity indicators are text woven into the title.
- `bell_on_tab "🔔 "` — an unfocused tab whose window rang the bell shows the
  symbol in its title.

## Bell

- `enable_audio_bell yes`, `visual_bell_duration 0.0` (seconds; 0 = off),
  `visual_bell_color none` → **falls back to the selection background**.
- The flash is eased: fades in and out over the duration (ease-in-out
  default).
- `window_alert_on_bell yes` — taskbar/dock attention request.

## Search

- Kitty's scrollback search (Ctrl+Shift+F) highlights **every match in the
  buffer in place** — not just the current one — using `search_match_background`
  (default a mid grey), with the current match distinguished separately.
- Search is a bottom overlay with the query and match navigation; matches
  stay visible in the scrollback while typing.

## Scrollback + scrollbar

- `scrollback_lines 2000` default; pager integration for full-buffer viewing.
- `scrollbar scrolled` — the bar only appears after the user starts scrolling
  back; `scrollbar_handle_color foreground`, `scrollbar_handle_opacity 0.5`,
  `scrollbar_track_opacity 0` (invisible track), width 0.5 cells, rounded
  handle (`scrollbar_radius`), tiny gap from the edge.

## Cursor + text

- `cursor_shape` block/beam/underline (program-overridable via DECSCUSR),
  `cursor_shape_unfocused hollow`, `cursor_blink_interval -1` (system
  default), `cursor_stop_blinking_after 15.0`.
- `cursor` default `#cccccc`, `cursor_text_color` — the text under the cursor
  is tinted for contrast rather than relying on inversion alone.
- `mouse_hide_wait 3.0` — the pointer hides after 3s of idle, so the terminal
  never has a floating arrow over text.
- `url_color #0087bd`, `url_style curly` — hovered URLs get a distinct
  underline color/style; `show_hyperlink_targets` controls when the URL text
  is shown.

## Window chrome

- `window_padding_width 0`, `window_margin_width 0` — kitty's default padding
  is zero; text sits flush against the border, which is itself 0.5pt and only
  drawn between multiple windows. The compositor supplies window shadows.

---

## Mapping to ZeroTerm

| kitty element | ZeroTerm state (v0.3.10) | Note |
| - | - | - |
| `tab_bar_min_tabs 2` | **Implemented this pass** — bar hidden with one tab, chrome math follows | `set_tab_bar_visible` synced in `resize_panes_to_rects` + `render` |
| In-place search match highlight | **Implemented this pass** — all matches tinted with `search_match_bg`, current match with `selection_bg` | `SearchMatch` spans flow into `CellBatch`; active pane only |
| `visual_bell_duration`/`visual_bell_color` | **Implemented this pass** — `[terminal] visual_bell_ms` (default 150), background lerps to selection color with a fade-in/out envelope | kitty defaults the color to selection bg; eased like kitty |
| `bell_on_tab` 🔔 | Already had — bell dot on inactive tabs (latch, cleared on focus) | Equivalent behavior |
| Tab pills/separators | Already had — brightness-ladder pills + explicit separator cell | Richer than kitty's default `fade` |
| `scrollbar scrolled` | Already had — bar hidden while scrollback is trivial | kitty shows it after first scroll-back |
| Track opacity 0, handle from fg | Partial — muted accent thumb on surface track | Candidate follow-up |
| `mouse_hide_wait` | Not implemented | Candidate follow-up |
| `cursor_shape` + hollow unfocused | DECSCUSR shapes done; hollow-unfocused not | Candidate follow-up |
| Curly url underline (`url_style`) | Straight accent underline on hover | Style upgrade candidate |
| Zero padding (`window_padding_width 0`) | 16px padding | Deliberate deviation (breathing room) |
| No status bar | ZeroTerm has a status bar (exit chip, cwd, scroll) | Deliberate deviation — a product feature, not chrome bloat |

## What was implemented in this pass

1. **Kitty-style tab bar hiding** (`tab_bar_min_tabs = 2`): with a single tab
   the bar disappears and the grid gains the row; spawn-size estimates,
   viewport offsets, and pane rects all follow the dynamic height.
2. **Kitty-style in-place search highlighting**: opening search tints every
   match in the active pane's buffer (theme `search_match_bg`), with the
   current match brighter (reuses `selection_bg`); matches carry column
   spans so multi-occurrence rows highlight fully.
3. **Kitty-style visual bell**: `[terminal] visual_bell_ms` (default 150, 0 to
   disable) — on BEL the background fades toward the theme's selection color
   and back, kitty's exact color fallback and easing shape.
