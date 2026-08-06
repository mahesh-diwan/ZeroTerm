# Changelog

All notable changes to ZeroTerm are documented here. Releases are tagged
`vX.Y.Z`; the most recent tag is published as a
[GitHub Release](https://github.com/mahesh-diwan/ZeroTerm/releases) with
prebuilt binaries by the CI pipeline.

## [v0.3.0] - 2026-08-06

### Bug fixes

- **Split panes now actually appear.** `Ctrl+Shift+E`/`Ctrl+Shift+D` created a
  PTY and a pane entry but never inserted the pane into the split tree, so the
  new pane rendered nothing. New panes are now wired into the tree, and the
  tree is reconciled whenever a pane is closed — fixing a blank-screen state
  that occurred when the last split was closed.
- **`clear` no longer wipes scrollback.** Erase Display mode 2 (`ESC [ 2 J`)
  erases only the visible screen; only mode 3 (`ESC [ 3 J`, "erase saved
  lines") clears the scrollback.
- **Insert/Delete Line operate at the cursor row.** `ESC [ n L` / `ESC [ n M`
  previously operated from the top of the scroll region instead of the cursor
  row, as VT100 requires.
- **Window resizes preserve scrollback.** Shrinking the window previously
  dropped all history; the resize path now pushes overflow rows into the
  scrollback (and pulls from it when growing), without leaking alternate-screen
  rows into history.
- **Invalid UTF-8 no longer swallows following text.** A lone or malformed
  byte used to cause subsequent valid text to be dropped; it now emits U+FFFD
  and resynchronizes, matching the behavior of vte/foot/alacritty.
- **Block dividers render at the correct row** when the view is scrolled
  (previously the divider tint appeared on the wrong line).
- **Floating panes stay floating.** The split-tree reconciliation ignored the
  detached floating pane and silently re-docked it on the next redraw.
- **Kitty images are no longer truncated** at 4 KB: the APC buffer cap was
  raised to 4 MiB (bounded by existing decode caps), and escape-sequence
  intermediates are cleared when entering a new state.
- **Window title tracks the release version** instead of a hardcoded string.

### Tooling & release

- The release pipeline now publishes a GitHub Release with all build artifacts
  (AppImage, macOS zip, Windows zip, .deb, .rpm, Flatpak bundle, systemd unit)
  when a tag is pushed.
- `scripts/install.sh` was rewritten: it resolves the latest release tag from
  the GitHub API, downloads the prebuilt package for the platform, and falls
  back to building from source at that tag. Supports `ZEROTERM_VERSION` and
  `ZEROTERM_INSTALL_DIR` overrides and an `upgrade` mode.
- README and landing page corrected to reflect reality (repo URLs, available
  install methods, verified shortcuts, platform status).

## [v0.2.0] - 2026-07-30

Initial tagged release. Feature set at this tag:

- GPU-accelerated rendering via wgpu (Metal/DX12/Vulkan), glyph atlas, themes.
- VT100/ANSI parser, screen buffer with scrollback, image protocols
  (Kitty/Sixel/iTerm2), Unicode support.
- Tabs, tiled split panes, session persistence and restore.
- Multi-line/readline-style line editor with history, syntax highlighting.
- Local AI completion/explain overlay (Ollama/LM Studio), settings overlay.
- Native SSH client (Unix) with agent forwarding and daemon mode.
- E2E-encrypted settings sync (ChaCha20-Poly1305), WASM plugin sandbox.
