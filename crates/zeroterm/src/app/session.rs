use std::collections::HashMap;
#[cfg(all(unix, feature = "ssh"))]
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use anyhow::Result;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;

use zeroterm_core::parser::Parser;
use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::Size as PtySize;
use zeroterm_mux::split::{SplitDir, SplitNode};
use zeroterm_mux::tab::Tab;

/// Commands the UI thread sends to the pty/ssh command thread.
pub enum PtyCommand {
    Write(Vec<u8>),
    Resize(PtySize),
    Kill,
}

pub struct PaneState {
    pub parser: Parser,
    pub pty_rx: Receiver<Vec<u8>>,
    pub pty_tx: Sender<PtyCommand>,
    pub title: String,
    pub pane_cmd: String,
    pub pty_dead: bool,
    /// Last (cols, rows) sent to the PTY. Resizes that don't change the size
    /// are skipped: every PTY resize delivers SIGWINCH, and bash reprints its
    /// prompt on each — the startup spawn-estimate → renderer-ready → Resized
    /// storm used to stack three prompts on top of each other.
    pub last_resize: Option<(usize, usize)>,
}
impl PaneState {
    /// Drain available pty output into the parser. Returns true if any bytes
    /// were parsed. Marks the pane dead once the pty channel disconnects so a
    /// dead pane is never drained twice (this is what stops the exit notice
    /// from being re-appended to the buffer on every subsequent drain call).
    pub fn drain(&mut self) -> bool {
        if self.pty_dead {
            return false;
        }
        let mut got = false;
        loop {
            match self.pty_rx.try_recv() {
                Ok(data) => {
                    self.parser.parse(&data);
                    got = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pty_dead = true;
                    break;
                }
            }
        }
        got
    }

    /// Resize the parser screen and forward to the PTY — but only when the
    /// size actually changed. Startup used to resize the PTY three times
    /// (spawn estimate, renderer-ready, first Resized event), and each resize
    /// makes bash reprint its prompt, stacking three prompts. Dedupe so a
    /// pane whose size is already correct stays quiet.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if self.last_resize == Some((cols, rows)) {
            return;
        }
        self.last_resize = Some((cols, rows));
        self.parser.screen_mut().resize(cols, rows);
        let _ = self.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
    }
}

pub fn starship_setup(
    shell: &str,
    shell_args: &[String],
) -> (String, Vec<String>, Vec<(&'static str, String)>) {
    let has_starship = std::process::Command::new("starship")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_starship {
        return (shell.to_string(), shell_args.to_vec(), Vec::new());
    }
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| std::path::Path::new(&h).join(".config"))
                .unwrap_or_default()
        });
    let user_starship = config_dir.join("starship.toml");
    let zt_starship = config_dir.join("zeroterm/starship.toml");
    if !user_starship.exists() && !zt_starship.exists() {
        if let Some(parent) = zt_starship.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&zt_starship, include_str!("../../assets/starship.toml"));
    }
    let mut env = Vec::new();
    if zt_starship.exists() {
        env.push((
            "STARSHIP_CONFIG",
            zt_starship.to_string_lossy().into_owned(),
        ));
    }
    env.push(("STARSHIP_SHELL", shell.to_string()));

    let init = if shell.ends_with("bash") {
        // bash --rcfile replaces ~/.bashrc for the interactive shell, but a
        // `-l` login shell IGNORES --rcfile and reads profile files instead
        // (verified in a PTY: with -l the rcfile never runs). So we run plain
        // interactive bash with --rcfile pointing at a bootstrap that
        // reproduces the login environment (/etc/profile, ~/.bash_profile),
        // sources the user's real ~/.bashrc, then enables starship. The old
        // `eval "$(starship init bash)"; exec bash -l` evaluated the init and
        // immediately discarded it across exec — the fresh login shell never
        // saw starship, so users got the stock bash prompt no matter what.
        let boot = config_dir.join("zeroterm/bashrc");
        if let Some(parent) = boot.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = r#"# ZeroTerm bootstrap: login-equivalent env + user rc + starship
if [ -f /etc/profile ]; then . /etc/profile; fi
profile_loaded=0
for f in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
  if [ -f "$f" ]; then . "$f"; profile_loaded=1; break; fi
done
# Source ~/.bashrc directly only if no login profile handled it: profiles
# commonly source ~/.bashrc themselves, and sourcing it twice re-runs
# aliases/exports/banners and can corrupt starship's precmd ordering.
if [ "$profile_loaded" -eq 0 ] && [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi
eval "$(starship init bash)"
"#;
        let _ = std::fs::write(&boot, content);
        Some(vec![r"--rcfile".into(), boot.to_string_lossy().into_owned()])
    } else if shell.ends_with("zsh") {
        // zsh has no --rcfile; ZDOTDIR points at a directory whose .zshrc
        // reproduces the login env (.zprofile), sources the user's real
        // .zshrc, then enables starship.
        let zdotdir = config_dir.join("zeroterm/zdotdir");
        if let Some(parent) = zdotdir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = r#"# ZeroTerm bootstrap: login-equivalent env + user rc + starship
if [ -f /etc/zprofile ]; then . /etc/zprofile; fi
if [ -f "$HOME/.zprofile" ]; then . "$HOME/.zprofile"; fi
if [ -f "$HOME/.zshrc" ]; then . "$HOME/.zshrc"; fi
eval "$(starship init zsh)"
"#;
        let _ = std::fs::write(zdotdir.join(".zshrc"), content);
        env.push(("ZDOTDIR", zdotdir.to_string_lossy().into_owned()));
        Some(vec![])
    } else {
        None
    };
    match init {
        Some(args) => (shell.to_string(), args, env),
        None => (shell.to_string(), shell_args.to_vec(), env),
    }
}

pub fn spawn_pty_process(
    shell: &str,
    shell_args: &[String],
    env: &[(&str, &str)],
    cols: usize,
    rows: usize,
    wake: EventLoopProxy<()>,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let shell_refs: Vec<&str> = shell_args.iter().map(|s| s.as_str()).collect();
    let mut backend = match PortablePtyBackend::new() {
        Ok(b) => b,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    let process = match backend.spawn(shell, &shell_refs, None, env) {
        Ok(p) => p,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    let (reader, mut process) = match process.split_reader() {
        Ok(r) => r,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    if let Err(e) = process.resize(PtySize { cols, rows }) {
        return Ok(spawn_err_channels(shell, e.to_string()));
    }

    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    // Dedicated reader thread: forward PTY output to the parser without ever
    // blocking the command channel (fixes the PTY I/O deadlock where a
    // blocking read starved pending keystrokes).
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = reader;
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    let _ = wake.send_event(());
                }
                Err(_) => break,
            }
        }
    });

    // Command thread: write/resize/kill only; blocks on recv, never on PTY.
    std::thread::spawn(move || {
        while let Ok(cmd) = input_rx.recv() {
            match cmd {
                PtyCommand::Write(data) => {
                    let _ = process.write(&data);
                }
                PtyCommand::Resize(size) => {
                    let _ = process.resize(size);
                }
                PtyCommand::Kill => {
                    let _ = process.kill();
                    return;
                }
            }
        }
        let _ = process.kill();
    });

    Ok((pty_rx, pty_tx))
}

/// Degraded spawn: return a fake channel that delivers one ANSI error message
/// and swallows all commands, so panes render "failed to spawn" instead of
/// dying silently. All `spawn_pty_process` call sites keep working unchanged.
fn spawn_err_channels(shell: &str, err: String) -> (Receiver<Vec<u8>>, Sender<PtyCommand>) {
    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let msg = format!(
        "\x1b[31m[zeroterm] failed to spawn shell '{}': {}\x1b[0m\r\n",
        shell, err
    );
    let _ = output_tx.send(msg.into_bytes());
    drop(output_tx);
    let (pty_tx, _discard) = mpsc::channel::<PtyCommand>();
    (pty_rx, pty_tx)
}

#[cfg(all(unix, feature = "ssh"))]
#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_process(
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    key_path: Option<&Path>,
    cols: usize,
    rows: usize,
    wake: EventLoopProxy<()>,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let host = host.to_string();
    let user = user.to_string();
    let password = password.map(|s| s.to_string());
    let key_path = key_path.map(|p| p.to_path_buf());

    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    std::thread::spawn(move || {
        let mut ssh = zeroterm_ssh::client::SshSession::new();
        if let Err(e) = ssh.connect(&host, port, &user, password.as_deref(), key_path.as_deref()) {
            let _ = output_tx.send(format!("\r\n\x1b[31mSSH: {}\x1b[0m\r\n", e).into_bytes());
            let _ = wake.send_event(());
            return;
        }
        if let Err(e) = ssh.resize(cols as u32, rows as u32) {
            let _ =
                output_tx.send(format!("\r\n\x1b[31mSSH resize: {}\x1b[0m\r\n", e).into_bytes());
            let _ = wake.send_event(());
        }
        // An idle SSH channel must not starve keystrokes, so bound the read
        // (mirrors the PTY fix: no blocking call ever holds up the command
        // channel).
        ssh.set_timeout(50);

        let mut buf = [0u8; 65536];
        loop {
            while let Ok(cmd) = input_rx.try_recv() {
                match cmd {
                    PtyCommand::Write(data) => {
                        if ssh.write(&data).is_err() {
                            return;
                        }
                    }
                    PtyCommand::Resize(size) => {
                        let _ = ssh.resize(size.cols as u32, size.rows as u32);
                    }
                    PtyCommand::Kill => {
                        let _ = ssh.disconnect();
                        return;
                    }
                }
            }
            match ssh.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    let _ = wake.send_event(());
                }
                Err(e) => match e.downcast_ref::<std::io::Error>() {
                    Some(ioe)
                        if ioe.kind() == std::io::ErrorKind::WouldBlock
                            || ioe.kind() == std::io::ErrorKind::TimedOut => {}
                    _ => break,
                },
            }
        }
        let _ = ssh.disconnect();
    });

    Ok((pty_rx, pty_tx))
}

/// Owns the pane/tab/split tree plus the shared view state. Pure tree and
/// navigation logic lives here; anything that needs the renderer/window
/// (redraws, hit-testing, resize math) stays in `App` and delegates to a
/// `SessionManager` method.
///
/// The split tree is PRIVATE: `App` may only reshape it through the ops below
/// (`split_active`, `dock_pane`, `float_pane`, `remove_pane_from_tree`, …),
/// each of which reconciles the tree against the live pane map before
/// returning. Direct field mutation from the app is what let a stale tree
/// blank the window (the old `reconcile_tree` repair pass was bolted on after
/// the fact); with a closed surface the tree↔panes invariant holds by
/// construction.
/// Outcome of a pane close: the removed `PaneState` (caller kills its pty)
/// plus whether it was the active pane (caller cancels the editor).
pub struct CloseEffect {
    pub pane: PaneState,
    pub was_active: bool,
}

pub struct SessionManager {
    pub panes: HashMap<usize, PaneState>,
    pub active_pane: usize,
    pub next_pane_id: usize,
    pub tabs: Vec<Tab>,
    // ponytail: per-pane scroll kept as single field, inactive panes render at offset 0
    split_root: SplitNode,
    // ponytail: no mouse hit-testing on the overlay rect; keyboard focus only
    pub floating: Option<usize>,
    // Split divider drag: Some(target) = dragging the divider whose first leaf
    // is `target`; anchor is the last window-space mouse position.
    pub dragging_divider: Option<usize>,
    pub divider_anchor: (f32, f32),
    pub scroll_offset: usize,
}

#[allow(dead_code)] // accessor surface grows as 1.7 composition (AppState) lands
impl SessionManager {
    pub fn new() -> Self {
        Self {
            panes: HashMap::new(),
            active_pane: 0,
            next_pane_id: 1,
            tabs: Vec::new(),
            split_root: SplitNode::Leaf(0),
            floating: None,
            dragging_divider: None,
            divider_anchor: (0.0, 0.0),
            scroll_offset: 0,
        }
    }

    pub fn active_pane(&self) -> Option<&PaneState> {
        self.panes.get(&self.active_pane)
    }

    pub fn active_pane_mut(&mut self) -> Option<&mut PaneState> {
        self.panes.get_mut(&self.active_pane)
    }

    pub fn pane(&self, id: usize) -> Option<&PaneState> {
        self.panes.get(&id)
    }

    pub fn pane_mut(&mut self, id: usize) -> Option<&mut PaneState> {
        self.panes.get_mut(&id)
    }

    /// Sorted pane ids — the canonical tab order.
    pub fn pane_ids(&self) -> Vec<usize> {
        let mut keys: Vec<usize> = self.panes.keys().copied().collect();
        keys.sort();
        keys
    }

    /// Read-only tree access for persistence (layout.json) — the app never
    /// mutates the tree through this handle.
    pub fn tree(&self) -> &SplitNode {
        &self.split_root
    }

    /// Replace the whole tree, e.g. with a remapped session-restore layout.
    pub fn set_tree(&mut self, root: SplitNode) {
        self.split_root = root;
        self.reconcile_tree();
    }

    pub fn rects(&self) -> HashMap<usize, (f32, f32, f32, f32)> {
        self.split_root.compute_rects()
    }

    pub fn leaves(&self) -> Vec<usize> {
        self.split_root.leaves()
    }

    /// Pane id containing the normalized content-space point, if any.
    pub fn pane_at(&self, x: f32, y: f32) -> Option<usize> {
        self.split_root.pane_at(x, y)
    }

    pub fn dividers(&self) -> Vec<(SplitDir, f32, usize)> {
        self.split_root.dividers()
    }

    /// Resize the divider whose second-child first leaf is `target`. Returns
    /// true when a matching divider was found and adjusted.
    pub fn resize_divider(&mut self, target: usize, boundary: f32, delta: f32) -> bool {
        self.split_root.resize_leaf(target, boundary, delta)
    }

    /// Advance to the next tab in sorted order (wraps). Returns true if the
    /// active pane actually changed.
    pub fn next_tab(&mut self) -> bool {
        let keys = self.pane_ids();
        if keys.len() <= 1 {
            return false;
        }
        let pos = keys
            .iter()
            .position(|k| *k == self.active_pane)
            .unwrap_or(0);
        let next = (pos + 1) % keys.len();
        if keys[next] == self.active_pane {
            return false;
        }
        self.active_pane = keys[next];
        self.scroll_offset = 0;
        true
    }

    /// Move to the previous tab in sorted order (wraps). Returns true if the
    /// active pane actually changed.
    pub fn previous_tab(&mut self) -> bool {
        let keys = self.pane_ids();
        if keys.len() <= 1 {
            return false;
        }
        let pos = keys
            .iter()
            .position(|k| *k == self.active_pane)
            .unwrap_or(0);
        let prev = if pos == 0 { keys.len() - 1 } else { pos - 1 };
        if keys[prev] == self.active_pane {
            return false;
        }
        self.active_pane = keys[prev];
        self.scroll_offset = 0;
        true
    }

    /// Select the tab at sorted index `idx`. Returns true if it changed.
    pub fn switch_to_tab(&mut self, idx: usize) -> bool {
        let keys = self.pane_ids();
        if idx >= keys.len() || keys[idx] == self.active_pane {
            return false;
        }
        self.active_pane = keys[idx];
        self.scroll_offset = 0;
        true
    }

    /// Move focus to the pane nearest `dir` using rect centers. Returns true
    /// if focus moved.
    pub fn focus_adjacent_pane(&mut self, dir: KeyCode) -> bool {
        let rects = self.split_root.compute_rects();
        if rects.len() <= 1 {
            return false;
        }
        let cur = self.active_pane;
        let cur_rect = match rects.get(&cur) {
            Some(r) => *r,
            None => return false,
        };
        let cx = cur_rect.0 + cur_rect.2 / 2.0;
        let cy = cur_rect.1 + cur_rect.3 / 2.0;
        let mut best: Option<(usize, f32)> = None;
        for (&id, &(x, y, w, h)) in &rects {
            if id == cur {
                continue;
            }
            let px = x + w / 2.0;
            let py = y + h / 2.0;
            let dx = px - cx;
            let dy = py - cy;
            let (in_dir, dist) = match dir {
                KeyCode::ArrowLeft if dx < 0.0 => (true, -dx + dy.abs()),
                KeyCode::ArrowRight if dx > 0.0 => (true, dx + dy.abs()),
                KeyCode::ArrowUp if dy < 0.0 => (true, -dy + dx.abs()),
                KeyCode::ArrowDown if dy > 0.0 => (true, dy + dx.abs()),
                _ => (false, 0.0),
            };
            if in_dir && best.map_or(true, |(_, b)| dist < b) {
                best = Some((id, dist));
            }
        }
        if let Some((id, _)) = best {
            self.active_pane = id;
            self.scroll_offset = 0;
            true
        } else {
            false
        }
    }

    pub fn max_scroll_offset(&self) -> usize {
        if let Some(pane) = self.active_pane() {
            let screen = pane.parser.screen();
            let total_rows = screen.scrollback().len() + screen.buffer().len();
            let visible_rows = screen.size().rows;
            total_rows.saturating_sub(visible_rows)
        } else {
            0
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let max = self.max_scroll_offset();
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(max);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Insert a freshly spawned pane into the split tree as a `dir` split of
    /// the active pane. Insert first, then reconcile: normally the tree is in
    /// sync (close_tab/drain reconcile it), so the new leaf lands next to the
    /// active pane; if the tree was stale the insert is a no-op and the
    /// reconcile rebuilds it from the full pane set. Either way the new pane
    /// renders exactly once.
    pub fn insert_pane_as_split(&mut self, new_id: usize, dir: SplitDir) {
        let parent = self.active_pane;
        self.split_root.insert_leaf(new_id, dir, parent, 0.5);
        self.reconcile_tree();
    }

    /// Remove a pane from the tree (after its PaneState is dropped) and
    /// reconcile so the tree never keeps a dead leaf — the stale-sole-leaf
    /// case that blanked the window.
    pub fn remove_pane_from_tree(&mut self, id: usize) {
        self.split_root.remove_leaf(id);
        self.reconcile_tree();
    }

    /// Re-insert a floating (or otherwise absent) pane into the tree at the
    /// first remaining leaf, then clear the floating slot.
    pub fn dock_pane(&mut self, id: usize) {
        if !self.split_root.leaves().contains(&id) {
            let parent = *self.split_root.leaves().first().unwrap_or(&id);
            self.split_root.insert_leaf(id, SplitDir::Vertical, parent, 0.5);
        }
        self.floating = None;
        self.reconcile_tree();
    }

    /// Float the pane: remove it from the tree and mark it floating. Returns
    /// false when it was the tree's only leaf (the overlay stays in the tree
    /// so at least one pane renders).
    pub fn float_pane(&mut self, id: usize) -> bool {
        if self.split_root.leaves().len() <= 1 {
            return false;
        }
        self.split_root.remove_leaf(id);
        self.floating = Some(id);
        self.reconcile_tree();
        true
    }

    /// Rebuild the split tree from the pane map whenever its leaves have
    /// drifted out of sync — e.g. after `close_tab` removed the tree's only
    /// leaf (remove_leaf leaves the tree pointing at the removed pane, which
    /// would blank the screen) or after any structural change.
    ///
    /// The floating pane is intentionally absent from the tree (it renders as
    /// the fullscreen overlay), so it is excluded from the comparison and the
    /// rebuild — otherwise a drain-triggered reconcile would silently dock it
    /// back into the tree and double-render it.
    pub fn reconcile_tree(&mut self) {
        let mut ids = self.pane_ids();
        if ids.is_empty() {
            return;
        }
        ids.retain(|id| self.floating != Some(*id));
        let mut leaves = self.split_root.leaves();
        leaves.sort_unstable();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        if leaves != sorted_ids {
            self.split_root = SplitNode::from_ids(&ids);
        }
    }

    /// Assign an id to a freshly spawned pane, register it, insert it into
    /// the split tree next to the active pane, focus it, and (optionally) add
    /// its tab. The caller provides the already-spawned `PaneState` (PTY
    /// channels + parser) — id allocation and tree/tab bookkeeping are the
    /// session's invariants, so no external code can desync them.
    pub fn register_pane(&mut self, pane: PaneState, dir: SplitDir, push_tab: bool) -> usize {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes.insert(id, pane);
        self.active_pane = id;
        self.scroll_offset = 0;
        self.insert_pane_as_split(id, dir);
        if push_tab {
            self.tabs.push(Tab::new(id));
        }
        id
    }

    /// Close a pane: drop it from the map, remove its tree leaf (reconciling
    /// so no dead id survives), drop its tab, clear the floating slot if it
    /// was floating, and refocus the lowest remaining pane when it was active.
    /// Refuses to close the last pane. Returns None when the pane is missing
    /// or the session would be emptied.
    pub fn close_pane(&mut self, id: usize) -> Option<CloseEffect> {
        if self.panes.len() <= 1 {
            return None;
        }
        let was_active = self.active_pane == id;
        let pane = self.panes.remove(&id)?;
        // Removes the leaf AND reconciles: the tree never keeps a dead id,
        // so rendering never dereferences a removed pane.
        self.remove_pane_from_tree(id);
        self.tabs.retain(|t| t.id != id);
        if self.floating == Some(id) {
            self.floating = None;
        }
        if was_active {
            self.active_pane = *self.panes.keys().next().unwrap_or(&0);
            self.scroll_offset = 0;
        }
        Some(CloseEffect { pane, was_active })
    }

    /// Toggle the active pane between the split tree and the floating
    /// overlay. Mirrors the old App-level dance: dock whatever was floating
    /// (one float at a time), then float the active pane; the last visible
    /// pane stays in the tree AND floats (overlay wins when drawn twice) so
    /// zero visible panes are impossible.
    pub fn toggle_floating(&mut self) {
        let active = self.active_pane;
        if self.floating == Some(active) {
            // Dock: re-insert at the first remaining tree leaf.
            self.dock_pane(active);
        } else {
            // Dock whatever was floating, then float active.
            if let Some(prev) = self.floating.take() {
                self.dock_pane(prev);
            }
            if !self.float_pane(active) {
                // ponytail: last visible pane stays in tree AND floats
                // (overlay wins when drawn twice); zero visible panes not
                // allowed.
                self.floating = Some(active);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_pane_as_split_adds_leaf_next_to_active() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state()); // caller inserts the pane first
        m.active_pane = 0;
        m.insert_pane_as_split(1, SplitDir::Vertical);
        assert_eq!(m.split_root.leaves(), vec![0, 1]);
        // The new pane's rect exists and shares the screen.
        let rects = m.split_root.compute_rects();
        assert!(rects.contains_key(&1));
        assert!((rects[&0].2 + rects[&1].2 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn insert_pane_as_split_with_stale_tree_still_renders_new_pane() {
        // Tree points at a dead pane 1 (post-close); inserting pane 2 with the
        // stale tree must still land pane 2 in the tree exactly once.
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(2, pane_state());
        m.split_root = SplitNode::Leaf(1); // stale
        m.active_pane = 0;
        m.insert_pane_as_split(2, SplitDir::Vertical);
        let mut leaves = m.split_root.leaves();
        leaves.sort_unstable();
        assert_eq!(leaves, vec![0, 2], "no dead id, no duplicate leaf");
    }

    #[test]
    fn reconcile_tree_repairs_stale_sole_leaf() {
        // close_tab removes the pane then remove_leaf; when the removed pane
        // was the tree's only leaf the tree is left pointing at a dead id.
        // reconcile_tree must rebuild it from the live panes.
        let mut m = SessionManager::new();
        // Pane 1 was closed: removed from the map, but the tree was left
        // pointing at it (the sole-leaf close path).
        m.panes.insert(0, pane_state());
        m.split_root = SplitNode::Leaf(1);
        m.split_root.remove_leaf(1);
        assert_eq!(
            m.split_root.leaves(),
            vec![1],
            "stale leaf survives remove_leaf"
        );
        m.reconcile_tree();
        assert_eq!(m.split_root.leaves(), vec![0], "tree repaired to live pane");
    }

    #[test]
    fn reconcile_tree_is_noop_when_in_sync() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.split_root = SplitNode::from_ids(&[0, 1]);
        m.reconcile_tree();
        assert_eq!(m.split_root.leaves(), vec![0, 1]);
    }

    #[test]
    fn reconcile_tree_rebuilds_after_pane_drop() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.panes.insert(2, pane_state());
        m.split_root = SplitNode::from_ids(&[0, 1, 2]);
        m.panes.remove(&1);
        m.reconcile_tree();
        let mut leaves = m.split_root.leaves();
        leaves.sort_unstable();
        assert_eq!(leaves, vec![0, 2]);
    }

    #[test]
    fn reconcile_tree_preserves_floating_pane() {
        // The floating pane is intentionally absent from the tree (it renders
        // as the overlay). A drain-triggered reconcile must not dock it back.
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.split_root = SplitNode::Leaf(0);
        m.floating = Some(1);
        m.reconcile_tree();
        assert_eq!(
            m.split_root.leaves(),
            vec![0],
            "floating pane must stay out of the tree"
        );
    }

    #[test]
    fn insert_pane_as_split_keeps_floating_pane_floating() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.panes.insert(2, pane_state());
        m.split_root = SplitNode::Leaf(0);
        m.floating = Some(1);
        m.active_pane = 0;
        m.insert_pane_as_split(2, SplitDir::Vertical);
        let mut leaves = m.split_root.leaves();
        leaves.sort_unstable();
        assert_eq!(leaves, vec![0, 2], "new pane in tree, float preserved");
        assert_eq!(m.floating, Some(1));
    }

    fn pane_state() -> PaneState {
        let (_tx, rx) = mpsc::channel();
        PaneState {
            parser: Parser::new(80, 24),
            pty_rx: rx,
            pty_tx: mpsc::channel().0,
            title: String::new(),
            pane_cmd: String::new(),
            pty_dead: false,
            last_resize: None,
        }
    }

    #[test]
    fn resize_skips_unchanged_size() {
        // Regression: the startup resize storm re-sent the same PTY size over
        // and over, and bash reprints its prompt on every SIGWINCH. The dedupe
        // must skip sizes already sent and still deliver real changes.
        let (tx, rx) = mpsc::channel::<PtyCommand>();
        let mut pane = PaneState {
            parser: Parser::new(80, 24),
            pty_rx: mpsc::channel().1,
            pty_tx: tx,
            title: String::new(),
            pane_cmd: String::new(),
            pty_dead: false,
            last_resize: None,
        };
        pane.resize(80, 24);
        assert!(matches!(rx.try_recv(), Ok(PtyCommand::Resize(_))));
        // Same size again: must NOT re-send (no second SIGWINCH).
        pane.resize(80, 24);
        assert!(rx.try_recv().is_err(), "duplicate resize must be skipped");
        // A real change still goes through.
        pane.resize(90, 30);
        assert!(matches!(rx.try_recv(), Ok(PtyCommand::Resize(_))));
    }

    #[test]
    fn register_pane_assigns_id_and_keeps_session_consistent() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.active_pane = 0;
        let id = m.register_pane(pane_state(), SplitDir::Vertical, true);
        assert_eq!(id, 1);
        assert_eq!(m.next_pane_id, 2);
        assert_eq!(m.active_pane, 1, "new pane is focused");
        assert_eq!(m.scroll_offset, 0);
        let mut leaves = m.split_root.leaves();
        leaves.sort_unstable();
        assert_eq!(leaves, vec![0, 1], "pane registered in the tree");
        assert!(m.tabs.iter().any(|t| t.id == 1));
    }

    #[test]
    fn register_pane_without_tab_keeps_tabs_empty() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.active_pane = 0;
        m.register_pane(pane_state(), SplitDir::Horizontal, false);
        assert!(m.tabs.is_empty(), "split panes are not tabs");
    }

    #[test]
    fn close_pane_removes_leaf_and_refocuses() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.split_root = SplitNode::from_ids(&[0, 1]);
        m.active_pane = 1;
        let effect = m.close_pane(1).expect("pane 1 closes");
        assert!(effect.was_active);
        assert_eq!(m.active_pane, 0, "focus moves to the remaining pane");
        assert_eq!(m.panes.len(), 1);
        assert_eq!(m.split_root.leaves(), vec![0], "dead leaf removed");
    }

    #[test]
    fn close_pane_refuses_to_close_last_pane() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        assert!(m.close_pane(0).is_none(), "last pane cannot close");
        assert_eq!(m.panes.len(), 1);
    }

    #[test]
    fn close_pane_clears_floating_slot() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.split_root = SplitNode::Leaf(0);
        m.floating = Some(1);
        m.active_pane = 1;
        m.close_pane(1);
        assert_eq!(m.floating, None, "floating slot cleared on close");
    }

    #[test]
    fn toggle_floating_docks_then_floats() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.panes.insert(1, pane_state());
        m.split_root = SplitNode::from_ids(&[0, 1]);
        m.active_pane = 1;
        m.toggle_floating();
        assert_eq!(m.floating, Some(1), "active pane floats");
        assert_eq!(m.split_root.leaves(), vec![0], "floated pane leaves tree");
        // Toggle again: docks and floats the (same) active pane.
        m.toggle_floating();
        assert_eq!(m.split_root.leaves().len(), 2, "pane back in tree");
    }

    #[test]
    fn toggle_floating_keeps_last_visible_pane_in_tree() {
        let mut m = SessionManager::new();
        m.panes.insert(0, pane_state());
        m.active_pane = 0;
        m.toggle_floating();
        assert_eq!(
            m.split_root.leaves(),
            vec![0],
            "sole pane stays in the tree while floating"
        );
        assert_eq!(m.floating, Some(0));
    }

    #[test]
    fn spawn_err_channels_delivers_ansi_error_and_swallows_commands() {
        let (rx, tx) = spawn_err_channels("definitely-not-a-shell", "no such file".into());
        let chunk = rx.recv().unwrap();
        let text = String::from_utf8(chunk).unwrap();
        assert!(text.contains("failed to spawn shell 'definitely-not-a-shell'"));
        assert!(text.contains("no such file"));
        assert!(text.starts_with("\x1b[31m"));
        assert!(
            rx.recv().is_err(),
            "channel closes after the single error chunk"
        );

        // Commands go nowhere: the fake channel's command receiver is dropped.
        assert!(tx.send(PtyCommand::Write(vec![b'x'])).is_err());
        assert!(tx.send(PtyCommand::Kill).is_err());
    }
}
