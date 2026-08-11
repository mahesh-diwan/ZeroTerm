# ZeroTerm Domain Glossary

Canonical vocabulary for the ZeroTerm terminal emulator. When you talk or write
about ZeroTerm, use these terms with these meanings. Terms marked
*alias to avoid* are overloaded or legacy words that keep meaning something
else — prefer the canonical term.

## The workspace

- **Pane** — the smallest renderable unit of the terminal: one PTY-backed
  terminal viewport with its own grid, scrollback, and shell process.
  *Alias to avoid:* "session" (a pane is not a session; see Session).
- **Tab** — a classic tab that *owns* a set of panes plus their split tree,
  and renders full-window when selected. Switching tabs swaps the whole view.
- **Split** — dividing a tab's space into sibling panes along a direction
  (horizontal or vertical), with a draggable **divider** between them.
- **Split tree** — the nested split structure of one tab; its leaves are that
  tab's panes.
- **Floating pane** — a pane temporarily lifted out of the tiled split tree
  and overlaid on top of the other panes.
- **Session** — the whole app-level arrangement: every tab, every pane, and
  the focus state (active tab, active pane, floating pane). One session per
  window.
  *Alias to avoid:* "SSH session" (say **remote connection**), "sync session"
  (say **sync peer**), and "saved session" (say **saved layout**).
- **Saved layout** — the serialized form of a session (tabs, panes, split
  trees, active tab), written on quit and read back on launch when restore is
  on.
- **Session restore** — the opt-in behavior of rebuilding tabs and splits
  from the saved layout on launch. Off by default: every launch starts with a
  single fresh tab.
- **Pane spec** — the serializable description of one pane (command, title,
  working directory) used by session restore to re-spawn it.

## Terminal state (per pane)

- **Grid** — the visible character buffer of a pane: the rows and columns of
  cells the renderer draws.
- **Scrollback** — the pane's history buffer above the grid. Scrolling
  reveals it; the grid's bottom edge stays anchored to the cursor.
- **Alternate screen** — the second buffer full-screen programs (vim, htop,
  tmux, fzf) switch to via the alternate-screen mode. While active, the
  normal grid and its scrollback are suspended.
- **Cursor** — the input position, with a shape (block, underline, bar),
  blink, and visibility, controlled by the program via the cursor-style
  protocol (DECSCUSR).
- **Command block** — the lifecycle of one shell command: its start line,
  end line, command text, exit code, and duration. Built from shell
  integration markers, with a prompt-sigil heuristic as fallback.
- **Last exit** — the sticky exit code of the most recently finished command
  block; survives until the next command reports a code. Shown as the status
  chip and the tab failure dot.
- **Working directory (cwd)** — the directory the shell reports via the
  working-directory announcement (OSC 7); shown in the status bar and usable
  to seed new tabs.
- **Hyperlink** — a clickable link (OSC 8) on a range of cells. Cells carry a
  link id that resolves to a URI through the pane's link registry; hovering
  shows the URI, a plain click opens it.
- **Link registry** — the pane's bounded table mapping link ids to URIs.
- **Bell latch** — a per-pane latch set when the shell rings the bell and
  cleared when the tab gains focus; surfaces as the activity dot on the tab.
- **Selection** — the user-chosen character range in the grid, for copy
  (dragging selects; a click without drag opens a hyperlink instead).
- **Bracketed paste** — the paste mode in which the shell wraps pasted text
  in start/end markers so it is inserted literally, never interpreted as
  keys.
- **Sync output** — the synchronized-update mode in which output drains and
  renders as a batch instead of flickering line by line.
- **Mouse tracking** — the reporting modes that forward mouse events to the
  shell (used by TUI apps for clickable UI).
- **Shell integration** — the command-block protocol (OSC 133) plus the
  working-directory announcement (OSC 7) that give ZeroTerm knowledge of
  prompts, commands, exit codes, and cwd.
- **Bootstrap** — the ZeroTerm-generated shell startup file that reproduces
  a login environment, sources the user's real shell rc, enables the prompt
  (starship), and installs the shell-integration hooks. Written per shell
  (bash, zsh) under the config directory.

## Window chrome

- **Tab bar** — the single-cell strip at the top of the window: one pill per
  tab with separators, a close glyph, and activity/failure dots. Hidden while
  only one tab exists (kitty `tab_bar_min_tabs`), so a single pane gets the
  full grid height.
- **Status bar** — the single-cell strip at the bottom: active pane title,
  the exit chip (✓/✗), the hovered hyperlink URI, the scroll indicator, and
  the active overlay mode marker.
- **Chrome** — the fixed tab-bar and status-bar rows subtracted from the
  window height to get the content area. Every size calculation (grid
  dimensions, PTY spawn size) accounts for chrome.
- **Overlay** — a modal layer (search, settings, host picker) drawn on top of
  the grid; exactly one overlay owns the screen at a time.
- **Focus-follow** — the config-gated behavior of switching the active pane
  to whichever pane the mouse hovers, when not drag-selecting.

## Feature names

- **Kitty keyboard protocol** — the opt-in keyboard enhancement protocol
  (CSI-u) that lets apps disambiguate modified keys; enabled per app, legacy
  shells are unaffected.
- **Notifications** — desktop notifications for the notification escape
  (OSC 9); bursts are collapsed and capped.
- **Search match** — one in-buffer occurrence of the search query: a column
  span on a global row. While search is open, every match is highlighted in
  place; the current match reads brighter.
- **Visual bell** — the background flash shown when the shell rings the bell
  (kitty `visual_bell_duration`): the background fades toward the selection
  color and back over `[terminal] visual_bell_ms`; the tab dot still marks
  inactive tabs.
