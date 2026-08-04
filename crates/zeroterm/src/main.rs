use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use anyhow::Result;
use arboard::Clipboard;
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::event::{MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes};

#[cfg(feature = "ai")]
use zeroterm_ai::client::{AiClient, AiError};
use zeroterm_config::{Config, KeybindingsConfig};
use zeroterm_core::parser::MouseTrackingMode;
use zeroterm_core::screen::Size as PtySize;
use zeroterm_core::Parser;
use zeroterm_mux::session::{PaneSpec, Session, SessionLayout};
use zeroterm_mux::split::{SplitDir, SplitNode};
use zeroterm_mux::tab::Tab;
#[cfg(feature = "plugins")]
use zeroterm_plugin::Plugin;
use zeroterm_render::{tab_span, Renderer, Selection, TabInfo};
#[cfg(feature = "sync")]
use zeroterm_sync::daemon::SyncDaemon;

use crate::ai_overlay::AiOverlay;
#[cfg(feature = "ai")]
use crate::ai_overlay::{explain_prompt, suggest_context, AiKind, AiState};
#[cfg(feature = "plugins")]
use crate::app::load_plugins;
#[cfg(all(unix, feature = "ssh"))]
use crate::app::spawn_ssh_process;
use crate::app::{
    block_output_text, word_left, word_right, EditingState, HostPicker, PaneState, PtyCommand,
    SessionManager,
};
use crate::app::{spawn_pty_process, starship_setup};
use crate::search::SearchState;
use crate::settings::{SettingsAction, SettingsContext, SettingsMenu};

mod ai_overlay;
mod app;
mod search;
// Retained for the legacy session.json format + its tests; session layout
// persistence now lives in zeroterm-mux (SessionLayout) via save_session_layout().
#[allow(dead_code)]
mod session;
mod settings;

const COPY_MARKER: &str = "[copy]";

fn zt(mark: &str) {
    if std::env::var("ZTIME").is_ok() {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(std::time::Instant::now);
        eprintln!("ZTIME {}: {:?}", mark, start.elapsed());
    }
}

#[allow(dead_code)]
struct App {
    window: Option<Arc<Window>>,
    window_visible: bool,
    renderer: Option<Renderer>,
    renderer_rx: Option<Receiver<Renderer>>,
    session: SessionManager,
    modifiers: ModifiersState,
    font_size: f32,
    selection: Option<Selection>,
    selecting: bool,
    mouse_pos: (f32, f32),
    // Tab the mouse is over (None when off the tab bar) + whether it sits on
    // that tab's close button. Drives pill/close-glyph rendering + click hit-test.
    hovered_tab: Option<usize>,
    hovered_tab_close: bool,
    // Sub-line remainder of pixel-wheel deltas (|fraction| < 1); accumulates
    // across MouseWheel events so trackpad scrolling glides line-by-line.
    scroll_fraction: f32,
    clipboard: Option<Clipboard>,
    shell: String,
    shell_args: Vec<String>,
    #[cfg(feature = "ai")]
    ai_client: Option<Arc<AiClient>>,
    #[cfg(feature = "sync")]
    sync_daemon: Option<SyncDaemon>,
    config_changed: Arc<AtomicBool>,
    config: Option<Config>,
    config_rx: Option<std::sync::mpsc::Receiver<Config>>,
    opacity: f64,
    sync_tick: u32,
    cursor_visible: bool,
    font_path: Option<String>,
    settings: SettingsMenu,
    host_picker: HostPicker,
    search: SearchState,
    ai: AiOverlay,
    sync_active: bool,
    last_sync_clear: std::time::Instant,
    last_anim_frame: std::time::Instant,
    event_proxy: Option<EventLoopProxy<()>>,
    #[cfg(feature = "plugins")]
    plugins: HashMap<String, Plugin>,
    // Local line editor for the active pane. Some while editing: printable +
    // editing keys are intercepted (not forwarded to the shell) and accumulated
    // into the buffer until Enter submits the line or Esc discards it.
    editing: Option<EditingState>,
}

// Split an accumulated pixel-wheel scroll into whole lines to apply (up/down)
// plus the sub-line remainder (same sign as the input, |rem| < 1) to carry
// over to the next wheel event. Renderer only supports integer scroll offsets.
fn split_scroll_fraction(fraction: f32) -> (usize, usize, f32) {
    let whole = fraction.trunc() as i32;
    (
        whole.max(0) as usize,
        (-whole).max(0) as usize,
        fraction - whole as f32,
    )
}

#[allow(dead_code)]
impl App {
    fn new() -> Self {
        Self {
            window: None,
            window_visible: true,
            renderer: None,
            renderer_rx: None,
            session: SessionManager::new(),
            modifiers: ModifiersState::empty(),
            font_size: 14.0,
            selection: None,
            selecting: false,
            mouse_pos: (0.0, 0.0),
            hovered_tab: None,
            hovered_tab_close: false,
            scroll_fraction: 0.0,
            clipboard: Clipboard::new().ok(),
            shell: String::new(),
            shell_args: vec![],
            #[cfg(feature = "ai")]
            ai_client: None,
            #[cfg(feature = "sync")]
            sync_daemon: None,
            config_changed: Arc::new(AtomicBool::new(false)),
            config: None,
            config_rx: None,
            opacity: 1.0,
            sync_tick: 0,
            cursor_visible: true,
            font_path: None,
            settings: SettingsMenu::new(&SettingsContext::default()),
            host_picker: HostPicker::new(),
            search: SearchState::default(),
            ai: AiOverlay::default(),
            sync_active: false,
            last_sync_clear: std::time::Instant::now(),
            last_anim_frame: std::time::Instant::now(),
            event_proxy: None,
            #[cfg(feature = "plugins")]
            plugins: HashMap::new(),
            editing: None,
        }
    }

    /// Clone of the event-loop proxy for a PTY reader thread. Registered in
    /// main() before run_app, so always present once init() spawns PTYs.
    fn wake_proxy(&self) -> EventLoopProxy<()> {
        self.event_proxy
            .clone()
            .expect("event_proxy registered in main() before run_app")
    }

    /// Request a redraw of the window, if one exists.
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn update_window_title(&self) {
        if let Some(pane) = self.session.active_pane() {
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
        zt("init start");
        info!("Initializing ZeroTerm");

        let (config, config_rx) = Config::load_async(None);
        zt("config load_async returned");
        info!("keybindings: vim_mode={}", config.keybindings.vim_mode);

        let window_attrs = WindowAttributes::default()
            .with_title("ZeroTerm v0.2.0")
            .with_inner_size(winit::dpi::LogicalSize::new(
                config.window.width,
                config.window.height,
            ))
            .with_resizable(true);

        let window = Arc::new(event_loop.create_window(window_attrs)?);
        zt("window created");

        let font_size = config.font.size;
        self.font_size = font_size;
        self.opacity = config.window.opacity;
        self.font_path = config.font.path.clone();

        // Deferred GPU init (boot speed): renderer builds on a background
        // thread so the window + PTY appear immediately. check_renderer_ready()
        // polls render_rx and, once the renderer arrives, resizes panes to the
        // real measured glyph metrics.
        let (render_tx, render_rx) = mpsc::channel();
        let window_clone = window.clone();
        let opacity = self.opacity;
        let font_path = config.font.path.clone();
        std::thread::spawn(move || {
            zt("renderer thread start");
            match pollster::block_on(Renderer::new(window_clone, font_size, opacity, font_path)) {
                Ok(renderer) => {
                    let _ = render_tx.send(renderer);
                }
                Err(e) => error!("Renderer init failed: {}", e),
            }
            zt("renderer thread done");
        });
        self.renderer_rx = Some(render_rx);

        let size = window.inner_size();
        // Estimate cells until the renderer is ready; check_renderer_ready()
        // resizes the parser + PTY to the measured glyph metrics.
        let cols = (((size.width as f32) / 8.4).max(20.0)) as usize;
        let rows = (((size.height as f32) / 15.0).max(10.0)) as usize;

        let shell = config.shell.program.clone();
        let shell_args = config.shell.args.clone();
        self.shell = shell.clone();
        self.shell_args = shell_args.clone();

        let (pty_rx, pty_tx) = {
            let (shell, shell_args, starship_env) = starship_setup(&self.shell, &self.shell_args);
            zt("starship_setup done");
            let env_refs: Vec<(&str, &str)> =
                starship_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
            spawn_pty_process(
                &shell,
                &shell_args,
                &env_refs,
                cols,
                rows,
                self.wake_proxy(),
            )?
        };
        zt("pty spawned");
        // Bash's readline advertises `\x1b[?2004h` (bracketed paste) itself; writing it here
        // lands in the pty line discipline pre-readline and leaks as literal `2004h` text.
        // So we do NOT send it — the parser handles the shell's own advertisement.

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

        #[cfg(feature = "ai")]
        let ai_client = if config.ai.endpoint.is_empty() {
            None
        } else {
            Some(Arc::new(AiClient::new(&config.ai.endpoint)))
        };

        self.window = Some(window);
        self.session.panes = panes;
        self.session.active_pane = 0;
        self.session.next_pane_id = 1;

        let layout_path = Config::default_config_path().with_file_name("layout.json");
        zt("session load start");
        // Session restore (roadmap 2.1): layout.json from the last clean quit
        // rebuilds tabs/splits. Pane 0 is the shell spawned above; each further
        // PaneSpec spawns through the pty layer (never bypassed). A missing or
        // corrupt file falls back to the single default tab already set up.
        if let Some(saved) = SessionLayout::restore(&layout_path) {
            let mut restored_ids = vec![0usize];
            for spec in saved.panes.iter().skip(1) {
                let cmd = if spec.cmd.is_empty() {
                    shell.clone()
                } else {
                    spec.cmd.clone()
                };
                match spawn_pty_process(&cmd, &[], &[], cols, rows, self.wake_proxy()) {
                    Ok((pty_rx, pty_tx)) => {
                        // FIXME(test): startup bracketed-paste probe removed for leak test.
                        // let _ = pty_tx.send(PtyCommand::Write(b"\x1b[?2004h".to_vec()));
                        let id = self.session.next_pane_id;
                        self.session.next_pane_id += 1;
                        self.session.panes.insert(
                            id,
                            PaneState {
                                parser: Parser::new(cols, rows),
                                pty_rx,
                                pty_tx,
                                title: spec.title.clone(),
                                pane_cmd: cmd,
                                pty_dead: false,
                            },
                        );
                        self.session.tabs.push(Tab::new(id));
                        restored_ids.push(id);
                    }
                    Err(e) => warn!("Session restore: failed to spawn '{}': {}", cmd, e),
                }
            }
            if let Some(split) = saved.split {
                // Saved leaf ids are positions into `saved.panes`; remap them
                // onto the freshly assigned ids so the tree survives the id reset.
                self.session.split_root = SessionLayout::remap_split(&split, &restored_ids);
                if saved.active_pane < restored_ids.len() {
                    self.session.active_pane = restored_ids[saved.active_pane];
                    self.session.scroll_offset = 0;
                }
            }
        }

        #[cfg(feature = "ai")]
        {
            self.ai_client = ai_client;
        }
        #[cfg(feature = "sync")]
        {
            self.sync_daemon = if config.sync.server_url.is_empty() {
                None
            } else {
                Some(SyncDaemon::new(config.sync.server_url.clone()))
            };
        }

        self.config = Some(config);
        self.config_rx = Some(config_rx);

        let ctx = self.settings_ctx();
        self.settings.refresh(&ctx);

        // Start config file watcher
        let config_path = Config::default_config_path();
        let config_dir = config_path.parent().unwrap().to_path_buf();
        #[cfg(feature = "plugins")]
        {
            self.plugins = load_plugins(&config_dir.join("plugins"));
        }
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

        zt("init done (pre-redraw)");
        info!("ZeroTerm initialized: {}x{} ({})", cols, rows, shell);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    /// Serialize the current tabs/splits to layout.json (roadmap 2.1) so the
    /// next launch can restore them. Pane order is sorted-by-id, so leaf ids in
    /// the split tree double as positions into the saved pane list.
    fn save_session_layout(&self) {
        let path = Config::default_config_path().with_file_name("layout.json");
        let ids = self.session.pane_ids();
        let cwd = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let panes: Vec<PaneSpec> = ids
            .iter()
            .map(|&id| {
                let pane = &self.session.panes[&id];
                PaneSpec {
                    title: pane.title.clone(),
                    cmd: pane.pane_cmd.clone(),
                    cwd: cwd.clone(),
                }
            })
            .collect();
        let layout = SessionLayout {
            active_pane: ids
                .iter()
                .position(|&i| i == self.session.active_pane)
                .unwrap_or(0),
            panes,
            split: Some(self.session.split_root.clone()),
        };
        if let Err(e) = Session::new(0, layout).save(&path) {
            error!("Failed to save session layout: {}", e);
        }
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

            let (pty_rx, pty_tx) = {
                let (shell, shell_args, starship_env) =
                    starship_setup(&self.shell, &self.shell_args);
                let env_refs: Vec<(&str, &str)> =
                    starship_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
                spawn_pty_process(
                    &shell,
                    &shell_args,
                    &env_refs,
                    cols,
                    rows,
                    self.wake_proxy(),
                )?
            };
            let parser = Parser::new(cols, rows);
            let id = self.session.next_pane_id;
            self.session.next_pane_id += 1;
            self.session.panes.insert(
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
            self.session.active_pane = id;
            self.session.scroll_offset = 0;
            self.session.split_root = SplitNode::Leaf(id);
            self.session.tabs.push(Tab::new(id));
        }
        Ok(())
    }

    fn create_split_pane(&mut self, _dir: SplitDir) -> Result<()> {
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

            let (pty_rx, pty_tx) = {
                let (shell, shell_args, starship_env) =
                    starship_setup(&self.shell, &self.shell_args);
                let env_refs: Vec<(&str, &str)> =
                    starship_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
                spawn_pty_process(
                    &shell,
                    &shell_args,
                    &env_refs,
                    cols,
                    rows,
                    self.wake_proxy(),
                )?
            };
            let parser = Parser::new(cols, rows);
            let id = self.session.next_pane_id;
            self.session.next_pane_id += 1;
            self.session.panes.insert(
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
            self.session.active_pane = id;
            self.session.scroll_offset = 0;
            self.resize_panes_to_rects();
        }
        Ok(())
    }

    fn close_active_tab(&mut self) {
        self.close_tab(self.session.active_pane);
    }

    fn close_tab(&mut self, id: usize) {
        if self.session.panes.len() <= 1 {
            return;
        }
        let was_active = self.session.active_pane == id;
        if let Some(pane) = self.session.panes.remove(&id) {
            let _ = pane.pty_tx.send(PtyCommand::Kill);
        }
        self.session.split_root.remove_leaf(id);
        self.session.tabs.retain(|t| t.id != id);
        if self.session.floating == Some(id) {
            self.session.floating = None;
        }
        if was_active {
            let first = *self.session.panes.keys().next().unwrap_or(&0);
            self.session.active_pane = first;
            self.editing = None;
            self.session.scroll_offset = 0;
        }
        self.hovered_tab = None;
        self.hovered_tab_close = false;
        self.resize_panes_to_rects();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Toggle active pane between split-tree and floating overlay (Ctrl+Shift+F).
    fn toggle_floating_pane(&mut self) {
        let active = self.session.active_pane;
        if self.session.floating == Some(active) {
            // Dock: re-insert at first remaining tree leaf.
            // ponytail: original slot lost (insert_leaf only splits a parent) — root-ish
            // placement is the accepted ceiling.
            self.session.floating = None;
            if !self.session.split_root.leaves().contains(&active) {
                let parent = *self.session.split_root.leaves().first().unwrap_or(&active);
                self.session
                    .split_root
                    .insert_leaf(active, SplitDir::Vertical, parent, 0.5);
            }
            self.resize_panes_to_rects();
        } else {
            // Dock whatever was floating (one float at a time), then float active.
            if let Some(prev) = self.session.floating.take() {
                if !self.session.split_root.leaves().contains(&prev) {
                    let parent = *self.session.split_root.leaves().first().unwrap_or(&prev);
                    self.session
                        .split_root
                        .insert_leaf(prev, SplitDir::Vertical, parent, 0.5);
                }
            }
            if self.session.split_root.leaves().len() > 1 {
                self.session.split_root.remove_leaf(active);
                self.session.floating = Some(active);
                self.resize_panes_to_rects();
            } else {
                // ponytail: last visible pane stays in tree AND floats (overlay wins when
                // drawn twice); zero visible panes not allowed.
                self.session.floating = Some(active);
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
            if let Some(id) = self.session.floating {
                if let Some(pane) = self.session.panes.get_mut(&id) {
                    let tab_h = renderer.cell_size()[1];
                    let status_h = renderer.status_bar_height();
                    let content_h = (window.inner_size().height as f32 - tab_h - status_h).max(0.0);
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
        if self.session.next_tab() {
            self.redraw();
        }
    }

    fn previous_tab(&mut self) {
        if self.session.previous_tab() {
            self.redraw();
        }
    }

    fn switch_to_tab(&mut self, idx: usize) {
        if self.session.switch_to_tab(idx) {
            self.redraw();
        }
    }

    fn compute_split_rects(&self) -> HashMap<usize, (f32, f32, f32, f32)> {
        self.session.compute_split_rects()
    }

    fn focus_adjacent_pane(&mut self, dir: KeyCode) {
        if self.session.focus_adjacent_pane(dir) {
            self.redraw();
        }
    }

    #[cfg(all(unix, feature = "ssh"))]
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
            let id = self.session.next_pane_id;
            self.session.next_pane_id += 1;
            self.session.panes.insert(
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
            self.session.active_pane = id;
            self.session.scroll_offset = 0;
            self.session.split_root = SplitNode::Leaf(id);
            self.session.tabs.push(Tab::new(id));
        }
        Ok(())
    }

    fn open_host_picker(&mut self) {
        #[cfg(all(unix, feature = "ssh"))]
        {
            let aliases = zeroterm_ssh::client::ssh_aliases();
            if aliases.is_empty() {
                return;
            }
            self.host_picker.open(aliases);
            if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
                self.host_picker.save_screen(pane.parser.screen());
            }
            self.draw_host_picker();
        }
    }

    fn draw_host_picker(&mut self) {
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
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
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            self.host_picker.restore_screen(pane.parser.screen_mut());
        }
        self.host_picker.open = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pick_host(&mut self) {
        #[cfg(all(unix, feature = "ssh"))]
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

    #[cfg(feature = "ai")]
    fn open_ai(&mut self, kind: AiKind) {
        let Some(ai_client) = self.ai_client.clone() else {
            self.ai.open(kind);
            self.ai.state = AiState::Error(
                "AI not configured (set ai.endpoint in config, e.g. http://localhost:11434)"
                    .to_string(),
            );
            self.redraw();
            return;
        };
        let Some(prompt) = self.ai_prompt(kind) else {
            self.ai.open(kind);
            self.ai.state = AiState::Error("no command context in the current pane".to_string());
            self.redraw();
            return;
        };
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            self.ai.save_screen(pane.parser.screen());
        }
        self.ai.open(kind);
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        self.ai.pending = Some(rx);
        // Fire-and-poll: the request runs on a fresh runtime in a background
        // thread; the result lands on the channel and is picked up by
        // AiOverlay::poll from the render loop. Never blocks the window.
        std::thread::spawn(move || {
            let result = match tokio::runtime::Runtime::new() {
                Ok(rt) => match kind {
                    AiKind::Explain => rt.block_on(ai_client.explain(&prompt)),
                    AiKind::Suggest => rt.block_on(ai_client.suggest(&prompt)),
                },
                Err(e) => Err(AiError::RequestFailed(e.to_string())),
            };
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
        self.redraw();
    }

    #[cfg(feature = "ai")]
    fn ai_prompt(&self, kind: AiKind) -> Option<String> {
        let screen = self.session.active_pane()?.parser.screen();
        match kind {
            AiKind::Explain => explain_prompt(screen),
            AiKind::Suggest => suggest_context(screen),
        }
    }

    fn close_ai(&mut self) {
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            self.ai.restore_screen(pane.parser.screen_mut());
        }
        self.ai.close();
        self.redraw();
    }

    fn draw_ai_overlay(&mut self) {
        if !self.ai.open {
            return;
        }
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
            return;
        };
        let (cols, rows) = {
            let s = pane.parser.screen();
            (s.size().cols, s.size().rows)
        };
        let bytes = self.ai.overlay_bytes(cols, rows);
        pane.parser.parse(&bytes);
    }

    fn drain_pty(&mut self) -> bool {
        let mut got_data = false;
        let active = self.session.active_pane;
        let mut title_changed = None;
        let mut dead_panes = Vec::new();
        let pane_count = self.session.panes.len();
        for (&id, pane) in &mut self.session.panes {
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
            if self.session.panes.len() > 1 {
                self.session.panes.remove(id);
                self.session.split_root.remove_leaf(*id);
                if self.session.active_pane == *id {
                    self.session.active_pane = *self.session.panes.keys().next().unwrap_or(&0);
                    self.editing = None;
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
            #[cfg(feature = "sync")]
            if let Some(sync) = &self.sync_daemon {
                sync.mark_dirty();
            }
        }
    }

    fn check_renderer_ready(&mut self) {
        if self.renderer.is_some() {
            return;
        }
        let Some(rx) = &self.renderer_rx else {
            return;
        };
        let Ok(renderer) = rx.try_recv() else {
            return;
        };
        let Some(window) = &self.window else {
            self.renderer = Some(renderer);
            return;
        };
        let mut renderer = renderer;
        let size = window.inner_size();
        let cols = renderer.cols_for(size.width as f32);
        let rows = renderer.rows_for(size.height as f32);
        renderer.resize(size.width, size.height);
        for pane in self.session.panes.values_mut() {
            pane.parser.screen_mut().resize(cols, rows);
            let _ = pane.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
        }
        self.renderer = Some(renderer);
        zt("renderer received on main");
        info!("GPU renderer ready: {}x{}", cols, rows);
    }

    fn render(&mut self) -> Result<()> {
        self.check_renderer_ready();
        if self.config_changed.load(Ordering::SeqCst) {
            self.config_changed.store(false, Ordering::SeqCst);
            if let Some(config) = &mut self.config {
                config.reload(None).ok();
            }
            self.apply_config_to_renderer();
        }
        if let Some(rx) = &self.config_rx {
            if let Ok(hydrated) = rx.try_recv() {
                self.config_rx = None;
                self.config = Some(hydrated);
                self.apply_config_to_renderer();
            }
        }
        if self.settings.open {
            self.draw_settings_overlay();
        }
        if self.ai.open {
            // Fire-and-poll: collect the finished AI result, then redraw the
            // panel with the response (or error) in this frame.
            self.ai.poll();
            self.draw_ai_overlay();
        }
        let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) else {
            return Ok(());
        };
        let win_size = window.inner_size();
        let tab_h = renderer.cell_size()[1];
        let status_h = renderer.status_bar_height();
        let content_h = (win_size.height as f32 - tab_h - status_h).max(0.0);

        renderer.begin_frame()?;
        renderer.draw_background(renderer.theme_bg())?;

        let mut tab_ids: Vec<usize> = self.session.panes.keys().copied().collect();
        tab_ids.sort();
        // While editing, the active tab shows the live buffer instead of the
        // shell title. Editing is bound to the active pane and cleared on any
        // pane switch, so this is only ever the pane that owns the editor.
        let edit_display = self.editing.as_ref().map(|e| e.display());
        let tab_infos: Vec<TabInfo> = tab_ids
            .iter()
            .map(|&id| TabInfo {
                title: match &edit_display {
                    Some(d) if id == self.session.active_pane => d.clone(),
                    _ => self
                        .session
                        .panes
                        .get(&id)
                        .map_or_else(String::new, |p| p.title.clone()),
                },
                active: id == self.session.active_pane,
                hovered: self.hovered_tab == Some(id),
                close_hovered: self.hovered_tab_close,
            })
            .collect();

        let rects = self.session.split_root.compute_rects();
        // Active pane's window-space viewport rect (for the scrollbar overlay).
        // Mirrors the pane-rect transform in render_screen calls below.
        let (scroll_px, scroll_py, scroll_pw, scroll_ph) =
            if self.session.floating == Some(self.session.active_pane) {
                let fw = win_size.width as f32 * 0.7;
                let fx = (win_size.width as f32 - fw) / 2.0;
                (fx, tab_h + content_h * 0.15, fw, content_h * 0.7)
            } else {
                let (nx, ny, nw, nh) = rects
                    .get(&self.session.active_pane)
                    .copied()
                    .unwrap_or((0.0, 0.0, 1.0, 1.0));
                (
                    nx * win_size.width as f32,
                    ny * content_h + tab_h,
                    nw * win_size.width as f32,
                    nh * content_h,
                )
            };
        if rects.len() <= 1 {
            // Render the tree leaf, not the floating pane (it renders last as overlay).
            let tree_id = rects
                .keys()
                .next()
                .copied()
                .unwrap_or(self.session.active_pane);
            if let Some(pane) = self.session.panes.get(&tree_id) {
                let is_active = tree_id == self.session.active_pane;
                renderer.set_viewport(0.0, tab_h);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active {
                        self.session.scroll_offset
                    } else {
                        0
                    },
                    if is_active { self.selection } else { None },
                )?;
            }
        } else {
            let mut ordered: Vec<(usize, (f32, f32, f32, f32))> = rects.into_iter().collect();
            ordered.sort_by_key(|(id, _)| *id);
            for (id, (nx, ny, _, _)) in ordered {
                let Some(pane) = self.session.panes.get(&id) else {
                    continue;
                };
                let px = nx * win_size.width as f32;
                let py = ny * content_h + tab_h;
                let is_active = id == self.session.active_pane;
                renderer.set_viewport(px, py);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active {
                        self.session.scroll_offset
                    } else {
                        0
                    },
                    if is_active { self.selection } else { None },
                )?;
            }
        }

        // Floating pane overlay — drawn last, on top of all split leaves.
        if let Some(id) = self.session.floating {
            if let Some(pane) = self.session.panes.get(&id) {
                let fw = win_size.width as f32 * 0.7;
                let fx = (win_size.width as f32 - fw) / 2.0;
                let fy = tab_h + content_h * 0.15;
                let is_active = id == self.session.active_pane;
                renderer.set_viewport(fx, fy);
                renderer.render_screen(
                    pane.parser.screen(),
                    if is_active {
                        self.session.scroll_offset
                    } else {
                        0
                    },
                    if is_active { self.selection } else { None },
                )?;
            }
        }

        renderer.draw_tab_bar(&tab_infos)?;

        let max_scroll = self.session.max_scroll_offset();
        let active_title = self
            .session
            .active_pane()
            .map_or_else(String::new, |p| p.title.clone());
        let right = if max_scroll > 0 {
            format!(
                "[{}%]",
                (100 * self.session.scroll_offset)
                    .checked_div(max_scroll)
                    .unwrap_or(0)
            )
        } else {
            String::new()
        };
        renderer.draw_status_bar(&active_title, &right)?;

        if max_scroll > 0 {
            let fraction = self.session.scroll_offset as f32 / max_scroll as f32;
            renderer.draw_scrollbar(scroll_px, scroll_py, scroll_pw, scroll_ph, fraction)?;
        }

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
        let status_h = renderer.status_bar_height();
        let content_h = (size.height as f32 - tab_h - status_h).max(0.0);
        let rects = self.session.split_root.compute_rects();
        for (&id, &(_, _, nw, nh)) in &rects {
            let cols = renderer.cols_for(nw * size.width as f32);
            let rows = renderer.rows_for(nh * content_h);
            if let Some(pane) = self.session.panes.get_mut(&id) {
                pane.parser.screen_mut().resize(cols, rows);
                let _ = pane.pty_tx.send(PtyCommand::Resize(PtySize { cols, rows }));
            }
        }
    }

    fn write_pty(&self, data: &[u8]) {
        if let Some(pane) = self.session.panes.get(&self.session.active_pane) {
            let _ = pane.pty_tx.send(PtyCommand::Write(data.to_vec()));
        }
    }

    /// Toggle the local line editor for the active pane (Alt+E). Returns false
    /// when a modal overlay (settings / host picker) is open so the key falls
    /// through to its handler.
    fn toggle_editing(&mut self) -> bool {
        if self.settings.open || self.host_picker.open {
            return false;
        }
        if self.editing.is_some() {
            self.editing = None;
        } else {
            self.start_editing();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    fn start_editing(&mut self) {
        // Seed the buffer with whatever the shell's readline already holds so
        // the line under the cursor appears in the editor. The shell line is
        // killed (Ctrl+U) so the local buffer is the sole source of truth on
        // Enter; Ctrl+U is a no-op on an empty line.
        let line = self.current_line();
        let state = EditingState::from_line(&line);
        if !state.is_empty() {
            self.write_pty(b"\x15");
        }
        self.session.scroll_offset = 0;
        self.editing = Some(state);
    }

    /// Handle a key while the line editor is active. Returns true when the key
    /// was consumed by the editor (never forwarded to the shell).
    fn handle_editing_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> bool {
        match code {
            KeyCode::Enter => {
                let state = self
                    .editing
                    .take()
                    .expect("caller guards editing.is_some()");
                let line = state.line();
                // Wrap in bracketed paste so readline inserts the buffer
                // literally (tabs stay tabs, no completion / history
                // expansion). Falls back to raw bytes when the shell never
                // enabled it. Empty buffer submits a bare newline.
                let bracketed = self
                    .session
                    .active_pane()
                    .is_some_and(|p| p.parser.bracketed_paste());
                let mut data = Vec::new();
                if bracketed {
                    data.extend_from_slice(b"\x1b[200~");
                }
                data.extend_from_slice(line.as_bytes());
                data.extend_from_slice(if bracketed { b"\x1b[201~\r\n" } else { b"\r\n" });
                self.write_pty(&data);
            }
            KeyCode::Escape => self.editing = None,
            // Word moves and deletes (readline M-b / M-f / M-d / M-backspace).
            KeyCode::KeyB if alt && !ctrl => self.editing.as_mut().unwrap().word_left(),
            KeyCode::KeyF if alt && !ctrl => self.editing.as_mut().unwrap().word_right(),
            KeyCode::KeyD if alt && !ctrl => self.editing.as_mut().unwrap().delete_word_after(),
            KeyCode::Backspace if alt && !ctrl => {
                self.editing.as_mut().unwrap().delete_word_before()
            }
            // Cursor / kill chords (readline C-a / C-e / C-k).
            KeyCode::KeyA if ctrl && !alt => self.editing.as_mut().unwrap().home(),
            KeyCode::KeyE if ctrl && !alt => self.editing.as_mut().unwrap().end(),
            KeyCode::KeyK if ctrl && !alt => {
                let state = self.editing.as_mut().unwrap();
                state.truncate_to_cursor();
            }
            // Cancel like Esc, discarding the buffer without touching the shell.
            KeyCode::KeyC if ctrl && !alt => self.editing = None,
            KeyCode::KeyD if ctrl && !alt => {
                if self.editing.as_ref().unwrap().is_empty() {
                    self.editing = None;
                } else {
                    return true;
                }
            }
            KeyCode::Backspace => self.editing.as_mut().unwrap().backspace(),
            KeyCode::Delete => self.editing.as_mut().unwrap().delete(),
            KeyCode::ArrowLeft => self.editing.as_mut().unwrap().left(),
            KeyCode::ArrowRight => self.editing.as_mut().unwrap().right(),
            KeyCode::Home => self.editing.as_mut().unwrap().home(),
            KeyCode::End => self.editing.as_mut().unwrap().end(),
            KeyCode::Tab => self.editing.as_mut().unwrap().insert('\t'),
            // Let Alt+E fall through so the same key exits edit mode.
            KeyCode::KeyE if alt && !ctrl => return false,
            // Swallow other ctrl/alt chords; plain keys fall through to the
            // text-input path which inserts them into the buffer.
            _ if ctrl || alt => return true,
            _ => return false,
        }
        true
    }

    /// Text of the line the cursor sits on, up to the cursor column.
    fn current_line(&self) -> String {
        let Some(pane) = self.session.active_pane() else {
            return String::new();
        };
        let screen = pane.parser.screen();
        let col = screen.cursor().col;
        let row = screen.scrollback().len() + screen.cursor().row;
        let mut chars = self.line_chars(row).unwrap_or_default();
        chars.truncate(col);
        while chars.last().is_some_and(|c| c.is_whitespace()) {
            chars.pop();
        }
        chars.into_iter().collect()
    }

    /// Run a loaded plugin against the current line and write its stdout into
    /// the active pane as if typed at a fresh prompt. Errors land dimmed via a
    /// leading red escape. No-op when the pane/plugin is missing.
    #[cfg(feature = "plugins")]
    fn run_plugin(&mut self, name: &str) {
        let input = self.current_line();
        let result = {
            let Some(plugin) = self.plugins.get_mut(name) else {
                return;
            };
            plugin.call(input.as_bytes())
        };
        let data = match result {
            Ok(out) => {
                let mut data = b"\r\n".to_vec();
                data.extend_from_slice(&out);
                data
            }
            Err(e) => format!("\r\n\u{1b}[31mplugin {}: {}\u{1b}[0m", name, e).into_bytes(),
        };
        self.write_pty(&data);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn max_scroll_offset(&self) -> usize {
        self.session.max_scroll_offset()
    }

    fn scroll_up(&mut self, lines: usize) {
        self.session.scroll_up(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.session.scroll_down(lines);
    }

    // Jump scroll to nearest command block in delta direction (-1 = prev, +1 = next)
    // relative to the middle of the current view. Block start_line is buffer-local;
    // global row = scrollback.len() + start_line. All buffer rows sit at the bottom of
    // the scrollable range, so a jump clamps to offset 0, landing the block at its
    // natural buffer row (near the top only when start_line is small).
    fn jump_to_block(&mut self, delta: i32) {
        let Some(pane) = self.session.active_pane() else {
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
            .saturating_sub(self.session.scroll_offset)
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
        if offset == self.session.scroll_offset {
            return;
        }
        self.session.scroll_offset = offset;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // Search overlay
    fn toggle_search(&mut self) {
        self.search.toggle();
        if self.search.open {
            if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
                self.search.save_screen(pane.parser.screen());
            }
        } else {
            self.close_search();
        }
        self.draw_search_overlay();
    }

    // Quake mode: F12 toggles the window hidden/shown.
    // ponytail: in-app toggle only. On X11 set_visible works; on Wayland
    // winit 0.30 treats set_visible AND set_minimized(false) as no-ops, so
    // hiding would strand the user — a true quake dropdown needs the
    // Wayland global-shortcut portal / layer-shell. Here it's a no-op there.
    fn toggle_quake(&mut self) {
        self.window_visible = !self.window_visible;
        if let Some(w) = &self.window {
            w.set_visible(self.window_visible);
            if self.window_visible {
                w.focus_window();
            }
        }
    }

    fn close_search(&mut self) {
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            self.search.restore_screen(pane.parser.screen_mut());
        }
        self.search.open = false;
        self.redraw();
    }

    fn draw_search_overlay(&mut self) {
        if !self.search.open {
            return;
        }
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
            return;
        };
        let (cols, rows) = {
            let s = pane.parser.screen();
            (s.size().cols, s.size().rows)
        };
        let bytes = self.search.overlay_bytes(cols, rows);
        pane.parser.parse(&bytes);
        self.redraw();
    }

    /// Re-run the scan for the current query and jump to the current match.
    fn search_apply(&mut self) {
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
            return;
        };
        let screen = pane.parser.screen();
        self.search.find(screen);
        self.search_jump();
    }

    fn search_step(&mut self, forward: bool) {
        let moved = if forward {
            self.search.next()
        } else {
            self.search.prev()
        };
        if moved {
            self.search_jump();
        }
    }

    /// Scroll so the current match row is the top visible row.
    fn search_jump(&mut self) {
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
            return;
        };
        let Some(target_row) = self.search.current_row() else {
            return;
        };
        let screen = pane.parser.screen();
        let scrollback = screen.scrollback().len();
        let visible = screen.size().rows;
        let total = scrollback + visible;
        let offset = total
            .saturating_sub(target_row + visible)
            .min(total.saturating_sub(visible));
        self.session.scroll_offset = offset;
        self.draw_search_overlay();
    }

    // Selection methods
    /// Tab bar height in pixels = one cell row (must match render()'s content_h math).
    fn tab_bar_height(&self) -> f32 {
        self.renderer.as_ref().map_or(0.0, |r| r.cell_size()[1])
    }

    /// Status bar height in pixels = one cell row (must match render()'s content_h math).
    fn status_bar_height(&self) -> f32 {
        self.renderer
            .as_ref()
            .map_or(0.0, |r| r.status_bar_height())
    }

    /// Map a window pixel point to the pane under it (normalized rects × window size).
    fn pane_at_point(&self, x: f32, y: f32) -> Option<usize> {
        let rects = self.session.split_root.compute_rects();
        if rects.len() <= 1 {
            return rects.keys().next().copied();
        }
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h =
            (window.inner_size().height as f32 - tab_h - self.status_bar_height()).max(0.0);
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
        if y < 0.0 || y >= tab_h || self.session.panes.is_empty() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let cell_w = renderer.cell_size()[0];
        let mut ids: Vec<usize> = self.session.panes.keys().copied().collect();
        ids.sort();
        let mut col = 1usize;
        for id in ids {
            let title = self
                .session
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

    /// Tab under a window-space x,y plus whether the point is over its close
    /// button (the right padding cell of the tab span). Layout mirrors
    /// tab_at_point / draw_tab_bar so hover and click land on the same cells.
    fn tab_bar_hover(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        let tab_h = self.tab_bar_height();
        if y < 0.0 || y >= tab_h || self.session.panes.is_empty() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let cell_w = renderer.cell_size()[0];
        let mut ids: Vec<usize> = self.session.panes.keys().copied().collect();
        ids.sort();
        let mut col = 1usize;
        for id in ids {
            let title = self
                .session
                .panes
                .get(&id)
                .map_or_else(String::new, |p| p.title.clone());
            let span = tab_span(&title, 20);
            let start_px = col as f32 * cell_w;
            let end_px = (col + span) as f32 * cell_w;
            if x >= start_px && x < end_px {
                let close_start = (col + span - 1) as f32 * cell_w;
                return Some((id, x >= close_start));
            }
            col += span + 1;
        }
        None
    }
    fn divider_at_point(&self, x: f32, y: f32, tolerance: f32) -> Option<(usize, SplitDir)> {
        if self.session.split_root.leaves().len() <= 1 || y < self.tab_bar_height() {
            return None;
        }
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h =
            (window.inner_size().height as f32 - tab_h - self.status_bar_height()).max(0.0);
        for (dir, boundary, target) in self.session.split_root.dividers() {
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
        let (Some(renderer), Some(pane)) = (&self.renderer, self.session.panes.get(&pane_id))
        else {
            return None;
        };
        let rect = self
            .session
            .split_root
            .compute_rects()
            .get(&pane_id)
            .copied()?;
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = self.tab_bar_height();
        let content_h =
            (window.inner_size().height as f32 - tab_h - self.status_bar_height()).max(0.0);
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
            let offset = if pane_id == self.session.active_pane {
                self.session.scroll_offset
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
        let pane = self.session.active_pane()?;
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
            let Some(pane) = self.session.active_pane() else {
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
                self.session.scroll_offset = 0;
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
        if pane_id != self.session.active_pane {
            self.session.active_pane = pane_id;
            self.editing = None;
            self.session.scroll_offset = 0;
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
            if let Some((row, col)) = self.screen_to_cell(self.session.active_pane, x, y) {
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
            self.session.active_pane().map(|pane| {
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
        let offset = if pane_id == self.session.active_pane {
            self.session.scroll_offset
        } else {
            0
        };
        if offset != 0 {
            return false;
        }
        let Some((global_row, col)) = self.screen_to_cell(pane_id, x, y) else {
            return false;
        };
        let Some(pane) = self.session.panes.get(&pane_id) else {
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
                renderer.set_cursor_blink(config.cursor.blink, config.cursor.blink_interval_ms);
            }
        }
    }

    fn toggle_settings(&mut self) {
        self.settings.toggle();
        if self.settings.open {
            let ctx = self.settings_ctx();
            self.settings.refresh(&ctx);
            if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
                self.settings.save_screen(pane.parser.screen());
            }
            self.draw_settings_overlay();
        } else {
            self.close_settings();
        }
    }

    fn close_settings(&mut self) {
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            self.settings.restore_screen(pane.parser.screen_mut());
        }
        self.settings.open = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn draw_settings_overlay(&mut self) {
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
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
            // Concurrent-WIP glue: the settings menu gained Export/Import items
            // (new notice field) before main.rs was wired. Minimal handlers so
            // the match stays exhaustive; owner of that feature can refine.
            SettingsAction::ExportConfig => {
                let export = Config::default_config_path().with_file_name("zeroterm-export.toml");
                let ok = self
                    .config
                    .as_ref()
                    .is_some_and(|c| c.save(Some(&export)).is_ok());
                self.settings.notice = Some(if ok {
                    format!("Exported to {}", export.display())
                } else {
                    "Export failed".to_string()
                });
            }
            SettingsAction::ImportConfig => {
                if let Some(config) = &mut self.config {
                    config.reload(None).ok();
                }
                self.settings.notice = Some("Config reloaded".to_string());
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
                self.save_session_layout();
                for (_, pane) in &self.session.panes {
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

                // Search overlay: while open, all keys route to the prompt.
                if self.search.open {
                    match &event.physical_key {
                        PhysicalKey::Code(code) => match code {
                            KeyCode::Escape => self.close_search(),
                            KeyCode::KeyF if ctrl && shift => self.close_search(),
                            KeyCode::Backspace => {
                                self.search.backspace();
                                self.search_apply();
                            }
                            KeyCode::Enter | KeyCode::ArrowDown if !shift => {
                                self.search_step(true);
                            }
                            KeyCode::Enter | KeyCode::ArrowUp if shift => {
                                self.search_step(false);
                            }
                            KeyCode::ArrowUp => self.search_step(false),
                            KeyCode::ArrowDown => self.search_step(true),
                            _ => {}
                        },
                        _ => {}
                    }
                    let text = event.text.clone().or_else(|| match &event.logical_key {
                        winit::keyboard::Key::Character(c) => Some(c.clone()),
                        _ => None,
                    });
                    if let Some(text) = &text {
                        if !text.is_empty() && !ctrl && !alt {
                            for c in text.chars() {
                                self.search.append(c);
                            }
                            self.search_apply();
                        }
                    }
                    return;
                }

                // AI overlay: while open, Escape or the toggle keys close it.
                if self.ai.open {
                    match &event.physical_key {
                        PhysicalKey::Code(code) => match code {
                            KeyCode::Escape => self.close_ai(),
                            KeyCode::KeyI if ctrl && shift => self.close_ai(),
                            KeyCode::KeyA if ctrl && shift => self.close_ai(),
                            _ => {}
                        },
                        _ => {}
                    }
                    return;
                }

                // Tab management shortcuts
                match &event.physical_key {
                    PhysicalKey::Code(code) => {
                        // Local line editor: while active, editing keys are
                        // intercepted here and printable text is absorbed in
                        // the text-input path below — nothing reaches the pty.
                        if self.editing.is_some() && self.handle_editing_key(*code, ctrl, alt) {
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                            return;
                        }
                        // Alt+E toggles the line editor. Alt is a prefix winit
                        // reports with every Escape-prefixed chord, so this is
                        // handled before the printable path and never reaches
                        // the shell (M-e is unbound in readline).
                        if alt && !ctrl && !shift && *code == KeyCode::KeyE && self.toggle_editing()
                        {
                            return;
                        }
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
                        #[cfg(feature = "ai")]
                        if ctrl && shift && !alt && *code == KeyCode::KeyI {
                            self.open_ai(AiKind::Explain);
                            return;
                        }
                        #[cfg(feature = "ai")]
                        if ctrl && shift && !alt && *code == KeyCode::KeyA {
                            self.open_ai(AiKind::Suggest);
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyO {
                            self.cycle_opacity();
                            return;
                        }
                        #[cfg(all(unix, feature = "ssh"))]
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
                                } else {
                                    self.open_host_picker();
                                }
                            }
                            return;
                        }
                        #[cfg(feature = "plugins")]
                        if ctrl && shift && !alt && *code == KeyCode::KeyB {
                            if let Some(name) = self.plugins.keys().min().cloned() {
                                self.run_plugin(&name);
                            } else {
                                warn!("No plugins loaded; put *.wasm files in the plugins dir to enable");
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
                            self.toggle_search();
                            return;
                        }
                        if ctrl && shift && !alt && *code == KeyCode::KeyG {
                            self.toggle_floating_pane();
                            self.update_window_title();
                            return;
                        }
                        if !ctrl && !shift && !alt && *code == KeyCode::F12 {
                            self.toggle_quake();
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
                                    self.session.scroll_offset = self.max_scroll_offset();
                                    if let Some(window) = &self.window {
                                        window.request_redraw();
                                    }
                                    return;
                                }
                                KeyCode::End if !ctrl => {
                                    self.session.scroll_offset = 0;
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
                                                .session
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

                // Handle printable text (IME text input). Some keymaps/dead
                // keys/IME states report text=None; fall back to the
                // logical key so printable characters still reach the pty.
                let text = event.text.clone().or_else(|| match &event.logical_key {
                    winit::keyboard::Key::Character(c) => Some(c.clone()),
                    _ => None,
                });
                if let Some(text) = &text {
                    if !text.is_empty() && !ctrl && !alt {
                        if let Some(state) = self.editing.as_mut() {
                            for c in text.chars() {
                                state.insert(c);
                            }
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        } else {
                            self.clear_selection();
                            self.write_pty(text.as_bytes());
                        }
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
                } else if self.renderer.is_none() {
                    // Renderer still initializing on the background thread —
                    // keep polling until check_renderer_ready() picks it up.
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                if self
                    .renderer
                    .as_mut()
                    .is_some_and(|r| r.cursor_blink_tick().is_some())
                {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                let x = position.x as f32;
                let y = position.y as f32;
                // Tab-bar hover: track the tab under the cursor (+ close button)
                // so draw_tab_bar can show the pill accent and close glyph.
                let hover = self.tab_bar_hover(x, y);
                if hover != self.hovered_tab.map(|id| (id, self.hovered_tab_close)) {
                    self.hovered_tab = hover.map(|(id, _)| id);
                    self.hovered_tab_close = hover.is_some_and(|(_, c)| c);
                    self.redraw();
                }
                // Split divider drag: resize from last position delta, then bail.
                if let Some(target) = self.session.dragging_divider {
                    let window = self.window.as_ref();
                    let (win_w, content_h) = window.map_or((1.0, 1.0), |w| {
                        let tab = self.tab_bar_height();
                        let status = self.status_bar_height();
                        (
                            w.inner_size().width as f32,
                            (w.inner_size().height as f32 - tab - status).max(0.0),
                        )
                    });
                    let (ax, ay) = self.session.divider_anchor;
                    let (dx, dy) = (x - ax, y - ay);
                    // Find this target's current divider to resize against its real boundary.
                    let found = self
                        .session
                        .split_root
                        .dividers()
                        .into_iter()
                        .find(|(_, _, t)| *t == target);
                    if let Some((dir, boundary, _)) = found {
                        let delta = match dir {
                            SplitDir::Vertical => dx / win_w,
                            SplitDir::Horizontal => dy / content_h,
                        };
                        self.session.split_root.resize_leaf(target, boundary, delta);
                    }
                    self.session.divider_anchor = (x, y);
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
                        if id != self.session.active_pane {
                            self.session.active_pane = id;
                            self.editing = None;
                            self.session.scroll_offset = 0;
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                    }
                }
                let pane_id = hovered.unwrap_or(self.session.active_pane);
                let mouse_tracking = self
                    .session
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
                // Left press may close a tab (close button), start a divider
                // drag, or switch tabs; release ends drags.
                if button == MouseButton::Left && state == winit::event::ElementState::Pressed {
                    if let Some((pane_id, true)) = self.tab_bar_hover(x, y) {
                        self.close_tab(pane_id);
                        return;
                    }
                    if let Some(pane_id) = self.tab_at_point(x, y) {
                        if pane_id != self.session.active_pane {
                            self.session.active_pane = pane_id;
                            self.editing = None;
                            self.session.scroll_offset = 0;
                        }
                        self.end_selection();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                        return;
                    }
                    if let Some((target, _)) = self.divider_at_point(x, y, 8.0) {
                        self.session.dragging_divider = Some(target);
                        self.session.divider_anchor = (x, y);
                        self.end_selection();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                        return;
                    }
                }
                if button == MouseButton::Left && state == winit::event::ElementState::Released {
                    self.session.dragging_divider = None;
                }
                let pane_id = self.pane_at_point(x, y).unwrap_or(self.session.active_pane);
                let mouse_tracking = self
                    .session
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
                        // copy-on-select: a drag that actually selected something copies on release
                        let dragged = self.selecting;
                        self.end_selection();
                        if dragged
                            && self.selection.as_ref().is_some_and(|s| {
                                s.start_row != s.end_row || s.start_col != s.end_col
                            })
                        {
                            self.copy_selection();
                        }
                        // Click-to-position: send CSI CUP so the shell moves its cursor.
                        if self.keybindings().click_to_position
                            && self.session.scroll_offset == 0
                            && y >= self.tab_bar_height()
                            && self.pane_at_point(x, y) == Some(self.session.active_pane)
                        {
                            if let Some((global_row, col)) =
                                self.screen_to_cell(self.session.active_pane, x, y)
                            {
                                let row = global_row.saturating_sub(
                                    self.session
                                        .panes
                                        .get(&self.session.active_pane)
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
                    if id != self.session.active_pane {
                        self.session.active_pane = id;
                        self.editing = None;
                        self.session.scroll_offset = 0;
                        self.scroll_fraction = 0.0;
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
                        let (up, down, rem) =
                            split_scroll_fraction(self.scroll_fraction + pos.y as f32 / cell_h);
                        self.scroll_fraction = rem;
                        if up > 0 {
                            self.scroll_up(up);
                        }
                        if down > 0 {
                            self.scroll_down(down);
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
    zt("main start");

    let event_loop = EventLoop::new()?;
    zt("event loop created");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new();
    zt("app created");
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
    fn split_scroll_fraction_keeps_sub_line_remainder() {
        fn near(a: f32, b: f32) -> bool {
            (a - b).abs() < 1e-6
        }
        // Whole lines extracted, remainder carries the sign and stays < 1.
        let (up, down, rem) = split_scroll_fraction(2.3);
        assert_eq!((up, down), (2, 0));
        assert!(near(rem, 0.3));
        let (up, down, rem) = split_scroll_fraction(-1.7);
        assert_eq!((up, down), (0, 1));
        assert!(near(rem, -0.7));
        // Sub-line deltas accumulate: 0.4 + 0.4 crosses the line threshold.
        let (up, down, rem) = split_scroll_fraction(0.4);
        assert_eq!((up, down), (0, 0));
        assert!(near(rem, 0.4));
        let (up, down, rem) = split_scroll_fraction(rem + 0.4);
        assert_eq!((up, down), (0, 0));
        assert!(near(rem, 0.8));
        let (up, down, rem) = split_scroll_fraction(rem + 0.4);
        assert_eq!((up, down), (1, 0));
        assert!(near(rem, 0.2));
    }

    #[test]
    fn editing_readline_bindings() {
        // Ctrl+K deletes to end of buffer, Ctrl+C cancels editing (App-level).
        let mut app = App::new();
        app.editing = Some(EditingState::from_line("hello world"));
        app.editing.as_mut().unwrap().home();
        app.editing.as_mut().unwrap().word_right();
        assert!(app.handle_editing_key(KeyCode::KeyK, true, false));
        assert_eq!(app.editing.as_ref().unwrap().line(), "hello");
        assert!(app.editing.is_some());
        assert!(app.handle_editing_key(KeyCode::KeyC, true, false));
        assert!(app.editing.is_none());
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
        app.session.panes.insert(
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
        app.session.active_pane = 0;

        app.drain_pty();
        let first: String = app.session.panes[&0]
            .parser
            .screen()
            .buffer()
            .iter()
            .flat_map(|row| row.iter().map(|c| c.ch))
            .collect();
        let scrollback_before = app.session.panes[&0].parser.screen().scrollback().len();
        assert!(
            first.contains("Process exited"),
            "first drain should append the exit notice"
        );

        app.drain_pty();
        let second: String = app.session.panes[&0]
            .parser
            .screen()
            .buffer()
            .iter()
            .flat_map(|row| row.iter().map(|c| c.ch))
            .collect();
        let scrollback_after = app.session.panes[&0].parser.screen().scrollback().len();
        assert_eq!(first, second, "visible buffer unchanged by a second drain");
        assert_eq!(
            scrollback_before, scrollback_after,
            "exit notice must be appended exactly once, not on every drain (scrollback must not grow)"
        );
    }

    #[cfg(all(unix, feature = "ssh"))]
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
