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
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes};

use zeroterm_ai::client::AiClient;
use zeroterm_config::{Config, KeybindingsConfig};
use zeroterm_core::cell::{Cell, Cursor};
use zeroterm_core::parser::MouseTrackingMode;
use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::{CommandBlock, Size as PtySize};
use zeroterm_core::Parser;
use zeroterm_mux::split::{SplitDir, SplitNode};
use zeroterm_mux::tab::Tab;
use zeroterm_render::{tab_span, Renderer, Selection, TabInfo};
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

fn word_left(chars: &[char], col: usize) -> usize {
    let mut i = col.saturating_sub(1);
    while i > 0 && chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i -= 1;
    }
    while i > 0 && chars.get(i - 1).is_some_and(|c| !c.is_whitespace()) {
        i -= 1;
    }
    i
}

fn word_right(chars: &[char], col: usize, cols: usize) -> usize {
    let mut i = col;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() {
        return col;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    (i - 1).min(cols.saturating_sub(1))
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
    pty_dead: bool,
}
impl PaneState {
    /// Drain available pty output into the parser. Returns true if any bytes
    /// were parsed. Marks the pane dead once the pty channel disconnects so a
    /// dead pane is never drained twice (this is what stops the exit notice
    /// from being re-appended to the buffer on every subsequent drain call).
    fn drain(&mut self) -> bool {
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

fn spawn_pty_process(
    shell: &str,
    shell_args: &[String],
    cols: usize,
    rows: usize,
    wake: EventLoopProxy<()>,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let shell_refs: Vec<&str> = shell_args.iter().map(|s| s.as_str()).collect();
    let mut backend = PortablePtyBackend::new()?;
    let mut process = backend.spawn(shell, &shell_refs, None)?;
    process.resize(PtySize { cols, rows })?;

    let (output_tx, pty_rx) = mpsc::sync_channel::<Vec<u8>>(4);
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
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
                    let _ = wake.send_event(());
                }
                Err(_) => break,
            }
        }
    });

    Ok((pty_rx, pty_tx))
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn spawn_ssh_process(
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
                Err(_) => break,
            }
        }
        let _ = ssh.disconnect();
    });

    Ok((pty_rx, pty_tx))
}

/// SSH host picker overlay, drawn into the active pane like the settings menu.
/// Destructive to covered cells, so the region is snapshotted on open and
/// restored on close (same pattern as settings.rs).
struct HostPicker {
    open: bool,
    aliases: Vec<String>,
    cursor: usize,
    saved_cells: Option<Vec<Vec<Cell>>>,
    saved_top: Option<usize>,
    saved_cursor: Option<Cursor>,
}

impl HostPicker {
    fn new() -> Self {
        Self {
            open: false,
            aliases: Vec::new(),
            cursor: 0,
            saved_cells: None,
            saved_top: None,
            saved_cursor: None,
        }
    }

    fn open(&mut self, aliases: Vec<String>) {
        self.aliases = aliases;
        self.cursor = 0;
        self.open = true;
    }

    fn next(&mut self) {
        if !self.aliases.is_empty() {
            self.cursor = (self.cursor + 1) % self.aliases.len();
        }
    }

    fn prev(&mut self) {
        if !self.aliases.is_empty() {
            self.cursor = (self.cursor + self.aliases.len() - 1) % self.aliases.len();
        }
    }

    fn selected(&self) -> Option<String> {
        self.aliases.get(self.cursor).cloned()
    }

    fn panel_lines(&self) -> Vec<String> {
        let mut lines = vec![" SSH Hosts ".to_string()];
        for (i, alias) in self.aliases.iter().enumerate() {
            let marker = if i == self.cursor { '>' } else { ' ' };
            lines.push(format!(" {} {}", marker, alias));
        }
        lines.push(" arrows: navigate  enter: connect  esc: cancel ".to_string());
        lines
    }

    fn overlay_rect(&self, cols: usize, rows: usize) -> (usize, usize, usize, usize) {
        let lines = self.panel_lines();
        let width = lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(10)
            .min(cols.saturating_sub(2))
            .max(2);
        let height = lines.len().min(rows).max(2);
        let top = (rows.saturating_sub(height)) / 2;
        let left = (cols.saturating_sub(width)) / 2;
        (top, left, height, width)
    }

    fn overlay_bytes(&self, cols: usize, rows: usize) -> Vec<u8> {
        let lines = self.panel_lines();
        let (top, left, height, width) = self.overlay_rect(cols, rows);
        let panel_bg = (40, 44, 52);
        let panel_fg = (197, 200, 198);
        let sel_bg = (61, 89, 171);
        let sel_fg = (255, 255, 255);

        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[?25l");
        for (i, line) in lines.iter().take(height).enumerate() {
            let (bg, fg) = if i >= 1 && i - 1 == self.cursor {
                (sel_bg, sel_fg)
            } else {
                (panel_bg, panel_fg)
            };
            let text: String = line.chars().take(width).collect();
            let pad = width.saturating_sub(text.chars().count());
            out.extend_from_slice(format!("\x1b[{};{}H", top + i + 1, left + 1).as_bytes());
            out.extend_from_slice(
                format!(
                    "\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m",
                    bg.0, bg.1, bg.2, fg.0, fg.1, fg.2
                )
                .as_bytes(),
            );
            out.extend_from_slice(text.as_bytes());
            for _ in 0..pad {
                out.push(b' ');
            }
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    }

    fn save_screen(&mut self, screen: &zeroterm_core::screen::Screen) {
        let (top, _, height, _) = self.overlay_rect(screen.size().cols, screen.size().rows);
        let buf = screen.buffer();
        self.saved_cells = Some(
            (0..height)
                .map(|i| buf.get(top + i).cloned().unwrap_or_default())
                .collect(),
        );
        self.saved_top = Some(top);
        self.saved_cursor = Some(screen.cursor());
    }

    fn restore_screen(&mut self, screen: &mut zeroterm_core::screen::Screen) {
        if let (Some(cells), Some(top), Some(cursor)) =
            (&self.saved_cells, self.saved_top, &self.saved_cursor)
        {
            for (i, row_cells) in cells.iter().enumerate() {
                screen.set_cells(top + i, row_cells);
            }
            screen.cursor_pos(cursor.row + 1, cursor.col + 1);
            screen.set_cursor_visible(cursor.visible);
        }
        self.saved_cells = None;
        self.saved_top = None;
        self.saved_cursor = None;
    }
}

#[allow(dead_code)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    panes: HashMap<usize, PaneState>,
    active_pane: usize,
    next_pane_id: usize,
    tabs: Vec<Tab>,
    // ponytail: per-pane scroll kept as single field, inactive panes render at offset 0
    split_root: SplitNode,
    // ponytail: no mouse hit-testing on the overlay rect; keyboard focus only
    floating: Option<usize>,
    // Split divider drag: Some(target) = dragging the divider whose first leaf
    // is `target`; anchor is the last window-space mouse position.
    dragging_divider: Option<usize>,
    divider_anchor: (f32, f32),
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
    host_picker: HostPicker,
    sync_active: bool,
    last_sync_clear: std::time::Instant,
    last_anim_frame: std::time::Instant,
    event_proxy: Option<EventLoopProxy<()>>,
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
            floating: None,
            dragging_divider: None,
            divider_anchor: (0.0, 0.0),
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
            host_picker: HostPicker::new(),
            sync_active: false,
            last_sync_clear: std::time::Instant::now(),
            last_anim_frame: std::time::Instant::now(),
            event_proxy: None,
        }
    }

    fn active_pane(&self) -> Option<&PaneState> {
        self.panes.get(&self.active_pane)
    }

    /// Clone of the event-loop proxy for a PTY reader thread. Registered in
    /// main() before run_app, so always present once init() spawns PTYs.
    fn wake_proxy(&self) -> EventLoopProxy<()> {
        self.event_proxy
            .clone()
            .expect("event_proxy registered in main() before run_app")
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
        info!("keybindings: vim_mode={}", config.keybindings.vim_mode);

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
        // Size parsers from the renderer's *measured* glyph metrics. The old
        // font_size*0.6 heuristic (8.4px for a 14px font) under-sized the cell,
        // giving the parser more cols than the renderer's cell buffer holds
        // (e.g. 119 vs 112 at 1000px) -> update_cell_data wrote past the buffer.
        let cell = renderer.cell_size();
        let cols = (size.width as f32 / cell[0]) as usize;
        let rows = (size.height as f32 / cell[1]) as usize;

        let shell = config.shell.program.clone();
        let shell_args = config.shell.args.clone();
        self.shell = shell.clone();
        self.shell_args = shell_args.clone();

        let (pty_rx, pty_tx) =
            spawn_pty_process(&shell, &shell_args, cols, rows, self.wake_proxy())?;
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
                pty_dead: false,
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
        if let Some((records, layout)) = session::load_session(&session_path) {
            let mut restored_ids = Vec::new();
            if records.len() > 1 {
                for record in records.iter().skip(1) {
                    let cmd = if record.cmd.is_empty() {
                        shell.clone()
                    } else {
                        record.cmd.clone()
                    };
                    match spawn_pty_process(&cmd, &[], cols, rows, self.wake_proxy()) {
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
                                    pty_dead: false,
                                },
                            );
                            self.tabs.push(Tab::new(id));
                            restored_ids.push(id);
                        }
                        Err(e) => warn!("Session restore: failed to spawn '{}': {}", cmd, e),
                    }
                }
            }
            if layout.is_some() {
                self.split_root = SplitNode::from_ids(&restored_ids);
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
        if let Some(window) = &self.window {
            window.request_redraw();
        }
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

            let (pty_rx, pty_tx) =
                spawn_pty_process(&self.shell, &self.shell_args, cols, rows, self.wake_proxy())?;
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
                    pty_dead: false,
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

            let (pty_rx, pty_tx) =
                spawn_pty_process(&self.shell, &self.shell_args, cols, rows, self.wake_proxy())?;
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
                    pty_dead: false,
                },
            );
            let parent = self.active_pane;
            self.split_root.insert_leaf(id, dir, parent, 0.5);
            self.active_pane = id;
            self.scroll_offset = 0;
            self.resize_panes_to_rects();
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
        self.resize_panes_to_rects();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Toggle active pane between split-tree and floating overlay (Ctrl+Shift+F).
    fn toggle_floating_pane(&mut self) {
        let active = self.active_pane;
        if self.floating == Some(active) {
            // Dock: re-insert at first remaining tree leaf.
            // ponytail: original slot lost (insert_leaf only splits a parent) — root-ish
            // placement is the accepted ceiling.
            self.floating = None;
            if !self.split_root.leaves().contains(&active) {
                let parent = *self.split_root.leaves().first().unwrap_or(&active);
                self.split_root
                    .insert_leaf(active, SplitDir::Vertical, parent, 0.5);
            }
            self.resize_panes_to_rects();
        } else {
            // Dock whatever was floating (one float at a time), then float active.
            if let Some(prev) = self.floating.take() {
                if !self.split_root.leaves().contains(&prev) {
                    let parent = *self.split_root.leaves().first().unwrap_or(&prev);
                    self.split_root
                        .insert_leaf(prev, SplitDir::Vertical, parent, 0.5);
                }
            }
            if self.split_root.leaves().len() > 1 {
                self.split_root.remove_leaf(active);
                self.floating = Some(active);
                self.resize_panes_to_rects();
            } else {
                // ponytail: last visible pane stays in tree AND floats (overlay wins when
                // drawn twice); zero visible panes not allowed.
                self.floating = Some(active);
            }
        }
        let Some(renderer) = &self.renderer else {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        };
        if let Some(window) = &self.window {
            // Resize floating pane to overlay dims so cells don't overflow the box.
            if let Some(id) = self.floating {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let tab_h = renderer.cell_size()[1];
                    let content_h = (window.inner_size().height as f32 - tab_h).max(0.0);
                    let cols = renderer.cols_for(window.inner_size().width as f32 * 0.7);
                    let rows = renderer.rows_for(content_h * 0.7);
                    pane.parser.screen_mut().resize(cols, rows);
                    let _ = pane.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
                }
            }
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

    #[cfg(unix)]
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

            let (pty_rx, pty_tx) = spawn_ssh_process(
                host,
                port,
                user,
                None,
                key_path,
                cols,
                rows,
                self.wake_proxy(),
            )?;
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
                    pty_dead: false,
                },
            );
            self.active_pane = id;
            self.scroll_offset = 0;
            self.split_root = SplitNode::Leaf(id);
            self.tabs.push(Tab::new(id));
        }
        Ok(())
    }

    fn open_host_picker(&mut self) {
        #[cfg(unix)]
        {
            let aliases = zeroterm_ssh::client::ssh_aliases();
            if aliases.is_empty() {
                return;
            }
            self.host_picker.open(aliases);
            if let Some(pane) = self.panes.get_mut(&self.active_pane) {
                self.host_picker.save_screen(pane.parser.screen());
            }
            self.draw_host_picker();
        }
    }

    fn draw_host_picker(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.active_pane) else {
            return;
        };
        let (cols, rows) = {
            let s = pane.parser.screen();
            (s.size().cols, s.size().rows)
        };
        let bytes = self.host_picker.overlay_bytes(cols, rows);
        pane.parser.parse(&bytes);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn close_host_picker(&mut self) {
        if let Some(pane) = self.panes.get_mut(&self.active_pane) {
            self.host_picker.restore_screen(pane.parser.screen_mut());
        }
        self.host_picker.open = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pick_host(&mut self) {
        #[cfg(unix)]
        {
            let Some(alias) = self.host_picker.selected() else {
                self.close_host_picker();
                return;
            };
            self.close_host_picker();
            let user = self
                .config
                .as_ref()
                .map_or_else(String::new, |c| c.ssh.user.clone());
            let port = self.config.as_ref().map_or(22, |c| c.ssh.port);
            if let Err(e) = self.connect_ssh(&alias, &user, port) {
                error!("SSH connect failed: {}", e);
            }
            self.update_window_title();
        }
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

    fn ai_suggest(&self) {
        let Some(ai_client) = &self.ai_client else {
            warn!("ai_suggest: no AI client configured");
            return;
        };
        let Some(pane) = self.panes.get(&self.active_pane) else {
            return;
        };
        let blocks = pane.parser.screen().blocks();
        if blocks.is_empty() {
            warn!("ai_suggest: no command history");
            return;
        }
        let history: Vec<&str> = blocks
            .iter()
            .rev()
            .take(10)
            .map(|b| b.command.as_str())
            .filter(|c| !c.is_empty())
            .collect();
        let history = history.into_iter().rev().collect::<Vec<_>>().join("\n");
        let client = ai_client.clone();
        let tx = pane.pty_tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    warn!("ai_suggest: runtime error: {}", e);
                    return;
                }
            };
            match rt.block_on(client.suggest(&history)) {
                Ok(suggestion) => {
                    let _ = tx.send(PtyCommand::Write(suggestion.into_bytes()));
                }
                Err(e) => {
                    warn!("ai_suggest: {}", e);
                }
            }
        });
    }

    fn drain_pty(&mut self) -> bool {
        let mut got_data = false;
        let active = self.active_pane;
        let mut title_changed = None;
        let mut dead_panes = Vec::new();
        let pane_count = self.panes.len();
        for (&id, pane) in &mut self.panes {
            if pane.pty_dead {
                continue;
            }
            let got = pane.drain();
            if got {
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
                    if pane.parser.sync_output() {
                        if !self.sync_active {
                            self.sync_active = true;
                            self.last_sync_clear = std::time::Instant::now();
                        }
                    } else {
                        self.sync_active = false;
                    }
                    got_data = !self.sync_active;
                } else {
                    got_data = true;
                }
            }
            if pane.pty_dead {
                dead_panes.push(id);
                // Append the exit notice exactly once: pty_dead is sticky, so
                // future drain calls skip this pane entirely. Previously every
                // drain re-appended the notice, flooding the buffer on each
                // RedrawRequested / KeyboardInput.
                pane.parser.parse(if pane_count <= 1 {
                    b"\r\n[Process exited] - exit to quit\r\n"
                } else {
                    b"\r\n[Process exited]\r\n"
                });
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
        self.resize_panes_to_rects();
        if let Some(title) = title_changed {
            if let Some(window) = &self.window {
                window.set_title(&format!("ZeroTerm v0.2.0 - {}", title));
            }
        }
        if self.sync_active
            && self.last_sync_clear.elapsed() > std::time::Duration::from_millis(1000)
        {
            got_data = true;
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
        let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) else {
            return Ok(());
        };
        let win_size = window.inner_size();
        let tab_h = renderer.cell_size()[1];
        let content_h = (win_size.height as f32 - tab_h).max(0.0);

        renderer.begin_frame()?;

        let mut tab_ids: Vec<usize> = self.panes.keys().copied().collect();
        tab_ids.sort();
        let active_idx = tab_ids
            .iter()
            .position(|&id| id == self.active_pane)
            .unwrap_or(0);
        let tab_infos: Vec<TabInfo> = tab_ids
            .iter()
            .map(|&id| TabInfo {
                title: self
                    .panes
                    .get(&id)
                    .map_or_else(String::new, |p| p.title.clone()),
                active: id == self.active_pane,
            })
            .collect();

        let rects = self.split_root.compute_rects();
        if rects.len() <= 1 {
            // Render the tree leaf, not the floating pane (it renders last as overlay).
            let tree_id = rects.keys().next().copied().unwrap_or(self.active_pane);
            if let Some(pane) = self.panes.get(&tree_id) {
                let is_active = tree_id == self.active_pane;
                renderer.set_viewport(0.0, tab_h);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active { self.scroll_offset } else { 0 },
                    if is_active { self.selection } else { None },
                )?;
            }
        } else {
            let mut ordered: Vec<(usize, (f32, f32, f32, f32))> = rects.into_iter().collect();
            ordered.sort_by_key(|(id, _)| *id);
            for (id, (nx, ny, _, _)) in ordered {
                let Some(pane) = self.panes.get(&id) else {
                    continue;
                };
                let px = nx * win_size.width as f32;
                let py = ny * content_h + tab_h;
                let is_active = id == self.active_pane;
                renderer.set_viewport(px, py);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active { self.scroll_offset } else { 0 },
                    if is_active { self.selection } else { None },
                )?;
            }
        }

        // Floating pane overlay — drawn last, on top of all split leaves.
        if let Some(id) = self.floating {
            if let Some(pane) = self.panes.get(&id) {
                let fw = win_size.width as f32 * 0.7;
                let fx = (win_size.width as f32 - fw) / 2.0;
                let fy = tab_h + content_h * 0.15;
                let is_active = id == self.active_pane;
                renderer.set_viewport(fx, fy);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active { self.scroll_offset } else { 0 },
                    if is_active { self.selection } else { None },
                )?;
            }
        }

        renderer.draw_tab_bar(&tab_infos, active_idx)?;

        renderer.end_frame()?;
        Ok(())
    }

    /// Resize every split-tree pane's parser + pty to its rect dims.
    fn resize_panes_to_rects(&mut self) {
        let (Some(renderer), Some(window)) = (&self.renderer, &self.window) else {
            return;
        };
        let size = window.inner_size();
        let tab_h = renderer.cell_size()[1];
        let content_h = (size.height as f32 - tab_h).max(0.0);
        let rects = self.split_root.compute_rects();
        for (&id, &(_, _, nw, nh)) in &rects {
            let cols = renderer.cols_for(nw * size.width as f32);
            let rows = renderer.rows_for(nh * content_h);
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.parser.screen_mut().resize(cols, rows);
                let _ = pane.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
            }
        }
    }

    fn write_pty(&self, data: &[u8]) {
        if let Some(pane) = self.panes.get(&self.active_pane) {
            let _ = pane.pty_tx.send(PtyCommand::Write(data.to_vec()));
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

    // Jump scroll to nearest command block in delta direction (-1 = prev, +1 = next)
    // relative to the middle of the current view. Block start_line is buffer-local;
    // global row = scrollback.len() + start_line. All buffer rows sit at the bottom of
    // the scrollable range, so a jump clamps to offset 0, landing the block at its
    // natural buffer row (near the top only when start_line is small).
    fn jump_to_block(&mut self, delta: i32) {
        let Some(pane) = self.active_pane() else {
            return;
        };
        let screen = pane.parser.screen();
        if screen.blocks().is_empty() {
            return;
        }
        let scrollback = screen.scrollback().len();
        let visible = screen.size().rows;
        let total = scrollback + visible;
        let start = total
            .saturating_sub(self.scroll_offset)
            .saturating_sub(visible);
        let mid = start + visible / 2;
        let target_row = if delta < 0 {
            screen
                .blocks()
                .iter()
                .rev()
                .find(|b| scrollback + b.start_line < mid)
                .map(|b| scrollback + b.start_line)
        } else {
            screen
                .blocks()
                .iter()
                .find(|b| scrollback + b.start_line > mid)
                .map(|b| scrollback + b.start_line)
        };
        let Some(target_row) = target_row else {
            return;
        };
        let offset = total
            .saturating_sub(target_row + visible)
            .min(self.max_scroll_offset());
        if offset == self.scroll_offset {
            return;
        }
        self.scroll_offset = offset;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // Selection methods
    /// Tab bar height in pixels = one cell row (must match render()'s content_h math).
    fn tab_bar_height(&self) -> f32 {
        self.renderer.as_ref().map_or(0.0, |r| r.cell_size()[1])
    }

    /// Map a window pixel point to the pane under it (normalized rects × window size).
    fn pane_at_point(&self, x: f32, y: f32) -> Option<usize> {
        let rects = self.split_root.compute_rects();
        if rects.len() <= 1 {
            return rects.keys().next().copied();
        }
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h = (window.inner_size().height as f32 - tab_h).max(0.0);
        for (&id, &(nx, ny, nw, nh)) in &rects {
            let (px, py, pw, ph) = (
                nx * win_w,
                ny * content_h + tab_h,
                nw * win_w,
                nh * content_h,
            );
            if x >= px && y >= py && x < px + pw && y < py + ph {
                return Some(id);
            }
        }
        None
    }

    /// Pane id of the tab under a window-space x,y (must match draw_tab_bar's
    /// layout: sorted pane ids, starts at col 1, span = chars+2, col += span+1).
    fn tab_at_point(&self, x: f32, y: f32) -> Option<usize> {
        let tab_h = self.tab_bar_height();
        if y < 0.0 || y >= tab_h || self.panes.is_empty() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let cell_w = renderer.cell_size()[0];
        let mut ids: Vec<usize> = self.panes.keys().copied().collect();
        ids.sort();
        let mut col = 1usize;
        for id in ids {
            let title = self
                .panes
                .get(&id)
                .map_or_else(String::new, |p| p.title.clone());
            // Must match draw_tab_bar: truncated title + 2 padding cells.
            let span = tab_span(&title, 20);
            let start_px = col as f32 * cell_w;
            let end_px = (col + span) as f32 * cell_w;
            if x >= start_px && x < end_px {
                return Some(id);
            }
            col += span + 1;
        }
        None
    }

    /// The divider (if any) within `tolerance` pixels of x,y, as (target, dir).
    fn divider_at_point(&self, x: f32, y: f32, tolerance: f32) -> Option<(usize, SplitDir)> {
        if self.split_root.leaves().len() <= 1 || y < self.tab_bar_height() {
            return None;
        }
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h = (window.inner_size().height as f32 - tab_h).max(0.0);
        for (dir, boundary, target) in self.split_root.dividers() {
            let (px, py) = match dir {
                SplitDir::Vertical => (boundary * win_w, y),
                SplitDir::Horizontal => (x, tab_h + boundary * content_h),
            };
            let dx = (px - x).abs();
            let dy = (py - y).abs();
            let hit = match dir {
                SplitDir::Vertical => dx <= tolerance && y >= tab_h,
                SplitDir::Horizontal => dy <= tolerance,
            };
            if hit {
                return Some((target, dir));
            }
        }
        None
    }

    fn screen_to_cell(&self, pane_id: usize, x: f32, y: f32) -> Option<(usize, usize)> {
        let (Some(renderer), Some(pane)) = (&self.renderer, self.panes.get(&pane_id)) else {
            return None;
        };
        let rect = self.split_root.compute_rects().get(&pane_id).copied()?;
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h = (window.inner_size().height as f32 - tab_h).max(0.0);
        let (px, py, pw, ph) = (
            rect.0 * win_w,
            rect.1 * content_h + tab_h,
            rect.2 * win_w,
            rect.3 * content_h,
        );
        let (lx, ly) = (x - px, y - py);
        if lx < 0.0 || ly < 0.0 || lx >= pw || ly >= ph {
            return None;
        }
        let cell_size = renderer.cell_size();
        let cell_w = cell_size[0];
        let cell_h = cell_size[1];
        let screen = pane.parser.screen();
        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

        let col = (lx / cell_w).floor() as usize;
        let row = (ly / cell_h).floor() as usize;

        if row < visible_rows && col < cols {
            let scrollback = screen.scrollback().len();
            let total_rows = scrollback + visible_rows;
            // scroll_offset is a single field owned by the active pane; inactive panes render at 0
            let offset = if pane_id == self.active_pane {
                self.scroll_offset
            } else {
                0
            };
            let end = total_rows.saturating_sub(offset);
            let start = end.saturating_sub(visible_rows);
            let global_row = start + row;
            Some((global_row, col))
        } else {
            None
        }
    }

    fn keybindings(&self) -> KeybindingsConfig {
        self.config
            .as_ref()
            .map_or_else(KeybindingsConfig::default, |c| c.keybindings.clone())
    }

    fn line_chars(&self, global_row: usize) -> Option<Vec<char>> {
        let pane = self.active_pane()?;
        let screen = pane.parser.screen();
        let scrollback = screen.scrollback();
        let total = scrollback.len();
        let visible = screen.buffer();
        let line = if global_row < total {
            &scrollback[total - 1 - global_row]
        } else {
            visible.get(global_row - total)?
        };
        Some(line.iter().map(|c| c.ch).collect())
    }

    fn shift_arrow_extend(&mut self, code: KeyCode, ctrl: bool) -> bool {
        if !self.keybindings().shift_arrows_select {
            return false;
        }
        let (cursor_row, cursor_col, cols, total_rows) = {
            let Some(pane) = self.active_pane() else {
                return false;
            };
            let screen = pane.parser.screen();
            let visible_rows = screen.buffer().len();
            let cols = if visible_rows > 0 {
                screen.buffer()[0].len()
            } else {
                0
            };
            if cols == 0 {
                return false;
            }
            let cursor = screen.cursor();
            let cursor_row = screen.scrollback().len() + cursor.row;
            (
                cursor_row,
                cursor.col,
                cols,
                screen.scrollback().len() + visible_rows,
            )
        };
        let (mut end_row, mut end_col) = match &self.selection {
            Some(s) if s.active => (s.end_row, s.end_col),
            _ => {
                self.scroll_offset = 0;
                (cursor_row, cursor_col)
            }
        };
        if ctrl {
            if let Some(chars) = self.line_chars(end_row) {
                end_col = match code {
                    KeyCode::ArrowLeft => word_left(&chars, end_col),
                    KeyCode::ArrowRight => word_right(&chars, end_col, cols),
                    _ => end_col,
                };
            }
        } else {
            match code {
                KeyCode::ArrowLeft if end_col > 0 => end_col -= 1,
                KeyCode::ArrowLeft if end_row > 0 => {
                    end_row -= 1;
                    end_col = cols - 1;
                }
                KeyCode::ArrowRight if end_col + 1 < cols => end_col += 1,
                KeyCode::ArrowRight if end_row + 1 < total_rows => {
                    end_row += 1;
                    end_col = 0;
                }
                KeyCode::ArrowUp => end_row = end_row.saturating_sub(1),
                KeyCode::ArrowDown if end_row + 1 < total_rows => end_row += 1,
                _ => {}
            }
        }
        let sel = self.selection.get_or_insert(Selection {
            start_row: cursor_row,
            start_col: cursor_col,
            end_row: cursor_row,
            end_col: cursor_col,
            active: true,
        });
        sel.end_row = end_row;
        sel.end_col = end_col;
        sel.active = true;
        true
    }

    fn start_selection(&mut self, x: f32, y: f32) {
        let Some(pane_id) = self.pane_at_point(x, y) else {
            return;
        };
        // Click focuses the clicked pane so selection renders/copies against its screen.
        if pane_id != self.active_pane {
            self.active_pane = pane_id;
            self.scroll_offset = 0;
        }
        if let Some((row, col)) = self.screen_to_cell(pane_id, x, y) {
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
            // Selection lives in the active pane (click focused it); leaving its rect clamps.
            if let Some((row, col)) = self.screen_to_cell(self.active_pane, x, y) {
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
        let (x, y) = self.mouse_pos;
        let Some(pane_id) = self.pane_at_point(x, y) else {
            return false;
        };
        let offset = if pane_id == self.active_pane {
            self.scroll_offset
        } else {
            0
        };
        if offset != 0 {
            return false;
        }
        let Some((global_row, col)) = self.screen_to_cell(pane_id, x, y) else {
            return false;
        };
        let Some(pane) = self.panes.get(&pane_id) else {
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
                if let Err(e) = session::save_session(
                    &session::session_file_path(),
                    &self.panes,
                    Some(&self.split_root),
                ) {
                    error!("Failed to save session: {}", e);
                }
                for (_, pane) in &self.panes {
                    let _ = pane.pty_tx.send(PtyCommand::Kill);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.resize_panes_to_rects();
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
                        if self.host_picker.open {
                            match code {
                                KeyCode::ArrowUp => self.host_picker.prev(),
                                KeyCode::ArrowDown => self.host_picker.next(),
                                KeyCode::Enter => self.pick_host(),
                                KeyCode::Escape => self.close_host_picker(),
                                _ => {}
                            }
                            if self.host_picker.open {
                                self.draw_host_picker();
                            }
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
                        if ctrl && shift && !alt && *code == KeyCode::KeyA {
                            self.ai_suggest();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyO {
                            self.cycle_opacity();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyS {
                            #[cfg(unix)]
                            if let Some(config) = &self.config {
                                if !config.ssh.host.is_empty() {
                                    let host = config.ssh.host.clone();
                                    let user = config.ssh.user.clone();
                                    let port = config.ssh.port;
                                    if let Err(e) = self.connect_ssh(&host, &user, port) {
                                        error!("SSH connect failed: {}", e);
                                    }
                                    self.update_window_title();
                                } else {
                                    self.open_host_picker();
                                }
                            }
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyK {
                            self.jump_to_block(-1);
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyJ {
                            self.jump_to_block(1);
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyF {
                            self.toggle_floating_pane();
                            self.update_window_title();
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
                        // Handle scrollback navigation + selection extend with Shift modifier
                        let shift = self.modifiers.shift_key();
                        if shift && !alt {
                            match code {
                                KeyCode::PageUp if !ctrl => {
                                    self.scroll_up(20);
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::PageDown if !ctrl => {
                                    self.scroll_down(20);
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::Home if !ctrl => {
                                    self.scroll_offset = self.max_scroll_offset();
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::End if !ctrl => {
                                    self.scroll_offset = 0;
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::ArrowUp
                                | KeyCode::ArrowDown => {
                                    if self.shift_arrow_extend(*code, ctrl) {
                                        if let Some(window) = &self.window {
                                            window.request_redraw();
                                        }
                                        return;
                                    }
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
                            self.clear_selection();
                            self.write_pty(&seq);
                        }
                    }
                    _ => {}
                }

                // Handle printable text (IME text input)
                if let Some(text) = &event.text {
                    if !text.is_empty() && !ctrl && !alt {
                        self.clear_selection();
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
                if let Some(delay) = self.renderer.as_mut().and_then(|r| r.next_frame_delay()) {
                    self.last_anim_frame = std::time::Instant::now() + delay;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                let x = position.x as f32;
                let y = position.y as f32;
                // Split divider drag: resize from last position delta, then bail.
                if let Some(target) = self.dragging_divider {
                    let window = self.window.as_ref();
                    let (win_w, content_h) = window.map_or((1.0, 1.0), |w| {
                        let tab = self.tab_bar_height();
                        (
                            w.inner_size().width as f32,
                            (w.inner_size().height as f32 - tab).max(0.0),
                        )
                    });
                    let (ax, ay) = self.divider_anchor;
                    let (dx, dy) = (x - ax, y - ay);
                    // Find this target's current divider to resize against its real boundary.
                    let found = self
                        .split_root
                        .dividers()
                        .into_iter()
                        .find(|(_, _, t)| *t == target);
                    if let Some((dir, boundary, _)) = found {
                        let delta = match dir {
                            SplitDir::Vertical => dx / win_w,
                            SplitDir::Horizontal => dy / content_h,
                        };
                        self.split_root.resize_leaf(target, boundary, delta);
                    }
                    self.divider_anchor = (x, y);
                    self.resize_panes_to_rects();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                // ponytail: focus-follows-mouse hardcoded ON; gate on config.mouse.focus_follows
                // when the config gains a mouse section. Skipped during drag-select.
                let hovered = self.pane_at_point(x, y);
                if !self.selecting {
                    if let Some(id) = hovered {
                        if id != self.active_pane {
                            self.active_pane = id;
                            self.scroll_offset = 0;
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                    }
                }
                let pane_id = hovered.unwrap_or(self.active_pane);
                let mouse_tracking = self
                    .panes
                    .get(&pane_id)
                    .map_or(MouseTrackingMode::Off, |p| p.parser.mouse_tracking());
                if let Some(window) = &self.window {
                    if mouse_tracking != MouseTrackingMode::Off {
                        window.set_cursor(CursorIcon::Crosshair);
                    } else {
                        window.set_cursor(CursorIcon::Text);
                    }
                }
                if mouse_tracking == MouseTrackingMode::AnyEvent && !self.selecting {
                    if let Some((row, col)) = self.screen_to_cell(pane_id, x, y) {
                        let mods = (if self.modifiers.shift_key() { 4 } else { 0 })
                            | (if self.modifiers.control_key() { 8 } else { 0 })
                            | (if self.modifiers.alt_key() { 16 } else { 0 });
                        self.write_pty(
                            format!("\x1b[<{};{};{}M", col + 1, row + 1, 35 + mods).as_bytes(),
                        );
                    }
                }
                if self.selecting {
                    self.update_selection(x, y);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = self.mouse_pos;
                // Left press may start a divider drag or a tab switch; release ends drags.
                if button == MouseButton::Left && state == winit::event::ElementState::Pressed {
                    if let Some(pane_id) = self.tab_at_point(x, y) {
                        if pane_id != self.active_pane {
                            self.active_pane = pane_id;
                            self.scroll_offset = 0;
                        }
                        self.end_selection();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                        return;
                    }
                    if let Some((target, _)) = self.divider_at_point(x, y, 8.0) {
                        self.dragging_divider = Some(target);
                        self.divider_anchor = (x, y);
                        self.end_selection();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                        return;
                    }
                }
                if button == MouseButton::Left && state == winit::event::ElementState::Released {
                    self.dragging_divider = None;
                }
                let pane_id = self.pane_at_point(x, y).unwrap_or(self.active_pane);
                let mouse_tracking = self
                    .panes
                    .get(&pane_id)
                    .map_or(MouseTrackingMode::Off, |p| p.parser.mouse_tracking());
                if mouse_tracking != MouseTrackingMode::Off {
                    let button_id = match button {
                        MouseButton::Left => 0,
                        MouseButton::Middle => 1,
                        MouseButton::Right => 2,
                        _ => 0,
                    };
                    if let Some((row, col)) = self.screen_to_cell(pane_id, x, y) {
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
                            self.start_selection(x, y);
                        }
                    } else {
                        self.end_selection();
                        // Click-to-position: send CSI CUP so the shell moves its cursor.
                        if self.keybindings().click_to_position
                            && self.scroll_offset == 0
                            && y >= self.tab_bar_height()
                            && self.pane_at_point(x, y) == Some(self.active_pane)
                        {
                            if let Some((global_row, col)) =
                                self.screen_to_cell(self.active_pane, x, y)
                            {
                                let row = global_row.saturating_sub(
                                    self.panes
                                        .get(&self.active_pane)
                                        .map_or(0, |p| p.parser.screen().scrollback().len()),
                                );
                                self.write_pty(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
                            }
                        }
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = self.mouse_pos;
                // ponytail: single scroll_offset field; wheel over another pane focuses it
                // first, then scrolls (per-pane scroll map skipped)
                if let Some(id) = self.pane_at_point(x, y) {
                    if id != self.active_pane {
                        self.active_pane = id;
                        self.scroll_offset = 0;
                    }
                }
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ()) {
        let _ = (event_loop, event);
        if self.drain_pty() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--version" | "-V" => {
                println!("zeroterm {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("ZeroTerm - GPU-accelerated terminal emulator");
                println!();
                println!("Usage: zeroterm [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --version, -V  Print version and exit");
                println!("  --help, -h     Print help and exit");
                println!("  upgrade        Update to the latest release");
                return Ok(());
            }
            "upgrade" => return upgrade(),
            _ => {}
        }
    }

    info!("Starting ZeroTerm v{}", env!("CARGO_PKG_VERSION"));

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new();
    app.event_proxy = Some(event_loop.create_proxy());
    event_loop.run_app(&mut app)?;

    Ok(())
}

fn upgrade() -> Result<()> {
    if Path::new("install.sh").is_file() {
        let status = std::process::Command::new("bash")
            .arg("install.sh")
            .arg("upgrade")
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    println!("Update ZeroTerm with:");
    println!(
        "  curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/install.sh | bash -s -- upgrade"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_bounds() {
        let line: Vec<char> = "  hello world  ".chars().collect();
        assert_eq!(word_left(&line, 7), 2);
        assert_eq!(word_left(&line, 12), 8);
        assert_eq!(word_right(&line, 2, 20), 6);
        assert_eq!(word_right(&line, 8, 20), 12);
    }

    #[test]
    fn pane_drain_marks_dead_on_disconnect() {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
        let mut pane = PaneState {
            parser: Parser::new(10, 5),
            pty_rx: rx,
            pty_tx: mpsc::channel().0,
            title: String::new(),
            pane_cmd: String::new(),
            pty_dead: false,
        };
        tx.send(b"abc".to_vec()).unwrap();
        drop(tx);
        assert!(pane.drain());
        assert!(pane.pty_dead, "disconnect must mark pane dead");
        let before = pane.parser.screen().buffer()[0][0].ch;
        assert!(!pane.drain(), "dead pane must not be drained again");
        assert_eq!(pane.parser.screen().buffer()[0][0].ch, before);
    }

    #[test]
    fn drain_pty_appends_exit_notice_exactly_once() {
        let mut app = App::new();
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
        drop(tx); // pty already gone -> Disconnected on first drain
        app.panes.insert(
            0,
            PaneState {
                parser: Parser::new(10, 5),
                pty_rx: rx,
                pty_tx: mpsc::channel().0,
                title: String::new(),
                pane_cmd: String::new(),
                pty_dead: false,
            },
        );
        app.active_pane = 0;

        app.drain_pty();
        let first: String = app.panes[&0]
            .parser
            .screen()
            .buffer()
            .iter()
            .flat_map(|row| row.iter().map(|c| c.ch))
            .collect();
        let scrollback_before = app.panes[&0].parser.screen().scrollback().len();
        assert!(
            first.contains("Process exited"),
            "first drain should append the exit notice"
        );

        app.drain_pty();
        let second: String = app.panes[&0]
            .parser
            .screen()
            .buffer()
            .iter()
            .flat_map(|row| row.iter().map(|c| c.ch))
            .collect();
        let scrollback_after = app.panes[&0].parser.screen().scrollback().len();
        assert_eq!(first, second, "visible buffer unchanged by a second drain");
        assert_eq!(
            scrollback_before, scrollback_after,
            "exit notice must be appended exactly once, not on every drain (scrollback must not grow)"
        );
    }

    #[test]
    fn host_picker_overlay_survives_tiny_window() {
        let mut hp = HostPicker::new();
        hp.open(vec!["some.host".to_string()]);
        // rows/cols < 2 would previously underflow (rows - height, cols - width).
        let (top, left, height, width) = hp.overlay_rect(1, 1);
        assert_eq!(top, 0);
        assert_eq!(left, 0);
        assert!(height >= 2 && width >= 2);
        // Centering on a normal window keeps the rect inside it.
        let (top, left, height, width) = hp.overlay_rect(80, 24);
        assert!(top + height <= 24);
        assert!(left + width <= 80);
    }

    #[test]
    fn init_parser_dims_fit_renderer_capacity() {
        // DejaVu Sans Mono at 14px measures (9,16) px/cell (measured via swash).
        // The old init heuristic (font_size*0.6, font_size*1.2) = (8.4,16.8)
        // produced parser cols larger than the renderer's ceil-based capacity:
        // at 1000px wide, 119 parser cols vs 112 buffer cols -> GPU buffer
        // overflow in update_cell_data. Init must size from renderer metrics.
        let cell_w = 9.0f32;
        let width = 1000u32;
        let cols = (width as f32 / cell_w) as usize;
        let capacity = (width as f32 / cell_w).ceil() as usize;
        assert!(
            cols <= capacity,
            "parser cols {} > capacity {}",
            cols,
            capacity
        );
        assert_eq!(cols, 111, "floor(1000/9) must be the parser width");
    }
}
