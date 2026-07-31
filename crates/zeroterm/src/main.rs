use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use anyhow::Result;
use arboard::Clipboard;
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::event::{MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes};

use zeroterm_ai::client::AiClient;
use zeroterm_config::Config;
use zeroterm_core::parser::MouseTrackingMode;
use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::{CommandBlock, Size as PtySize};
use zeroterm_core::Parser;
use zeroterm_mux::split::{SplitDir, SplitNode};
use zeroterm_mux::tab::Tab;
use zeroterm_render::{Renderer, Selection};
use zeroterm_sync::daemon::SyncDaemon;

use crate::settings::{SettingsAction, SettingsContext, SettingsMenu};

mod session;
mod settings;

const COPY_MARKER: &str = "[copy]";

fn block_output_text(screen: &zeroterm_core::screen::Screen, block: &CommandBlock) -> String {
    let buffer = screen.buffer();
    let last = buffer.len().saturating_sub(1);
    let end = block
        .end_line
        .map_or(last, |e| e.saturating_sub(1))
        .min(last);
    let start = block.start_line.min(end);
    let mut text = String::new();
    for row in start..=end {
        if let Some(line) = buffer.get(row) {
            for cell in line {
                text.push(cell.ch);
            }
        }
        text.push('\n');
    }
    text.trim_end().to_string()
}

enum PtyCommand {
    Write(Vec<u8>),
    Resize(PtySize),
    Kill,
}

struct PaneState {
    parser: Parser,
    pty_rx: Receiver<Vec<u8>>,
    pty_tx: Sender<PtyCommand>,
    title: String,
    pane_cmd: String,
}

fn spawn_pty_process(
    shell: &str,
    shell_args: &[String],
    cols: usize,
    rows: usize,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let shell_refs: Vec<&str> = shell_args.iter().map(|s| s.as_str()).collect();
    let mut backend = PortablePtyBackend::new()?;
    let mut process = backend.spawn(shell, &shell_refs, None)?;
    process.resize(PtySize { cols, rows })?;

    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            while let Ok(cmd) = input_rx.try_recv() {
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
            match process.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok((pty_rx, pty_tx))
}

fn spawn_ssh_process(
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    key_path: Option<&Path>,
    cols: usize,
    rows: usize,
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
            return;
        }
        if let Err(e) = ssh.resize(cols as u32, rows as u32) {
            let _ =
                output_tx.send(format!("\r\n\x1b[31mSSH resize: {}\x1b[0m\r\n", e).into_bytes());
        }

        let mut buf = [0u8; 4096];
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
                }
                Err(_) => break,
            }
        }
        let _ = ssh.disconnect();
    });

    Ok((pty_rx, pty_tx))
}

#[allow(dead_code)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    panes: HashMap<usize, PaneState>,
    active_pane: usize,
    next_pane_id: usize,
    tabs: Vec<Tab>,
    // ponytail: split rendering shows active pane fullscreen; per-pane viewport rendering when renderer supports it
    split_root: SplitNode,
    modifiers: ModifiersState,
    scroll_offset: usize,
    font_size: f32,
    selection: Option<Selection>,
    selecting: bool,
    mouse_pos: (f32, f32),
    clipboard: Option<Clipboard>,
    shell: String,
    shell_args: Vec<String>,
    ai_client: Option<Arc<AiClient>>,
    sync_daemon: Option<SyncDaemon>,
    config_changed: Arc<AtomicBool>,
    config: Option<Config>,
    opacity: f64,
    sync_tick: u32,
    cursor_visible: bool,
    font_path: Option<String>,
    settings: SettingsMenu,
}

#[allow(dead_code)]
impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            panes: HashMap::new(),
            active_pane: 0,
            next_pane_id: 1,
            tabs: Vec::new(),
            split_root: SplitNode::Leaf(0),
            modifiers: ModifiersState::empty(),
            scroll_offset: 0,
            font_size: 14.0,
            selection: None,
            selecting: false,
            mouse_pos: (0.0, 0.0),
            clipboard: Clipboard::new().ok(),
            shell: String::new(),
            shell_args: vec![],
            ai_client: None,
            sync_daemon: None,
            config_changed: Arc::new(AtomicBool::new(false)),
            config: None,
            opacity: 1.0,
            sync_tick: 0,
            cursor_visible: true,
            font_path: None,
            settings: SettingsMenu::new(&SettingsContext::default()),
        }
    }

    fn active_pane(&self) -> Option<&PaneState> {
        self.panes.get(&self.active_pane)
    }

    fn active_pane_mut(&mut self) -> Option<&mut PaneState> {
        self.panes.get_mut(&self.active_pane)
    }

    fn update_window_title(&self) {
        if let Some(pane) = self.active_pane() {
            let title = pane.parser.screen().title();
            if let Some(window) = &self.window {
                if title.is_empty() {
                    window.set_title("ZeroTerm v0.2.0");
                } else {
                    window.set_title(&format!("ZeroTerm v0.2.0 - {}", title));
                }
            }
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        info!("Initializing ZeroTerm");

        let config = Config::load(None).unwrap_or_default();

        let window_attrs = WindowAttributes::default()
            .with_title("ZeroTerm v0.2.0")
            .with_inner_size(winit::dpi::LogicalSize::new(
                config.window.width,
                config.window.height,
            ))
            .with_resizable(true);

        let window = Arc::new(event_loop.create_window(window_attrs)?);

        let font_size = config.font.size;
        self.font_size = font_size;
        self.opacity = config.window.opacity;
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            font_size,
            self.opacity,
            config.font.path.clone(),
        ))?;
        self.font_path = config.font.path.clone();

        let size = window.inner_size();
        let cell_w = font_size * 0.6;
        let cell_h = font_size * config.font.line_height;
        let cols = (size.width as f32 / cell_w) as usize;
        let rows = (size.height as f32 / cell_h) as usize;

        let shell = config.shell.program.clone();
        let shell_args = config.shell.args.clone();
        self.shell = shell.clone();
        self.shell_args = shell_args.clone();

        let (pty_rx, pty_tx) = spawn_pty_process(&shell, &shell_args, cols, rows)?;
        let _ = pty_tx.send(PtyCommand::Write(b"\x1b[?2004h".to_vec()));

        let parser = Parser::new(cols, rows);
        let mut panes = HashMap::new();
        panes.insert(
            0,
            PaneState {
                parser,
                pty_rx,
                pty_tx,
                title: "ZeroTerm v0.2.0".into(),
                pane_cmd: shell.clone(),
            },
        );

        let ai_client = if config.ai.endpoint.is_empty() {
            None
        } else {
            Some(Arc::new(AiClient::new(&config.ai.endpoint)))
        };

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.panes = panes;
        self.active_pane = 0;
        self.next_pane_id = 1;

        let session_path = session::session_file_path();
        if let Some(records) = session::load_session(&session_path) {
            if records.len() > 1 {
                for record in records.iter().skip(1) {
                    let cmd = if record.cmd.is_empty() {
                        shell.clone()
                    } else {
                        record.cmd.clone()
                    };
                    match spawn_pty_process(&cmd, &[], cols, rows) {
                        Ok((pty_rx, pty_tx)) => {
                            let _ = pty_tx.send(PtyCommand::Write(b"\x1b[?2004h".to_vec()));
                            let id = self.next_pane_id;
                            self.next_pane_id += 1;
                            self.panes.insert(
                                id,
                                PaneState {
                                    parser: Parser::new(cols, rows),
                                    pty_rx,
                                    pty_tx,
                                    title: record.title.clone(),
                                    pane_cmd: cmd,
                                },
                            );
                            self.tabs.push(Tab::new(id));
                        }
                        Err(e) => warn!("Session restore: failed to spawn '{}': {}", cmd, e),
                    }
                }
            }
        }

        self.ai_client = ai_client;
        self.sync_daemon = if config.sync.server_url.is_empty() {
            None
        } else {
            Some(SyncDaemon::new(config.sync.server_url.clone()))
        };

        self.config = Some(config);

        let ctx = self.settings_ctx();
        self.settings.refresh(&ctx);

        // Start config file watcher
        let config_path = Config::default_config_path();
        let config_dir = config_path.parent().unwrap().to_path_buf();
        let changed = self.config_changed.clone();

        std::thread::spawn(move || {
            use notify::{EventKind, RecursiveMode, Watcher};
            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher = match notify::recommended_watcher(
                move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            let _ = tx.send(());
                        }
                    }
                },
            ) {
                Ok(w) => w,
                Err(e) => {
                    error!("Failed to start config watcher: {}", e);
                    return;
                }
            };
            let _ = watcher.watch(&config_dir, RecursiveMode::NonRecursive);
            while rx.recv().is_ok() {
                changed.store(true, Ordering::SeqCst);
            }
        });

        info!("ZeroTerm initialized: {}x{} ({})", cols, rows, shell);
        Ok(())
    }

    fn create_new_tab(&mut self) -> Result<()> {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let cell_size = self
                .renderer
                .as_ref()
                .map(|r| r.cell_size())
                .unwrap_or([self.font_size * 0.6, self.font_size * 1.2]);
            let cell_w = cell_size[0];
            let cell_h = cell_size[1];
            let cols = (size.width as f32 / cell_w) as usize;
            let rows = (size.height as f32 / cell_h) as usize;

            let (pty_rx, pty_tx) = spawn_pty_process(&self.shell, &self.shell_args, cols, rows)?;
            let parser = Parser::new(cols, rows);
            let id = self.next_pane_id;
            self.next_pane_id += 1;
            self.panes.insert(
                id,
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: "ZeroTerm v0.2.0".into(),
                    pane_cmd: self.shell.clone(),
                },
            );
            self.active_pane = id;
            self.scroll_offset = 0;
            self.split_root = SplitNode::Leaf(id);
            self.tabs.push(Tab::new(id));
        }
        Ok(())
    }

    fn create_split_pane(&mut self, dir: SplitDir) -> Result<()> {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let cell_size = self
                .renderer
                .as_ref()
                .map(|r| r.cell_size())
                .unwrap_or([self.font_size * 0.6, self.font_size * 1.2]);
            let cell_w = cell_size[0];
            let cell_h = cell_size[1];
            let cols = (size.width as f32 / cell_w) as usize;
            let rows = (size.height as f32 / cell_h) as usize;

            let (pty_rx, pty_tx) = spawn_pty_process(&self.shell, &self.shell_args, cols, rows)?;
            let parser = Parser::new(cols, rows);
            let id = self.next_pane_id;
            self.next_pane_id += 1;
            self.panes.insert(
                id,
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: "ZeroTerm v0.2.0".into(),
                    pane_cmd: self.shell.clone(),
                },
            );
            let parent = self.active_pane;
            self.split_root.insert_leaf(id, dir, parent, 0.5);
            self.active_pane = id;
            self.scroll_offset = 0;
        }
        Ok(())
    }

    fn close_active_tab(&mut self) {
        if self.panes.len() <= 1 {
            return;
        }
        let closed = self.active_pane;
        if let Some(pane) = self.panes.remove(&closed) {
            let _ = pane.pty_tx.send(PtyCommand::Kill);
        }
        self.split_root.remove_leaf(closed);
        self.tabs.retain(|t| t.id != closed);
        let first = *self.panes.keys().next().unwrap_or(&0);
        self.active_pane = first;
        self.scroll_offset = 0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn next_tab(&mut self) {
        let mut keys: Vec<&usize> = self.panes.keys().collect();
        keys.sort();
        if keys.len() <= 1 {
            return;
        }
        let pos = keys
            .iter()
            .position(|k| **k == self.active_pane)
            .unwrap_or(0);
        let next = (pos + 1) % keys.len();
        self.active_pane = *keys[next];
        self.scroll_offset = 0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn previous_tab(&mut self) {
        let mut keys: Vec<&usize> = self.panes.keys().collect();
        keys.sort();
        if keys.len() <= 1 {
            return;
        }
        let pos = keys
            .iter()
            .position(|k| **k == self.active_pane)
            .unwrap_or(0);
        let prev = if pos == 0 { keys.len() - 1 } else { pos - 1 };
        self.active_pane = *keys[prev];
        self.scroll_offset = 0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn switch_to_tab(&mut self, idx: usize) {
        let mut keys: Vec<&usize> = self.panes.keys().collect();
        keys.sort();
        if idx < keys.len() {
            self.active_pane = *keys[idx];
            self.scroll_offset = 0;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn compute_split_rects(&self) -> HashMap<usize, (f32, f32, f32, f32)> {
        self.split_root.compute_rects()
    }

    fn focus_adjacent_pane(&mut self, dir: KeyCode) {
        let rects = self.split_root.compute_rects();
        if rects.len() <= 1 {
            return;
        }
        let cur = self.active_pane;
        let cur_rect = match rects.get(&cur) {
            Some(r) => *r,
            None => return,
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
        }
    }

    fn connect_ssh(&mut self, host: &str, user: &str, port: u16) -> Result<()> {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let cell_size = self
                .renderer
                .as_ref()
                .map(|r| r.cell_size())
                .unwrap_or([self.font_size * 0.6, self.font_size * 1.2]);
            let cell_w = cell_size[0];
            let cell_h = cell_size[1];
            let cols = (size.width as f32 / cell_w) as usize;
            let rows = (size.height as f32 / cell_h) as usize;

            let key_path = self
                .config
                .as_ref()
                .and_then(|c| {
                    if c.ssh.key_path.is_empty() {
                        None
                    } else {
                        Some(c.ssh.key_path.as_str())
                    }
                })
                .map(Path::new);

            let (pty_rx, pty_tx) = spawn_ssh_process(host, port, user, None, key_path, cols, rows)?;
            let parser = Parser::new(cols, rows);
            let id = self.next_pane_id;
            self.next_pane_id += 1;
            self.panes.insert(
                id,
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: format!("SSH: {}@{}", user, host),
                    pane_cmd: format!("ssh {}@{}", user, host),
                },
            );
            self.active_pane = id;
            self.scroll_offset = 0;
            self.split_root = SplitNode::Leaf(id);
            self.tabs.push(Tab::new(id));
        }
        Ok(())
    }

    fn ai_explain(&self) {
        if let Some(ai_client) = &self.ai_client {
            if let Some(pane) = self.panes.get(&self.active_pane) {
                let screen = pane.parser.screen();
                let mut text = String::new();
                for row in screen.buffer() {
                    for cell in row {
                        text.push(cell.ch);
                    }
                    text.push('\n');
                }
                let client = ai_client.clone();
                let tx = pane.pty_tx.clone();
                std::thread::spawn(move || {
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = tx.send(PtyCommand::Write(
                                format!("\r\n\u{1b}[31mRuntime error: {}\u{1b}[0m\r\n", e)
                                    .into_bytes(),
                            ));
                            return;
                        }
                    };
                    match rt.block_on(client.explain(&text)) {
                        Ok(response) => {
                            let _ = tx.send(PtyCommand::Write(response.into_bytes()));
                        }
                        Err(e) => {
                            let _ = tx.send(PtyCommand::Write(
                                format!("\r\n\u{1b}[31mAI error: {}\u{1b}[0m\r\n", e).into_bytes(),
                            ));
                        }
                    }
                });
            }
        }
    }

    fn drain_pty(&mut self) -> bool {
        let mut got_data = false;
        let active = self.active_pane;
        let mut title_changed = None;
        let mut dead_panes = Vec::new();
        let pane_count = self.panes.len();
        for (&id, pane) in &mut self.panes {
            loop {
                match pane.pty_rx.try_recv() {
                    Ok(data) => {
                        pane.parser.parse(&data);
                        if let Some(text) = pane.parser.take_clipboard_text() {
                            if let Some(clipboard) = &mut self.clipboard {
                                if let Err(e) = clipboard.set_text(text) {
                                    warn!("clipboard error: {}", e);
                                }
                            }
                        }
                        if id == active {
                            let new_title = pane.parser.screen().title().to_string();
                            if new_title != pane.title {
                                pane.title = new_title.clone();
                                title_changed = Some(new_title);
                            }
                        }
                        got_data = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if pane_count <= 1 {
                            pane.parser
                                .parse(b"\r\n[Process exited] - exit to quit\r\n");
                        } else {
                            pane.parser.parse(b"\r\n[Process exited]\r\n");
                        }
                        dead_panes.push(id);
                        break;
                    }
                }
            }
        }
        for id in &dead_panes {
            warn!("Pane {} process exited", id);
            if self.panes.len() > 1 {
                self.panes.remove(id);
                self.split_root.remove_leaf(*id);
                if self.active_pane == *id {
                    self.active_pane = *self.panes.keys().next().unwrap_or(&0);
                }
            }
        }
        if let Some(title) = title_changed {
            if let Some(window) = &self.window {
                window.set_title(&format!("ZeroTerm v0.2.0 - {}", title));
            }
        }
        got_data
    }

    fn periodic_sync(&mut self) {
        self.sync_tick += 1;
        if self.sync_tick >= 300 {
            self.sync_tick = 0;
            if let Some(sync) = &self.sync_daemon {
                sync.mark_dirty();
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        if self.config_changed.load(Ordering::SeqCst) {
            self.config_changed.store(false, Ordering::SeqCst);
            if let Some(config) = &mut self.config {
                config.reload(None).ok();
            }
            self.apply_config_to_renderer();
        }
        if self.settings.open {
            self.draw_settings_overlay();
        }
        if let Some(renderer) = &mut self.renderer {
            if let Some(pane) = self.panes.get(&self.active_pane) {
                renderer.render(pane.parser.screen(), self.scroll_offset, self.selection)?;
            }
        }
        Ok(())
    }

    fn write_pty(&self, data: &[u8]) {
        if let Some(pane) = self.panes.get(&self.active_pane) {
            let _ = pane.pty_tx.send(PtyCommand::Write(data.to_vec()));
        }
    }

    fn resize_pty(&self, cols: usize, rows: usize) {
        if let Some(pane) = self.panes.get(&self.active_pane) {
            let _ = pane.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
        }
    }

    fn max_scroll_offset(&self) -> usize {
        if let Some(pane) = self.active_pane() {
            let screen = pane.parser.screen();
            let total_rows = screen.scrollback().len() + screen.buffer().len();
            let visible_rows = screen.size().rows;
            total_rows.saturating_sub(visible_rows)
        } else {
            0
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        let max = self.max_scroll_offset();
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(max);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    // Selection methods
    fn screen_to_cell(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if let (Some(renderer), Some(pane)) = (&self.renderer, self.active_pane()) {
            let cell_size = renderer.cell_size();
            let cell_w = cell_size[0];
            let cell_h = cell_size[1];
            let screen = pane.parser.screen();
            let buffer = screen.buffer();
            let visible_rows = buffer.len();
            let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

            let col = (x / cell_w).floor() as usize;
            let row = (y / cell_h).floor() as usize;

            if row < visible_rows && col < cols {
                let scrollback = screen.scrollback().len();
                let total_rows = scrollback + visible_rows;
                let end = total_rows.saturating_sub(self.scroll_offset);
                let start = end.saturating_sub(visible_rows);
                let global_row = start + row;
                Some((global_row, col))
            } else {
                None
            }
        } else {
            None
        }
    }

    fn start_selection(&mut self, x: f32, y: f32) {
        if let Some((row, col)) = self.screen_to_cell(x, y) {
            self.selection = Some(Selection {
                start_row: row,
                start_col: col,
                end_row: row,
                end_col: col,
                active: true,
            });
            self.selecting = true;
        }
    }

    fn update_selection(&mut self, x: f32, y: f32) {
        if self.selecting {
            if let Some((row, col)) = self.screen_to_cell(x, y) {
                if let Some(sel) = &mut self.selection {
                    sel.end_row = row;
                    sel.end_col = col;
                }
            }
        }
    }

    fn end_selection(&mut self) {
        self.selecting = false;
    }

    fn copy_selection(&mut self) {
        let sel = self.selection.clone();
        let text = sel.as_ref().and_then(|sel| {
            self.active_pane().map(|pane| {
                let screen = pane.parser.screen();
                let scrollback = screen.scrollback();
                let buffer = screen.buffer();
                let visible_rows = buffer.len();
                let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

                let (start_row, start_col, end_row, end_col) = if sel.start_row < sel.end_row
                    || (sel.start_row == sel.end_row && sel.start_col <= sel.end_col)
                {
                    (sel.start_row, sel.start_col, sel.end_row, sel.end_col)
                } else {
                    (sel.end_row, sel.end_col, sel.start_row, sel.start_col)
                };

                let mut text = String::new();
                let total_scrollback = scrollback.len();
                let total_rows = total_scrollback + visible_rows;

                for r in start_row..=end_row.min(total_rows - 1) {
                    let line = if r < total_scrollback {
                        &scrollback[total_scrollback - 1 - r]
                    } else {
                        &buffer[r - total_scrollback]
                    };

                    let line_start = if r == start_row { start_col } else { 0 };
                    let line_end = if r == end_row { end_col + 1 } else { cols };

                    for c in line_start..line_end.min(line.len()) {
                        text.push(line[c].ch);
                    }
                    if r < end_row {
                        text.push('\n');
                    }
                }
                text
            })
        });
        if let Some(text) = text {
            if let Some(clipboard) = &mut self.clipboard {
                let _ = clipboard.set_text(text.trim_end());
            }
        }
    }

    fn clear_selection(&mut self) {
        self.selection = None;
    }

    fn copy_block_output(&mut self) -> bool {
        if self.scroll_offset != 0 {
            return false;
        }
        let Some((global_row, col)) = self.screen_to_cell(self.mouse_pos.0, self.mouse_pos.1)
        else {
            return false;
        };
        let Some(pane) = self.active_pane() else {
            return false;
        };
        let screen = pane.parser.screen();
        let scrollback = screen.scrollback().len();
        if global_row < scrollback {
            return false;
        }
        let row = global_row - scrollback;
        let cols = screen.size().cols;
        if col + COPY_MARKER.len() < cols {
            return false;
        }
        let Some(block) = screen.blocks().iter().find(|b| b.start_line == row) else {
            return false;
        };
        let text = block_output_text(screen, block);
        if let Some(clipboard) = &mut self.clipboard {
            let _ = clipboard.set_text(&text);
        }
        true
    }

    fn cycle_opacity(&mut self) {
        const STEPS: [f64; 3] = [1.0, 0.85, 0.7];
        let idx = STEPS
            .iter()
            .position(|o| (*o - self.opacity).abs() < 0.01)
            .unwrap_or(0);
        self.opacity = STEPS[(idx + 1) % STEPS.len()];
        if let Some(renderer) = &mut self.renderer {
            renderer.set_opacity(self.opacity);
        }
    }

    fn settings_ctx(&self) -> SettingsContext {
        let config = self.config.as_ref();
        SettingsContext {
            font_size: config.map_or(14.0, |c| c.font.size),
            opacity: self.opacity,
            theme: config
                .map(|c| SettingsMenu::theme_name(&c.colors.background))
                .unwrap_or_else(|| "tokyo-night".to_string()),
        }
    }

    fn apply_config_to_renderer(&mut self) {
        if let Some(config) = &self.config {
            self.opacity = config.window.opacity;
            self.font_path = config.font.path.clone();
            if let Some(renderer) = &mut self.renderer {
                renderer.reload_config(config);
            }
        }
    }

    fn toggle_settings(&mut self) {
        self.settings.toggle();
        if self.settings.open {
            let ctx = self.settings_ctx();
            self.settings.refresh(&ctx);
            if let Some(pane) = self.panes.get_mut(&self.active_pane) {
                self.settings.save_screen(pane.parser.screen());
            }
            self.draw_settings_overlay();
        } else {
            self.close_settings();
        }
    }

    fn close_settings(&mut self) {
        if let Some(pane) = self.panes.get_mut(&self.active_pane) {
            self.settings.restore_screen(pane.parser.screen_mut());
        }
        self.settings.open = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn draw_settings_overlay(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.active_pane) else {
            return;
        };
        let (cols, rows) = {
            let s = pane.parser.screen();
            (s.size().cols, s.size().rows)
        };
        let bytes = self.settings.overlay_bytes(cols, rows);
        pane.parser.parse(&bytes);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn apply_settings_action(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::Close => {
                self.close_settings();
                return;
            }
            SettingsAction::FontSizeDelta(delta) => {
                if let Some(config) = &mut self.config {
                    config.font.size = (config.font.size + delta as f32).max(6.0);
                    let _ = config.save(None);
                }
            }
            SettingsAction::OpacityDelta(delta) => {
                self.opacity = (self.opacity + delta as f64).clamp(0.5, 1.0);
                if let Some(config) = &mut self.config {
                    config.window.opacity = self.opacity;
                }
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_opacity(self.opacity);
                }
            }
            SettingsAction::ToggleTheme | SettingsAction::CycleTheme => {
                let bg = self
                    .config
                    .as_ref()
                    .map_or("#1a1b26", |c| c.colors.background.as_str());
                let (fg, bg) = self.settings.next_theme(bg);
                if let Some(config) = &mut self.config {
                    config.colors.foreground = fg.to_string();
                    config.colors.background = bg.to_string();
                    let _ = config.save(None);
                }
            }
            SettingsAction::ReloadConfig => {
                if let Some(config) = &mut self.config {
                    config.reload(None).ok();
                }
            }
        }
        self.apply_config_to_renderer();
        self.draw_settings_overlay();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            if let Err(e) = self.init(event_loop) {
                error!("Failed to initialize: {}", e);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("Close requested");
                if let Err(e) = session::save_session(&session::session_file_path(), &self.panes) {
                    error!("Failed to save session: {}", e);
                }
                for (_, pane) in &self.panes {
                    let _ = pane.pty_tx.send(PtyCommand::Kill);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let cell_size = self
                    .renderer
                    .as_ref()
                    .map(|r| r.cell_size())
                    .unwrap_or([self.font_size * 0.6, self.font_size * 1.2]);
                let cell_w = cell_size[0];
                let cell_h = cell_size[1];
                let cols = (size.width as f32 / cell_w) as usize;
                let rows = (size.height as f32 / cell_h) as usize;

                self.resize_pty(cols, rows);
                if let Some(pane) = self.panes.get_mut(&self.active_pane) {
                    pane.parser.screen_mut().resize(cols, rows);
                }
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(state) => {
                self.modifiers = state.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != winit::event::ElementState::Pressed {
                    return;
                }

                let ctrl = self.modifiers.control_key();
                let shift = self.modifiers.shift_key();
                let alt = self.modifiers.alt_key();

                // Tab management shortcuts
                match &event.physical_key {
                    PhysicalKey::Code(code) => {
                        if ctrl && shift && !alt && *code == KeyCode::KeyP {
                            self.toggle_settings();
                            return;
                        }
                        if self.settings.open {
                            match code {
                                KeyCode::ArrowUp => self.settings.prev(),
                                KeyCode::ArrowDown => self.settings.next(),
                                KeyCode::Enter => {
                                    let ctx = self.settings_ctx();
                                    let action = self.settings.activate(&ctx);
                                    self.apply_settings_action(action);
                                }
                                KeyCode::Escape => self.close_settings(),
                                _ => {}
                            }
                            if self.settings.open {
                                self.draw_settings_overlay();
                            }
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyT {
                            if let Err(e) = self.create_new_tab() {
                                error!("Failed to create tab: {}", e);
                            }
                            self.update_window_title();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyW {
                            self.close_active_tab();
                            self.update_window_title();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyE {
                            if let Err(e) = self.create_split_pane(SplitDir::Vertical) {
                                error!("Failed to split pane: {}", e);
                            }
                            self.update_window_title();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyD {
                            if let Err(e) = self.create_split_pane(SplitDir::Horizontal) {
                                error!("Failed to split pane: {}", e);
                            }
                            self.update_window_title();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyI {
                            self.ai_explain();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyO {
                            self.cycle_opacity();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyS {
                            if let Some(config) = &self.config {
                                if !config.ssh.host.is_empty() {
                                    let host = config.ssh.host.clone();
                                    let user = config.ssh.user.clone();
                                    let port = config.ssh.port;
                                    if let Err(e) = self.connect_ssh(&host, &user, port) {
                                        error!("SSH connect failed: {}", e);
                                    }
                                    self.update_window_title();
                                }
                            }
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::Tab {
                            self.previous_tab();
                            self.update_window_title();
                            return;
                        }
                        if ctrl && !shift && !alt && *code == KeyCode::Tab {
                            self.next_tab();
                            self.update_window_title();
                            return;
                        }
                        if alt && !ctrl && !shift {
                            match code {
                                KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::ArrowUp
                                | KeyCode::ArrowDown => {
                                    self.focus_adjacent_pane(*code);
                                    self.update_window_title();
                                    return;
                                }
                                _ => {}
                            }
                            let idx = match code {
                                KeyCode::Digit1 => Some(0),
                                KeyCode::Digit2 => Some(1),
                                KeyCode::Digit3 => Some(2),
                                KeyCode::Digit4 => Some(3),
                                KeyCode::Digit5 => Some(4),
                                KeyCode::Digit6 => Some(5),
                                KeyCode::Digit7 => Some(6),
                                KeyCode::Digit8 => Some(7),
                                KeyCode::Digit9 => Some(8),
                                _ => None,
                            };
                            if let Some(idx) = idx {
                                self.switch_to_tab(idx);
                                self.update_window_title();
                                return;
                            }
                        }
                    }
                    _ => {}
                }

                match &event.physical_key {
                    PhysicalKey::Code(code) => {
                        // Handle scrollback navigation with Shift modifier
                        let shift = self.modifiers.shift_key();
                        if shift && !ctrl && !alt {
                            match code {
                                KeyCode::PageUp => {
                                    self.scroll_up(20);
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::PageDown => {
                                    self.scroll_down(20);
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::Home => {
                                    self.scroll_offset = self.max_scroll_offset();
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::End => {
                                    self.scroll_offset = 0;
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                _ => {}
                            }
                        }

                        let seq: Vec<u8> = match code {
                            KeyCode::Enter => vec![b'\r'],
                            KeyCode::Backspace => vec![0x7f],
                            KeyCode::Tab => vec![b'\t'],
                            KeyCode::Escape => vec![0x1b],
                            KeyCode::ArrowUp => vec![0x1b, b'[', b'A'],
                            KeyCode::ArrowDown => vec![0x1b, b'[', b'B'],
                            KeyCode::ArrowRight => vec![0x1b, b'[', b'C'],
                            KeyCode::ArrowLeft => vec![0x1b, b'[', b'D'],
                            KeyCode::Home => vec![0x1b, b'[', b'H'],
                            KeyCode::End => vec![0x1b, b'[', b'F'],
                            KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
                            KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
                            KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
                            KeyCode::F1 => vec![0x1b, b'[', b'1', b'1', b'~'],
                            KeyCode::F2 => vec![0x1b, b'[', b'1', b'2', b'~'],
                            KeyCode::F3 => vec![0x1b, b'[', b'1', b'3', b'~'],
                            KeyCode::F4 => vec![0x1b, b'[', b'1', b'4', b'~'],
                            KeyCode::F5 => vec![0x1b, b'[', b'1', b'5', b'~'],
                            KeyCode::F6 => vec![0x1b, b'[', b'1', b'7', b'~'],
                            KeyCode::F7 => vec![0x1b, b'[', b'1', b'8', b'~'],
                            KeyCode::F8 => vec![0x1b, b'[', b'1', b'9', b'~'],
                            KeyCode::F9 => vec![0x1b, b'[', b'2', b'0', b'~'],
                            KeyCode::F10 => vec![0x1b, b'[', b'2', b'1', b'~'],
                            KeyCode::F11 => vec![0x1b, b'[', b'2', b'3', b'~'],
                            KeyCode::F12 => vec![0x1b, b'[', b'2', b'4', b'~'],
                            _ if ctrl && !alt => match code {
                                KeyCode::KeyA => vec![0x01],
                                KeyCode::KeyB => vec![0x02],
                                KeyCode::KeyC => vec![0x03],
                                KeyCode::KeyD => vec![0x04],
                                KeyCode::KeyE => vec![0x05],
                                KeyCode::KeyF => vec![0x06],
                                KeyCode::KeyG => vec![0x07],
                                KeyCode::KeyH => vec![0x08],
                                KeyCode::KeyI => vec![0x09],
                                KeyCode::KeyJ => vec![0x0a],
                                KeyCode::KeyK => vec![0x0b],
                                KeyCode::KeyL => vec![0x0c],
                                KeyCode::KeyM => vec![0x0d],
                                KeyCode::KeyN => vec![0x0e],
                                KeyCode::KeyO => vec![0x0f],
                                KeyCode::KeyP => vec![0x10],
                                KeyCode::KeyQ => vec![0x11],
                                KeyCode::KeyR => vec![0x12],
                                KeyCode::KeyS => vec![0x13],
                                KeyCode::KeyT => vec![0x14],
                                KeyCode::KeyU => vec![0x15],
                                KeyCode::KeyV => vec![0x16],
                                KeyCode::KeyW => vec![0x17],
                                KeyCode::KeyX => vec![0x18],
                                KeyCode::KeyY => vec![0x19],
                                KeyCode::KeyZ => vec![0x1a],
                                KeyCode::Space => vec![0x00],
                                _ => vec![],
                            },
                            // Ctrl+Shift+C: Copy selection
                            _ if ctrl && shift && !alt => match code {
                                KeyCode::KeyC => {
                                    self.copy_selection();
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    vec![]
                                }
                                // Ctrl+Shift+V: Paste from clipboard
                                KeyCode::KeyV => {
                                    if let Some(clipboard) = &mut self.clipboard {
                                        if let Ok(text) = clipboard.get_text() {
                                            let bracketed = self
                                                .active_pane()
                                                .map_or(false, |p| p.parser.bracketed_paste());
                                            if bracketed {
                                                let mut data = b"\x1b[200~".to_vec();
                                                data.extend_from_slice(text.as_bytes());
                                                data.extend_from_slice(b"\x1b[201~");
                                                self.write_pty(&data);
                                            } else {
                                                self.write_pty(text.as_bytes());
                                            }
                                        }
                                    }
                                    vec![]
                                }
                                _ => vec![],
                            },
                            _ => vec![],
                        };
                        if !seq.is_empty() {
                            self.write_pty(&seq);
                        }
                    }
                    _ => {}
                }

                // Handle printable text (IME text input)
                if let Some(text) = &event.text {
                    if !text.is_empty() && !ctrl && !alt {
                        self.write_pty(text.as_bytes());
                    }
                }

                if self.drain_pty() {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.periodic_sync();
                self.drain_pty();

                if let Err(e) = self.render() {
                    error!("Render error: {}", e);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                let mouse_tracking = self
                    .active_pane()
                    .map_or(MouseTrackingMode::Off, |p| p.parser.mouse_tracking());
                if let Some(window) = &self.window {
                    if mouse_tracking != MouseTrackingMode::Off {
                        window.set_cursor(CursorIcon::Crosshair);
                    } else {
                        window.set_cursor(CursorIcon::Text);
                    }
                }
                if mouse_tracking == MouseTrackingMode::AnyEvent && !self.selecting {
                    if let Some((row, col)) =
                        self.screen_to_cell(position.x as f32, position.y as f32)
                    {
                        let mods = (if self.modifiers.shift_key() { 4 } else { 0 })
                            | (if self.modifiers.control_key() { 8 } else { 0 })
                            | (if self.modifiers.alt_key() { 16 } else { 0 });
                        self.write_pty(
                            format!("\x1b[<{};{};{}M", col + 1, row + 1, 35 + mods).as_bytes(),
                        );
                    }
                }
                if self.selecting {
                    self.update_selection(position.x as f32, position.y as f32);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let mouse_tracking = self
                    .active_pane()
                    .map_or(MouseTrackingMode::Off, |p| p.parser.mouse_tracking());
                if mouse_tracking != MouseTrackingMode::Off {
                    let button_id = match button {
                        MouseButton::Left => 0,
                        MouseButton::Middle => 1,
                        MouseButton::Right => 2,
                        _ => 0,
                    };
                    if let Some((row, col)) =
                        self.screen_to_cell(self.mouse_pos.0, self.mouse_pos.1)
                    {
                        let mods = (if self.modifiers.shift_key() { 4 } else { 0 })
                            | (if self.modifiers.control_key() { 8 } else { 0 })
                            | (if self.modifiers.alt_key() { 16 } else { 0 });
                        let cb = 32 + button_id + mods;
                        let final_byte = if state == winit::event::ElementState::Pressed {
                            'M'
                        } else {
                            'm'
                        };
                        self.write_pty(
                            format!("\x1b[<{};{};{}{}", col + 1, row + 1, cb, final_byte)
                                .as_bytes(),
                        );
                    }
                } else if button == MouseButton::Left {
                    if state == winit::event::ElementState::Pressed {
                        if !self.copy_block_output() {
                            self.start_selection(self.mouse_pos.0, self.mouse_pos.1);
                        }
                    } else {
                        self.end_selection();
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                match delta {
                    MouseScrollDelta::LineDelta(_, y) => {
                        if y > 0.0 {
                            self.scroll_up(y as usize);
                        } else {
                            self.scroll_down((-y) as usize);
                        }
                    }
                    MouseScrollDelta::PixelDelta(pos) => {
                        let cell_h = self
                            .renderer
                            .as_ref()
                            .map(|r| r.cell_size()[1])
                            .unwrap_or(20.0);
                        let lines = (pos.y as f32 / cell_h).round() as usize;
                        if pos.y > 0.0 {
                            self.scroll_up(lines.max(1));
                        } else {
                            self.scroll_down(lines.max(1));
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting ZeroTerm v0.1.0");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new();
    event_loop.run_app(&mut app)?;

    Ok(())
}
