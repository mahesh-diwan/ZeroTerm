use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use anyhow::Result;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;

use zeroterm_core::parser::Parser;
use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::Size as PtySize;
use zeroterm_mux::split::SplitNode;
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
        Some(r#"eval "$(starship init bash)"; exec bash -l"#)
    } else if shell.ends_with("zsh") {
        Some(r#"eval "$(starship init zsh)"; exec zsh -l"#)
    } else {
        None
    };
    match init {
        Some(init) => (shell.to_string(), vec!["-c".into(), init.into()], env),
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

    let (output_tx, pty_rx) = mpsc::sync_channel::<Vec<u8>>(4);
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
    let (output_tx, pty_rx) = mpsc::sync_channel::<Vec<u8>>(4);
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

    let (output_tx, pty_rx) = mpsc::sync_channel::<Vec<u8>>(4);
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
pub struct SessionManager {
    pub panes: HashMap<usize, PaneState>,
    pub active_pane: usize,
    pub next_pane_id: usize,
    pub tabs: Vec<Tab>,
    // ponytail: per-pane scroll kept as single field, inactive panes render at offset 0
    pub split_root: SplitNode,
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

    pub fn compute_split_rects(&self) -> HashMap<usize, (f32, f32, f32, f32)> {
        self.split_root.compute_rects()
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
