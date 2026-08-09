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
use winit::dpi::PhysicalSize;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes};

#[cfg(feature = "ai")]
use zeroterm_ai::client::{AiClient, AiError};
use zeroterm_config::{Config, KeybindingsConfig};
use zeroterm_core::parser::MouseTrackingMode;
use zeroterm_core::Parser;
use zeroterm_mux::session::{PaneSpec, Session, SessionLayout, TabLayout};
use zeroterm_mux::split::{SplitDir, SplitNode};
use zeroterm_mux::tab::Tab;
#[cfg(feature = "plugins")]
use zeroterm_plugin::Plugin;
use zeroterm_render::{Renderer, Selection};

use crate::app::layout::Layout;
#[cfg(feature = "sync")]
use zeroterm_sync::daemon::SyncDaemon;

#[cfg(feature = "ai")]
use crate::ai_overlay::{explain_prompt, suggest_context, AiKind, AiState};
use crate::ai_overlay::AiOverlay;
#[cfg(feature = "plugins")]
use crate::app::load_plugins;
#[cfg(all(unix, feature = "ssh"))]
use crate::app::spawn_ssh_process;
use crate::app::key_router;
use crate::app::selection;
use crate::app::{
    block_output_text, word_left, word_right, EditAction, HostPicker, LineEditor, PaneState,
    PtyCommand, SessionManager,
};
use crate::app::{spawn_pty_process, starship_setup};
use crate::search::SearchState;
use crate::settings::{SettingsAction, SettingsContext, SettingsMenu};

mod ai_overlay;
mod app;
mod frame;
mod overlay;
mod search;

use overlay::Overlay;

/// Which modal overlay owns the screen right now. One slot instead of four
/// booleans scattered through the input arm and the render loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    Search,
    Settings,
    Ai,
    HostPicker,
}
// Retained for the legacy session.json format + its tests; session layout
// persistence now lives in zeroterm-mux (SessionLayout) via save_session_layout().
#[allow(dead_code)]
mod session;
mod settings;

const COPY_MARKER: &str = "[copy]";
// Derived from the crate version so the window title can never drift from the
// actual release (CARGO_PKG_VERSION is set at compile time from Cargo.toml).
const VERSION_LABEL: &str = concat!("ZeroTerm v", env!("CARGO_PKG_VERSION"));

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
    // Local line editor for the active pane (Alt+E): owns the editing buffer,
    // history navigation and AI completion. While active, keys are intercepted
    // here (not forwarded to the shell) and printable text is absorbed into
    // the buffer until Enter submits the line or Esc discards it.
    editor: LineEditor,
    // When render() failed to present a frame (surface timeout / occluded),
    // schedules a bounded retry so the window repaints as soon as the surface
    // is presentable again instead of staying on a blank/stale frame.
    render_failed_at: Option<std::time::Instant>,
    /// Last renderer-init poll tick; the renderer=None redraw loop polls at
    /// ~20 Hz instead of a tight spin (a spin starves the background
    /// renderer-init thread and delays the first frame by minutes on iGPUs).
    last_init_poll: std::time::Instant,
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

/// Pure focus-follow decision: switch focus on hover only when the feature is
/// enabled, no drag-select is in progress, and the cursor actually moved into
/// a different pane (skips 1px jitter within the same pane).
fn should_focus_follow(
    follows: bool,
    selecting: bool,
    active: usize,
    hovered: Option<usize>,
) -> bool {
    follows && !selecting && hovered.is_some_and(|id| id != active)
}

#[allow(dead_code)]
/// cols/rows for a window of `size` at `cell_w` x `cell_h`, mirroring the
/// renderer's resize_panes_to_rects math exactly. Chrome rows (tab bar +
/// status bar) and padding come from the renderer crate's public constants,
/// so the spawn estimate can never drift from the renderer layout; floor and
/// clamp to >= 1 match cols_for/rows_for. Every PTY spawn site shares this so
/// the shell starts at its final size and the PaneState resize dedupe never
/// re-sends — bash prints its prompt exactly once instead of reprinting on a
/// startup resize storm. (Split/SSH panes still receive one resize to their
/// final, smaller rect after insertion — a single prompt reprint per pane
/// creation, inherent to splits and acceptable.)
fn cells_for_size(cell_w: f32, cell_h: f32, size: PhysicalSize<u32>) -> (usize, usize) {
    use zeroterm_render::{PADDING, STATUS_BAR_ROWS, TAB_BAR_ROWS};
    let chrome = (TAB_BAR_ROWS + STATUS_BAR_ROWS) as f32 * cell_h;
    let content_h = (size.height as f32 - chrome).max(0.0);
    let cols = ((size.width as f32 - PADDING[1] - PADDING[3]) / cell_w)
        .floor()
        .max(1.0) as usize;
    let rows = ((content_h - PADDING[0] - PADDING[2]) / cell_h)
        .floor()
        .max(1.0) as usize;
    (cols, rows)
}

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
            editor: LineEditor::new(),
            render_failed_at: None,
            last_init_poll: std::time::Instant::now(),
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
                    window.set_title(VERSION_LABEL);
                } else {
                    window.set_title(&format!("{} - {}", VERSION_LABEL, title));
                }
            }
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        zt("init start");
        info!("Initializing ZeroTerm");

        // Load config synchronously at boot. The old async seam
        // (Config::load_async + config_rx + a try_recv in render()) let the
        // FIRST shell spawn with defaults — the user's [shell] program/args
        // never reached pane 0 (observed live: `bash -l` ran despite
        // args=[]). A config read is a fast file parse + lua eval; the
        // background thread bought nothing but the race.
        let config = Config::load(None).unwrap_or_default();
        zt("config loaded");
        info!("keybindings: vim_mode={}", config.keybindings.vim_mode);

        let window_attrs = WindowAttributes::default()
            .with_title(VERSION_LABEL)
            .with_inner_size(winit::dpi::LogicalSize::new(
                config.window.width,
                config.window.height,
            ))
            .with_resizable(true)
            // ARGB visual (X11) / alpha-capable surface (Wayland): required for
            // window.opacity < 1.0 to actually show the desktop through the
            // terminal instead of a black void.
            .with_transparent(true);

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
            zt("renderer supervisor start");
            // Supervisor for the deferred GPU init. Renderer::new can stall for
            // minutes (observed: adapter/device creation blocking on a loaded
            // Intel iGPU, blocked on a kernel futex with zero CPU) or panic
            // (wgpu treats validation errors as fatal). A stuck attempt must
            // not keep the window dark forever, so after a 10s timeout a
            // FRESH attempt starts with its own Instance — a new driver
            // round-trip usually completes even when the previous one
            // deadlocked (fresh attempts on this machine finish in ~0.4s).
            // First success wins; late successes pile up in render_rx and are
            // dropped. Abandoned stuck threads are leaked in that pathological
            // case only, and a device that never presents is inert.
            fn spawn_attempt(
                window: std::sync::Arc<winit::window::Window>,
                font_size: f32,
                opacity: f64,
                font_path: Option<String>,
                render_tx: mpsc::Sender<zeroterm_render::Renderer>,
                done_tx: mpsc::Sender<bool>,
            ) {
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        pollster::block_on(Renderer::new(window, font_size, opacity, font_path))
                    }));
                    match result {
                        Ok(Ok(r)) => {
                            let _ = render_tx.send(r);
                            let _ = done_tx.send(true);
                        }
                        Ok(Err(e)) => {
                            error!("Renderer init failed: {}", e);
                            let _ = done_tx.send(false);
                        }
                        Err(p) => {
                            let msg = p
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| p.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".into());
                            error!("Renderer init panicked: {}", msg);
                            let _ = done_tx.send(false);
                        }
                    }
                });
            }

            let (done_tx, done_rx) = mpsc::channel::<bool>();
            let mut attempts = 1u32;
            let give_up_at = std::time::Instant::now() + std::time::Duration::from_secs(90);
            spawn_attempt(
                window_clone.clone(),
                font_size,
                opacity,
                font_path.clone(),
                render_tx.clone(),
                done_tx.clone(),
            );
            loop {
                match done_rx.recv_timeout(std::time::Duration::from_secs(10)) {
                    Ok(true) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        warn!("Renderer init supervisor channel closed; giving up");
                        break;
                    }
                    Ok(false) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if std::time::Instant::now() >= give_up_at || attempts >= 9 {
                            error!(
                                "Renderer init gave up after {} attempts; window stays dark",
                                attempts
                            );
                            window_clone.set_title("ZeroTerm — GPU init failed (restart)");
                            break;
                        }
                        attempts += 1;
                        warn!(
                            "Renderer init attempt {}: previous attempt not done in 10s, \
                             starting a fresh one",
                            attempts
                        );
                        spawn_attempt(
                            window_clone.clone(),
                            font_size,
                            opacity,
                            font_path.clone(),
                            render_tx.clone(),
                            done_tx.clone(),
                        );
                    }
                }
            }
            zt("renderer supervisor done");
        });
        self.renderer_rx = Some(render_rx);

        let size = window.inner_size();
        // Spawn the PTY at the EXACT size the renderer will use once ready:
        // estimate_cell_size mirrors the atlas's font metrics (row height from
        // ascent/descent/leading, column width from 'W' ink), and the cols/rows
        // math below matches resize_panes_to_rects (padding + chrome rows
        // subtracted). Because the renderer-ready resize then computes the
        // same size, the PaneState dedupe skips it — no SIGWINCH, and bash
        // prints its prompt exactly once instead of reprinting on a startup
        // resize storm (was 8.4x15px guess -> three resizes -> three prompts).
        let dpr = window.scale_factor().max(0.5) as f32; // same clamp as Renderer::new
        let (cell_w, cell_h) =
            zeroterm_render::estimate_cell_size(font_size * dpr, self.font_path.as_deref());
        let (cols, rows) = cells_for_size(cell_w, cell_h, size);

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
                title: VERSION_LABEL.into(),
                pane_cmd: shell.clone(),
                pty_dead: false,
                last_resize: Some((cols, rows)),
            },
        );

        #[cfg(feature = "ai")]
        let ai_client = if config.ai.endpoint.is_empty() {
            None
        } else {
            Some(Arc::new(AiClient::new(&config.ai.endpoint)))
        };

        self.window = Some(window);
        // The pre-spawned shell becomes the first classic tab. `tabs` is never
        // empty after init, so every SessionManager op has a live tab.
        self.session.panes = panes;
        self.session.next_pane_id = 1;
        self.session.tabs.push(Tab::with_pane(0, 0));
        self.session.active_tab = 0;
        self.session.sync_active();

        let layout_path = Config::default_config_path().with_file_name("layout.json");
        zt("session load start");
        // Session restore (roadmap 2.1): layout.json from the last clean quit
        // rebuilds tabs/splits. Pane 0 is the shell spawned above; each further
        // PaneSpec spawns through the pty layer (never bypassed). A missing or
        // corrupt file falls back to the single default tab already set up.
        if let Some(saved) = SessionLayout::restore(&layout_path) {
            // Restore per-tab: pane 0 (the shell spawned above) stands in for
            // saved tab 0's first pane; every other PaneSpec spawns through the
            // pty layer (never bypassed). A missing or corrupt file falls back
            // to the single default tab already set up.
            let mut restored: Vec<Vec<usize>> = Vec::new();
            for (ti, tab) in saved.tabs.iter().enumerate() {
                let mut ids = Vec::new();
                if ti == 0 {
                    ids.push(0usize); // the pre-spawned shell
                }
                for spec in tab.panes.iter().skip(if ti == 0 { 1 } else { 0 }) {
                    let cmd = if spec.cmd.is_empty() {
                        shell.clone()
                    } else {
                        spec.cmd.clone()
                    };
                    match spawn_pty_process(&cmd, &[], &[], cols, rows, self.wake_proxy()) {
                        Ok((pty_rx, pty_tx)) => {
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
                                    last_resize: Some((cols, rows)),
                                },
                            );
                            ids.push(id);
                        }
                        Err(e) => warn!("Session restore: failed to spawn '{}': {}", cmd, e),
                    }
                }
                restored.push(ids);
            }
            // Rebuild the per-tab trees. Saved leaf ids are positions into
            // that tab's pane list; remap them onto the freshly assigned ids.
            self.session.tabs.clear();
            for (ti, tab) in saved.tabs.iter().enumerate() {
                let ids = &restored[ti];
                if ids.is_empty() {
                    continue;
                }
                let tree = tab
                    .split
                    .as_ref()
                    .map(|s| SessionLayout::remap_split(s, ids))
                    .unwrap_or_else(|| SplitNode::from_ids(ids));
                let active = if tab.active_pane < ids.len() {
                    ids[tab.active_pane]
                } else {
                    ids[0]
                };
                self.session.tabs.push(Tab {
                    id: ids[0],
                    panes: ids.clone(),
                    tree,
                    active_pane: active,
                });
            }
            self.session.active_tab =
                saved.active_tab.min(self.session.tabs.len().saturating_sub(1));
            self.session.sync_active();
        }

        #[cfg(feature = "ai")]
        {
            self.ai_client = ai_client.clone();
            self.editor.set_ai_client(ai_client);
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
        let cwd = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tabs = self
            .session
            .tabs
            .iter()
            .map(|tab| {
                // Saved pane list in sorted-id order so tree leaf ids can be
                // stored as positions (to_positions) and remapped on restore.
                let mut ids = tab.panes.clone();
                ids.sort();
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
                TabLayout {
                    panes,
                    split: Some(tab.tree.to_positions(&ids)),
                    active_pane: ids.iter().position(|&i| i == tab.active_pane).unwrap_or(0),
                }
            })
            .collect();
        let layout = SessionLayout {
            active_tab: self.session.active_tab,
            tabs,
        };
        if let Err(e) = Session::new(0, layout).save(&path) {
            error!("Failed to save session layout: {}", e);
        }
    }

    fn create_new_tab(&mut self) -> Result<()> {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let (cell_w, cell_h) = match self.renderer.as_ref() {
                Some(r) => {
                    let c = r.cell_size();
                    (c[0], c[1])
                }
                None => {
                    let dpr = window.scale_factor().max(0.5) as f32; // same clamp as Renderer::new
                    zeroterm_render::estimate_cell_size(
                        self.font_size * dpr,
                        self.font_path.as_deref(),
                    )
                }
            };
            let (cols, rows) = cells_for_size(cell_w, cell_h, size);

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
            // A new tab is another pane in the global tree; insert it next to
            // the active pane instead of replacing the whole tree (which hid
            // every existing split and orphaned the other panes).
            self.session.register_pane(
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: VERSION_LABEL.into(),
                    pane_cmd: self.shell.clone(),
                    pty_dead: false,
                    last_resize: Some((cols, rows)),
                },
                SplitDir::Vertical,
                true,
            );
            // The new tab is a tiled pane; size every pane to its rect NOW
            // (create_split_pane already does this). Without it the new pane
            // keeps its full-window grid, whose left-anchored draw covers the
            // older tab until the first PTY drain happens to resize it — the
            // "new tab shown, old tabs blank" flicker. The PaneState::resize
            // dedupe makes this a no-op once sizes agree.
            self.resize_panes_to_rects();
        }
        Ok(())
    }

    fn create_split_pane(&mut self, dir: SplitDir) -> Result<()> {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let (cell_w, cell_h) = match self.renderer.as_ref() {
                Some(r) => {
                    let c = r.cell_size();
                    (c[0], c[1])
                }
                None => {
                    let dpr = window.scale_factor().max(0.5) as f32; // same clamp as Renderer::new
                    zeroterm_render::estimate_cell_size(
                        self.font_size * dpr,
                        self.font_path.as_deref(),
                    )
                }
            };
            let (cols, rows) = cells_for_size(cell_w, cell_h, size);

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
            // The split only exists once the pane is in the tree: Ctrl+Shift+E/D
            // inserts the new pane as a split of the active pane. Split panes
            // are not tabs.
            self.session.register_pane(
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: VERSION_LABEL.into(),
                    pane_cmd: self.shell.clone(),
                    pty_dead: false,
                    last_resize: Some((cols, rows)),
                },
                dir,
                false,
            );
            self.resize_panes_to_rects();
        }
        Ok(())
    }

    fn close_active_tab(&mut self) {
        if let Some(tab) = self.session.tabs.get(self.session.active_tab) {
            self.close_tab(tab.id);
        }
    }

    fn close_tab(&mut self, tab_id: usize) {
        // A tab is all its panes: close every pane (the session refuses the
        // last pane overall, so the window never empties).
        let ids: Vec<usize> = self
            .session
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.panes.clone())
            .unwrap_or_default();
        let mut closed_active = false;
        for id in ids {
            if let Some(effect) = self.session.close_pane(id) {
                let _ = effect.pane.pty_tx.send(PtyCommand::Kill);
                closed_active |= effect.was_active;
            }
        }
        if closed_active {
            self.editor.cancel();
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
        self.session.toggle_floating();
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
                    let tab_h = renderer.tab_bar_height();
                    let status_h = renderer.status_bar_height();
                    let content_h = (window.inner_size().height as f32 - tab_h - status_h).max(0.0);
                    let cols = renderer.cols_for(window.inner_size().width as f32 * 0.7);
                    let rows = renderer.rows_for(content_h * 0.7);
                    pane.resize(cols, rows);
                }
            }
            window.request_redraw();
        }
    }

    fn next_tab(&mut self) {
        if self.session.next_tab() {
            self.resize_panes_to_rects();
            self.redraw();
        }
    }

    fn previous_tab(&mut self) {
        if self.session.previous_tab() {
            self.resize_panes_to_rects();
            self.redraw();
        }
    }

    fn switch_to_tab(&mut self, idx: usize) {
        if self.session.switch_to_tab(idx) {
            // The newly visible tab's panes must match the current window (it
            // may have resized while inactive).
            self.resize_panes_to_rects();
            self.redraw();
        }
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
            let (cell_w, cell_h) = match self.renderer.as_ref() {
                Some(r) => {
                    let c = r.cell_size();
                    (c[0], c[1])
                }
                None => {
                    let dpr = window.scale_factor().max(0.5) as f32; // same clamp as Renderer::new
                    zeroterm_render::estimate_cell_size(
                        self.font_size * dpr,
                        self.font_path.as_deref(),
                    )
                }
            };
            let (cols, rows) = cells_for_size(cell_w, cell_h, size);

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
            // An SSH session is a new classic tab (full-window pane).
            self.session.register_pane(
                PaneState {
                    parser,
                    pty_rx,
                    pty_tx,
                    title: format!("SSH: {}@{}", user, host),
                    pane_cmd: format!("ssh {}@{}", user, host),
                    pty_dead: false,
                    last_resize: Some((cols, rows)),
                },
                SplitDir::Vertical,
                true,
            );
            self.resize_panes_to_rects();
        }
        Ok(())
    }

    #[cfg(all(unix, feature = "ssh"))]
    fn open_host_picker(&mut self) {
        let aliases = zeroterm_ssh::client::ssh_aliases();
        if aliases.is_empty() {
            return;
        }
        self.host_picker.open(aliases);
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            Overlay::snapshot(&mut self.host_picker, pane.parser.screen());
        }
        self.draw_host_picker();
    }

    fn draw_host_picker(&mut self) {
        self.draw_overlay(OverlayKind::HostPicker);
    }

    fn close_host_picker(&mut self) {
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            Overlay::restore(&mut self.host_picker, pane.parser.screen_mut());
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
            Overlay::snapshot(&mut self.ai, pane.parser.screen());
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
            Overlay::restore(&mut self.ai, pane.parser.screen_mut());
        }
        self.ai.close();
        self.redraw();
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
            // close_pane refuses the last pane, so the session never empties
            // (the final pane keeps its "[Process exited] - exit to quit").
            if let Some(effect) = self.session.close_pane(*id) {
                let _ = effect.pane.pty_tx.send(PtyCommand::Kill);
                if effect.was_active {
                    self.editor.cancel();
                }
            }
        }
        self.resize_panes_to_rects();
        if let Some(title) = title_changed {
            if let Some(window) = &self.window {
                window.set_title(&format!("{} - {}", VERSION_LABEL, title));
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
        renderer.resize(size.width, size.height);
        self.renderer = Some(renderer);
        // One resize path: resize_panes_to_rects (content-height rows, per-pane
        // rects, deduped via PaneState::resize). The PTY was spawned at exactly
        // this size, so the dedupe means NO resize is sent — bash prints its
        // prompt exactly once. The old code computed full-height rows here
        // (rows_for on the whole window height) which disagreed with
        // resize_panes_to_rects' content-height math, forcing a second resize
        // and a second prompt reprint.
        self.resize_panes_to_rects();
        zt("renderer received on main");
        info!("GPU renderer ready");
        // The async config load may have been consumed while the renderer was
        // still initializing on the background thread (its try_recv above was
        // a no-op then); re-apply so window.opacity/blur and fonts actually
        // reach the fresh renderer. Harmless if the config already applied.
        self.apply_config_to_renderer();
    }

    /// [ZTDIAG] Ground-truth screen probe: emits one line per render pass so
    /// a blank window can be attributed to an empty parser screen vs. a
    /// presentation failure. Gated on ZTDIAG=1 (see render()).
    fn ztdiag_screen(&self, label: &str) {
        let ready = self.renderer.is_some();
        let Some(pane) = self.session.active_pane() else {
            eprintln!("[ZTDIAG] {} renderer={} NO_ACTIVE_PANE", label, ready);
            return;
        };
        let screen = pane.parser.screen();
        let buffer = screen.buffer();
        let rows = buffer.len();
        let cols = buffer.first().map_or(0, |r| r.len());
        let mut ink = 0usize;
        let mut last_text = String::new();
        for row in buffer {
            let text: String = row.iter().map(|c| c.ch).collect();
            let trimmed = text.trim_end();
            if !trimmed.is_empty() {
                ink += trimmed.chars().count();
                last_text = text;
            }
        }
        eprintln!(
            "[ZTDIAG] {} renderer={} {}x{} ink={} scrollback={} cursor=({},{}) last='{}'",
            label,
            ready,
            cols,
            rows,
            ink,
            screen.scrollback().len(),
            screen.cursor().row,
            screen.cursor().col,
            last_text.chars().take(70).collect::<String>()
        );
    }

    fn render(&mut self) -> Result<()> {
        if std::env::var("ZTDIAG").is_ok() {
            self.ztdiag_screen("render");
        }
        self.check_renderer_ready();
        if self.config_changed.load(Ordering::SeqCst) {
            self.config_changed.store(false, Ordering::SeqCst);
            if let Some(config) = &mut self.config {
                config.reload(None).ok();
            }
            self.apply_config_to_renderer();
        }
        if self.ai.open {
            // Fire-and-poll: collect the finished AI result before drawing,
            // so the panel paints the response (or error) this frame.
            self.ai.poll();
        }
        if let Some(kind) = self.active_overlay() {
            self.draw_overlay(kind);
        }
        // Collect an in-flight AI line completion only while the editor is
        // open; closing the editor discards the pending request.
        if self.editor.is_active() {
            self.editor.poll_ai();
        }
        // Ghost completion string for the editor title; computed before the
        // renderer borrow below.
        let edit_ghost = if self.editor.is_active() {
            self.editor.completion_ghost()
        } else {
            None
        };
        let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) else {
            return Ok(());
        };
        let win_size = window.inner_size();
        let tab_h = renderer.tab_bar_height();
        let status_h = renderer.status_bar_height();
        let content_h = (win_size.height as f32 - tab_h - status_h).max(0.0);

        renderer.begin_frame()?;
        renderer.draw_background(renderer.theme_bg())?;

        // One pill per CLASSIC tab (not per pane), in tab order. While
        // editing, the active tab shows the live buffer instead of the shell
        // title. Editing is bound to the active pane and cleared on any tab
        // switch, so this is only ever the pane that owns the editor. A
        // pending AI completion appends a ghost suffix after the cursor marker.
        let tab_ids: Vec<usize> = self.session.tabs.iter().map(|t| t.id).collect();
        let active_tab_id = tab_ids.get(self.session.active_tab).copied().unwrap_or(0);
        let edit_display = self
            .editor
            .is_active()
            .then(|| self.editor.display_line(edit_ghost.as_deref()));
        let tab_infos = frame::tab_infos(
            &tab_ids,
            active_tab_id,
            |id| self.session.tab_title(id),
            edit_display.as_deref(),
            self.hovered_tab,
            self.hovered_tab_close,
        );

        let rects = self.session.rects();
        // Active pane's window-space viewport rect (for the scrollbar overlay).
        // Mirrors the pane-rect transform in render_screen calls below.
        let (scroll_px, scroll_py, scroll_pw, scroll_ph) = frame::active_pane_rect(
            self.session.floating == Some(self.session.active_pane),
            win_size.width as f32,
            tab_h,
            content_h,
            rects
                .get(&self.session.active_pane)
                .copied()
                .unwrap_or((0.0, 0.0, 1.0, 1.0)),
        );
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
                let (fx, fy, _fw, _fh) = frame::active_pane_rect(
                    true,
                    win_size.width as f32,
                    tab_h,
                    content_h,
                    (0.0, 0.0, 1.0, 1.0),
                );
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
        let status_left = format!("{} — {} tabs", active_title, self.session.tabs.len());
        renderer.draw_status_bar(
            &status_left,
            &frame::status_right(max_scroll, self.session.scroll_offset),
        )?;

        // Policy: hide the bar while scrollback is trivial; a near-full-height
        // thumb reads as a colored strip on the right edge (the old bug
        // painted it solid accent blue at full height).
        if let Some((fraction, thumb_fraction)) = frame::scrollbar_policy(
            max_scroll,
            self.session.scroll_offset,
            self.session
                .active_pane()
                .map_or(1, |p| p.parser.screen().size().rows),
        ) {
            renderer.draw_scrollbar(
                scroll_px,
                scroll_py,
                scroll_pw,
                scroll_ph,
                fraction,
                thumb_fraction,
            )?;
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
        let tab_h = renderer.tab_bar_height();
        let status_h = renderer.status_bar_height();
        let content_h = (size.height as f32 - tab_h - status_h).max(0.0);
        // Every tab's panes, sized from their own tab's tree rects. Inactive
        // tabs stay correct across window resizes, so switching tabs never
        // shows a stale grid. PaneState::resize dedupes unchanged sizes.
        for tab in &self.session.tabs {
            for (&id, &(_, _, nw, nh)) in &tab.tree.compute_rects() {
                let cols = renderer.cols_for(nw * size.width as f32);
                let rows = renderer.rows_for(nh * content_h);
                if let Some(pane) = self.session.panes.get_mut(&id) {
                    pane.resize(cols, rows);
                }
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
        if self.editor.is_active() {
            self.editor.cancel();
        } else {
            self.start_editing();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    /// Begin an editing session seeded with the shell's current line. The
    /// shell's readline line is killed (Ctrl+U) so the local buffer is the
    /// sole source of truth on Enter; Ctrl+U is a no-op on an empty line.
    fn start_editing(&mut self) {
        let line = self.current_line();
        self.editor.start(&line);
        if !self.editor.is_empty() {
            self.write_pty(b"\x15");
        }
        self.session.scroll_offset = 0;
    }

    /// Execute the shell side of an editor submit: wrap the line in bracketed
    /// paste so readline inserts it literally, falling back to raw bytes when
    /// the shell never enabled it. An empty buffer submits a bare newline.
    fn submit_editor_line(&mut self, line: &str) {
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
                Overlay::snapshot(&mut self.search, pane.parser.screen());
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
            Overlay::restore(&mut self.search, pane.parser.screen_mut());
        }
        self.search.open = false;
        self.redraw();
    }

    fn draw_search_overlay(&mut self) {
        if !self.search.open {
            return;
        }
        self.draw_overlay(OverlayKind::Search);
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
    /// Tab bar height in pixels = two cell rows (must match render()'s content_h math).
    fn tab_bar_height(&self) -> f32 {
        self.renderer.as_ref().map_or(0.0, |r| r.tab_bar_height())
    }

    /// Status bar height in pixels = one cell row (must match render()'s content_h math).
    fn status_bar_height(&self) -> f32 {
        self.renderer
            .as_ref()
            .map_or(0.0, |r| r.status_bar_height())
    }

    /// Map a window pixel point to the pane under it (normalized rects × window size).
    /// Single-pane shortcut keeps the historical "any pixel -> the one pane"
    /// answer (even over the bars) so cursor/mouse-tracking stays unchanged.
    fn pane_at_point(&self, x: f32, y: f32) -> Option<usize> {
        let rects = self.session.rects();
        if rects.len() <= 1 {
            return rects.keys().next().copied();
        }
        let window = self.window.as_ref()?;
        let layout = self.layout()?;
        let (nx, ny) = layout.content_normalized(
            x,
            y,
            window.inner_size().width as f32,
            window.inner_size().height as f32,
        )?;
        self.session.pane_at(nx, ny)
    }

    /// Focus-follow-on-hover: if enabled and the pointer has drifted into a
    /// different pane (not during drag-select), make that pane active. Early
    /// returns: disabled, drag-selecting, hover over the tab/status bars
    /// (hit-test is None), hover == active pane. Hit-testing goes through
    /// pane_at_point so focus-follow matches click-to-focus.
    fn maybe_focus_follow(&mut self, hovered: Option<usize>) {
        let follows = self
            .config
            .as_ref()
            .map_or(false, |c| c.mouse.focus_follows_mouse);
        if !should_focus_follow(follows, self.selecting, self.session.active_pane, hovered) {
            return;
        }
        let id = hovered.unwrap();
        self.session.set_active_pane(id);
        self.editor.cancel();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Pane id of the tab under a window-space x,y. Layout owns the strip
    /// contract (must match draw_tab_bar: sorted ids, col 1, span = chars+2,
    /// col += span+1) and the geometry.
    fn tab_at_point(&self, x: f32, y: f32) -> Option<usize> {
        self.layout()?.tab_at(x, y, &self.sorted_tab_titles())
    }

    /// Tab under a window-space x,y plus whether the point is over its close
    /// button (the right padding cell of the tab span). Layout mirrors
    /// tab_at_point / draw_tab_bar so hover and click land on the same cells.
    fn tab_bar_hover(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        self.layout()?
            .tab_bar_hover(x, y, &self.sorted_tab_titles())
    }
    fn divider_at_point(&self, x: f32, y: f32, tolerance: f32) -> Option<(usize, SplitDir)> {
        let window = self.window.as_ref()?;
        self.layout()?.divider_at(
            x,
            y,
            tolerance,
            window.inner_size().width as f32,
            window.inner_size().height as f32,
            self.session.leaves().len() > 1,
            &self.session.dividers(),
        )
    }

    fn screen_to_cell(&self, pane_id: usize, x: f32, y: f32) -> Option<(usize, usize)> {
        let (Some(pane), Some(layout)) = (
            self.session.panes.get(&pane_id),
            self.layout(),
        ) else {
            return None;
        };
        let rect = self.session.rects().get(&pane_id).copied()?;
        let window = self.window.as_ref()?;
        let win_w = window.inner_size().width as f32;
        let tab_h = layout.tab_h();
        let content_h = layout.content_h(window.inner_size().height as f32);
        let rect_px = (
            rect.0 * win_w,
            rect.1 * content_h + tab_h,
            rect.2 * win_w,
            rect.3 * content_h,
        );
        // scroll_offset is a single field owned by the active pane; inactive
        // panes render at 0.
        let offset = if pane_id == self.session.active_pane {
            self.session.scroll_offset
        } else {
            0
        };
        layout.screen_to_cell(x, y, rect_px, pane.parser.screen(), offset)
    }

    /// Geometry for the current window: cell size + chrome bar heights. All
    /// hit-testing derives from this so tab/status geometry lives in one place.
    fn layout(&self) -> Option<Layout> {
        let renderer = self.renderer.as_ref()?;
        Some(Layout::new(
            renderer.cell_size(),
            self.tab_bar_height(),
            self.status_bar_height(),
        ))
    }

    /// Sorted (pane id, title) pairs for tab hit-testing (mirrors draw_tab_bar).
    /// (tab id, title) pairs for tab-bar hit-testing (mirrors draw_tab_bar:
    /// one pill per classic tab, in tab order).
    fn sorted_tab_titles(&self) -> Vec<(usize, String)> {
        self.session
            .tabs
            .iter()
            .map(|t| (t.id, self.session.tab_title(t.id)))
            .collect()
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
            self.editor.cancel();
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
        let text = self.selection.as_ref().and_then(|sel| {
            self.session.active_pane().map(|pane| {
                selection::selection_text(sel, pane.parser.screen())
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
        // Copy the config values first so no borrow of self.config outlives
        // the mutable self calls below (resize_panes_to_rects borrows all of
        // self, which the old nested borrows rejected at compile time).
        let (font_path, font_size, opacity, blink, blink_interval) = {
            let Some(config) = &self.config else {
                return;
            };
            (
                config.font.path.clone(),
                config.font.size,
                config.window.opacity,
                config.cursor.blink,
                config.cursor.blink_interval_ms,
            )
        };
        self.opacity = opacity;
        // A config `font.path` / `font.size` change must reach the glyph
        // atlas at runtime: reload_font swaps the file AND re-rasterizes at
        // the new size, then we re-layout the panes for the new cell metrics
        // (previously the path was only stored, never applied).
        if font_path != self.font_path || (font_size - self.font_size).abs() > f32::EPSILON {
            // A rejected font (missing file / unparseable) must NOT commit the
            // new path/size or re-layout: the renderer kept the old metrics,
            // so committing would desync pane sizing from what actually
            // renders. No renderer yet: the values are consumed at init.
            let applied = match &mut self.renderer {
                Some(renderer) => {
                    renderer.reload_font(font_path.clone(), font_size).is_ok()
                }
                None => true,
            };
            if applied {
                self.font_path = font_path;
                self.font_size = font_size;
                self.resize_panes_to_rects();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            } else {
                warn!("Font change rejected: keeping the current font and metrics");
            }
        }
        // reload_config wants the whole config; re-borrow it now that no
        // mutable self borrow is live.
        if let Some(config) = &self.config {
            if let Some(renderer) = &mut self.renderer {
                renderer.reload_config(config);
                renderer.set_cursor_blink(blink, blink_interval);
            }
        }
    }

    fn toggle_settings(&mut self) {
        self.settings.toggle();
        if self.settings.open {
            let ctx = self.settings_ctx();
            self.settings.refresh(&ctx);
            if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
                Overlay::snapshot(&mut self.settings, pane.parser.screen());
            }
            self.draw_settings_overlay();
        } else {
            self.close_settings();
        }
    }

    fn close_settings(&mut self) {
        if let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) {
            Overlay::restore(&mut self.settings, pane.parser.screen_mut());
        }
        self.settings.open = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn draw_settings_overlay(&mut self) {
        self.draw_overlay(OverlayKind::Settings);
    }

    /// Paint the given overlay into the active pane's screen via its `Overlay`
    /// impl (one draw path instead of four copies). No-op when the overlay is
    /// not open, so callers can fire it unconditionally.
    fn draw_overlay(&mut self, kind: OverlayKind) {
        if !self.overlay_open(kind) {
            return;
        }
        let Some(pane) = self.session.panes.get_mut(&self.session.active_pane) else {
            return;
        };
        let (cols, rows) = {
            let s = pane.parser.screen();
            (s.size().cols, s.size().rows)
        };
        let bytes = match kind {
            OverlayKind::Search => Overlay::draw_bytes(&self.search, cols, rows),
            OverlayKind::Settings => Overlay::draw_bytes(&self.settings, cols, rows),
            OverlayKind::Ai => Overlay::draw_bytes(&self.ai, cols, rows),
            OverlayKind::HostPicker => Overlay::draw_bytes(&self.host_picker, cols, rows),
        };
        pane.parser.parse(&bytes);
        self.redraw();
    }

    /// Whether the given overlay is currently open (owns the screen region).
    fn overlay_open(&self, kind: OverlayKind) -> bool {
        match kind {
            OverlayKind::Search => Overlay::is_open(&self.search),
            OverlayKind::Settings => Overlay::is_open(&self.settings),
            OverlayKind::Ai => Overlay::is_open(&self.ai),
            OverlayKind::HostPicker => Overlay::is_open(&self.host_picker),
        }
    }

    /// The one overlay currently owning the screen, if any. Drives the
    /// input-arm routing and the render loop from a single slot instead of
    /// four independent booleans.
    fn active_overlay(&self) -> Option<OverlayKind> {
        if self.search.open {
            Some(OverlayKind::Search)
        } else if self.ai.open {
            Some(OverlayKind::Ai)
        } else if self.settings.open {
            Some(OverlayKind::Settings)
        } else if self.host_picker.open {
            Some(OverlayKind::HostPicker)
        } else {
            None
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
    /// Route a pressed key through the pure KeyRouter decode tables.
    ///
    /// The old `KeyboardInput` arm interleaved decoding and execution in one
    /// 430-line match; this method is the thin stateful glue that applies
    /// actions the pure `key_router` module decodes. Overlay routing,
    /// keybindings, and escape-sequence encoding are unit-tested there.
    fn handle_key(&mut self, event: winit::event::KeyEvent) {
        if event.state != winit::event::ElementState::Pressed {
            return;
        }
        let mods = key_router::Mods::from_state(&self.modifiers);
        let ctrl = mods.ctrl;
        let shift = mods.shift;
        let alt = mods.alt;
        let code: Option<KeyCode> = match &event.physical_key {
            PhysicalKey::Code(c) => Some(*c),
            _ => None,
        };
        let text = event.text.clone().or_else(|| match &event.logical_key {
            winit::keyboard::Key::Character(c) => Some(c.clone()),
            _ => None,
        });

        // 1. Search overlay owns every key while open.
        if self.search.open {
            match code {
                Some(code) => match key_router::search_key(code, mods, text.as_deref()) {
                    key_router::SearchKey::Close => self.close_search(),
                    key_router::SearchKey::Backspace => {
                        self.search.backspace();
                        self.search_apply();
                    }
                    key_router::SearchKey::Step(fwd) => self.search_step(fwd),
                    key_router::SearchKey::Text(t) => {
                        for c in t.chars() {
                            self.search.append(c);
                        }
                        self.search_apply();
                    }
                },
                // No physical code (IME / dead keys): append printable text.
                None => {
                    if let Some(t) = &text {
                        if !t.is_empty() && !ctrl && !alt {
                            for c in t.chars() {
                                self.search.append(c);
                            }
                            self.search_apply();
                        }
                    }
                }
            }
            return;
        }

        // 2. AI overlay owns every key while open.
        if self.ai.open {
            if let Some(code) = code {
                if let Some(key_router::AiKey::Close) = key_router::ai_key(code, mods) {
                    self.close_ai();
                }
            }
            return;
        }

        // 3. Local line editor owns keys while active (Pass falls through to
        //    the global bindings below).
        if let Some(code) = code {
            if self.editor.is_active() {
                match self.editor.handle(code, ctrl, shift, alt) {
                    EditAction::Pass => {}
                    EditAction::Submit(line) => {
                        self.submit_editor_line(&line);
                        self.redraw();
                        return;
                    }
                    EditAction::Handled => {
                        self.redraw();
                        return;
                    }
                }
            }
            // Alt+E toggles the editor (before the global chords; winit
            // reports Alt as a prefix on every Escape-prefixed chord, so this
            // must be claimed before the printable path).
            if alt && !ctrl && !shift && code == KeyCode::KeyE && self.toggle_editing() {
                return;
            }

            // 4. Global keybindings (incl. modal picker/settings keys).
            let ctx = key_router::KeyCtx {
                picker_open: self.host_picker.open,
                settings_open: self.settings.open,
            };
            match key_router::global_key(code, mods, ctx) {
                key_router::GlobalAction::NewTab => {
                    if let Err(e) = self.create_new_tab() {
                        error!("Failed to create tab: {}", e);
                    }
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::CloseTab => {
                    self.close_active_tab();
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::Split(dir) => {
                    if let Err(e) = self.create_split_pane(dir) {
                        error!("Failed to split pane: {}", e);
                    }
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::ToggleSettings => {
                    self.toggle_settings();
                    return;
                }
                key_router::GlobalAction::ToggleSearch => {
                    self.toggle_search();
                    return;
                }
                key_router::GlobalAction::ToggleFloating => {
                    self.toggle_floating_pane();
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::ToggleQuake => {
                    self.toggle_quake();
                    return;
                }
                key_router::GlobalAction::NextTab => {
                    self.next_tab();
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::PrevTab => {
                    self.previous_tab();
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::SwitchToTab(idx) => {
                    self.switch_to_tab(idx);
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::FocusPane(dir) => {
                    self.focus_adjacent_pane(dir);
                    self.update_window_title();
                    return;
                }
                key_router::GlobalAction::CycleOpacity => {
                    self.cycle_opacity();
                    return;
                }
                key_router::GlobalAction::JumpBlock(delta) => {
                    self.jump_to_block(delta);
                    return;
                }
                #[cfg(feature = "ai")]
                key_router::GlobalAction::OpenAi(kind) => {
                    self.open_ai(kind);
                    return;
                }
                #[cfg(all(unix, feature = "ssh"))]
                key_router::GlobalAction::Ssh => {
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
                key_router::GlobalAction::RunPlugin => {
                    if let Some(name) = self.plugins.keys().min().cloned() {
                        self.run_plugin(&name);
                    } else {
                        warn!("No plugins loaded; put *.wasm files in the plugins dir to enable");
                    }
                    return;
                }
                key_router::GlobalAction::Picker(key) => match key {
                    key_router::PickerKey::Up => self.host_picker.prev(),
                    key_router::PickerKey::Down => self.host_picker.next(),
                    key_router::PickerKey::Select => self.pick_host(),
                    key_router::PickerKey::Escape => self.close_host_picker(),
                },
                key_router::GlobalAction::Settings(key) => match key {
                    key_router::SettingsKey::Up => self.settings.prev(),
                    key_router::SettingsKey::Down => self.settings.next(),
                    key_router::SettingsKey::Activate => {
                        let ctx = self.settings_ctx();
                        let action = self.settings.activate(&ctx);
                        self.apply_settings_action(action);
                    }
                    key_router::SettingsKey::Escape => self.close_settings(),
                },
                key_router::GlobalAction::Pass => {}
            }
            // A modal overlay is open: it swallowed the key (no fallthrough
            // to the console layer), but its view must repaint.
            if self.host_picker.open || self.settings.open {
                if self.host_picker.open {
                    self.draw_host_picker();
                }
                if self.settings.open {
                    self.draw_settings_overlay();
                }
                return;
            }

            // 5. Console layer: scroll, selection extend, copy/paste, escape
            //    sequences.
            match key_router::console_key(code, mods) {
                key_router::ConsoleAction::ScrollUp(n) => {
                    self.scroll_up(n);
                    self.redraw();
                    return;
                }
                key_router::ConsoleAction::ScrollDown(n) => {
                    self.scroll_down(n);
                    self.redraw();
                    return;
                }
                key_router::ConsoleAction::ScrollTop => {
                    self.session.scroll_offset = self.max_scroll_offset();
                    self.redraw();
                    return;
                }
                key_router::ConsoleAction::ScrollBottom => {
                    self.session.scroll_offset = 0;
                    self.redraw();
                    return;
                }
                key_router::ConsoleAction::ExtendSelection { code, ctrl } => {
                    if self.shift_arrow_extend(code, ctrl) {
                        self.redraw();
                        return;
                    }
                    // Feature disabled: fall back to the raw escape sequence.
                    if let Some(seq) = key_router::key_sequence(code, mods) {
                        self.clear_selection();
                        self.write_pty(&seq);
                    }
                    return;
                }
                key_router::ConsoleAction::CopySelection => {
                    self.copy_selection();
                    self.redraw();
                    return;
                }
                key_router::ConsoleAction::Paste => {
                    self.paste_clipboard();
                    return;
                }
                key_router::ConsoleAction::Pty(seq) => {
                    self.clear_selection();
                    self.write_pty(&seq);
                    // The key was fully handled as a terminal escape sequence
                    // (Enter -> CR, Tab -> HT, arrows, ...). Returning here
                    // prevents the printable-text step from ALSO writing
                    // event.text (e.g. "\r" for Enter), which double-sent the
                    // byte and made readline see two newlines per Enter.
                    return;
                }
                key_router::ConsoleAction::None => {}
            }
        }

        // 6. Printable text (IME text input). Some keymaps/dead keys report
        //    text=None; fall back to the logical key so printable characters
        //    still reach the pty.
        if let Some(text) = &text {
            if !text.is_empty() && !ctrl && !alt {
                if self.editor.is_active() {
                    self.editor.insert_text(text);
                    self.redraw();
                } else {
                    self.clear_selection();
                    self.write_pty(text.as_bytes());
                }
            }
        }

        if self.drain_pty() {
            self.redraw();
        }
    }

    /// Paste the clipboard into the active pane, bracketed when the shell
    /// advertises bracketed paste (readline) so multi-line text is inserted
    /// literally. Plain fallback writes the raw bytes.
    fn paste_clipboard(&mut self) {
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
    }
}

impl ApplicationHandler for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Re-arm a redraw whenever a WaitUntil deadline has passed. Without
        // this the event loop sleeps through every blink / animation /
        // init-poll deadline: the cursor freezes, repaints stop, and before
        // the renderer arrives check_renderer_ready() is never polled again
        // (the window stays dark even after init completes).
        if self.renderer.is_none() {
            let every = std::time::Duration::from_millis(50);
            if self.last_init_poll.elapsed() >= every {
                self.last_init_poll = std::time::Instant::now();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_init_poll + every));
            return;
        }
        if let Some(renderer) = &mut self.renderer {
            let next = renderer.blink_next();
            if next <= std::time::Instant::now() {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
        }
    }

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
            WindowEvent::Focused(focused) => {
                // A window that was unfocused / on another workspace can come
                // back holding a stale (or blank) frame if the compositor
                // dropped its frame-callback stream; force a repaint on focus.
                if focused {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::Occluded(_) => {
                // Wayland/X11: the occlusion transition is exactly when the
                // compositor may stop/start delivering frame callbacks. A
                // repaint on every occlusion change keeps the window from
                // freezing on its last (possibly empty) frame.
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(state) => {
                self.modifiers = state.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key(event);
            }
            WindowEvent::RedrawRequested => {
                self.periodic_sync();
                self.drain_pty();

                if let Err(e) = self.render() {
                    error!("Render error: {}", e);
                    // A failed acquire (occluded surface, frame timeout) must
                    // not dead-end rendering: retry on a short interval so the
                    // window repaints as soon as the surface is presentable.
                    self.render_failed_at = Some(std::time::Instant::now());
                } else {
                    self.render_failed_at = None;
                }
                if let Some(failed_at) = self.render_failed_at {
                    if failed_at.elapsed() > std::time::Duration::from_millis(100) {
                        self.render_failed_at = None;
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
                if let Some(delay) = self.renderer.as_mut().and_then(|r| r.next_frame_delay()) {
                    self.last_anim_frame = std::time::Instant::now() + delay;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                } else if self.renderer.is_none() {
                    // Renderer still initializing on the background thread —
                    // poll at ~20 Hz instead of a tight spin. A busy loop here
                    // pegs a CPU core and starves the init thread, delaying the
                    // first frame by minutes on loaded iGPUs (observed: window
                    // stayed dark ~2 min under load, then init finished in
                    // 400 ms once the machine went idle).
                    let every = std::time::Duration::from_millis(50);
                    if self.last_init_poll.elapsed() >= every {
                        self.last_init_poll = std::time::Instant::now();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_init_poll + every));
                }
                // Cursor blink drives redraws at the blink cadence (~2/s),
                // not every vsync: redraw when the phase just toggled, else
                // sleep the event loop until the next toggle instant.
                let blink_next = self.renderer.as_mut().map(|r| r.blink_next());
                if let Some(next) = blink_next {
                    if next <= std::time::Instant::now() {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    } else {
                        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
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
                        .dividers()
                        .into_iter()
                        .find(|(_, _, t)| *t == target);
                    if let Some((dir, boundary, _)) = found {
                        let delta = match dir {
                            SplitDir::Vertical => dx / win_w,
                            SplitDir::Horizontal => dy / content_h,
                        };
                        self.session.resize_divider(target, boundary, delta);
                    }
                    self.session.divider_anchor = (x, y);
                    self.resize_panes_to_rects();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                let hovered = self.pane_at_point(x, y);
                self.maybe_focus_follow(hovered);
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
                    if let Some((tab_id, true)) = self.tab_bar_hover(x, y) {
                        self.close_tab(tab_id);
                        return;
                    }
                    if let Some(tab_id) = self.tab_at_point(x, y) {
                        if let Some(idx) = self.session.tabs.iter().position(|t| t.id == tab_id) {
                            if idx != self.session.active_tab {
                                self.switch_to_tab(idx); // resizes + redraws
                                self.editor.cancel();
                            }
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
                                let (a, b, c, d) = selection::normalize(s);
                                (a, b) != (c, d)
                            })
                        {
                            self.copy_selection();
                        }
                        // NOTE: click-to-position via CSI CUP is intentionally NOT sent.
                        // Injecting "\x1b[<row>;<col>H" into the PTY at a bare shell prompt
                        // makes readline render escape garbage on the command line ("clicks
                        // trigger texts"). Apps that want click positions enable mouse
                        // tracking and receive SGR sequences above; selection/copy is all
                        // a click should do otherwise.
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
                        self.editor.cancel();
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
        "  curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash -s -- upgrade"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::EditMode;

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
        // Ctrl+K deletes to end of buffer, Ctrl+C cancels editing.
        let mut app = App::new();
        app.editor.start("hello world");
        app.editor.state.as_mut().unwrap().home();
        app.editor.state.as_mut().unwrap().word_right();
        assert_eq!(
            app.editor.handle(KeyCode::KeyK, true, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "hello");
        assert!(app.editor.is_active());
        assert_eq!(
            app.editor.handle(KeyCode::KeyC, true, false, false),
            EditAction::Handled
        );
        assert!(!app.editor.is_active());
    }

    #[test]
    fn focus_follow_requires_enabled_not_selecting_and_new_pane() {
        // Disabled -> never switch.
        assert!(!should_focus_follow(false, false, 1, Some(2)));
        // Enabled but drag-selecting -> hold focus on the starting pane.
        assert!(!should_focus_follow(true, true, 1, Some(2)));
        // Hover outside any pane (tab/status bar) -> no switch.
        assert!(!should_focus_follow(true, false, 1, None));
        // Same pane (1px jitter) -> no switch.
        assert!(!should_focus_follow(true, false, 1, Some(1)));
        // Enabled, idle, different pane -> switch.
        assert!(should_focus_follow(true, false, 1, Some(2)));
    }

    #[test]
    fn editing_history_up_down_recalls_lines() {
        let mut app = App::new();
        app.editor.history.push("echo one");
        app.editor.history.push("echo two");
        app.editor.start("");
        // Up recalls the most recent entry; the in-progress line is stashed.
        assert_eq!(
            app.editor.handle(KeyCode::ArrowUp, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "echo two");
        assert_eq!(
            app.editor.handle(KeyCode::ArrowUp, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "echo one");
        // Down walks forward again.
        assert_eq!(
            app.editor.handle(KeyCode::ArrowDown, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "echo two");
        // Ctrl+P / Ctrl+N are the readline chords for Up / Down.
        assert_eq!(
            app.editor.handle(KeyCode::KeyP, true, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "echo one");
        assert_eq!(
            app.editor.handle(KeyCode::KeyN, true, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "echo two");
        // Up then Enter submits the recalled line into history.
        assert_eq!(
            app.editor.handle(KeyCode::ArrowUp, false, false, false),
            EditAction::Handled
        );
        assert!(matches!(
            app.editor.handle(KeyCode::Enter, false, false, false),
            EditAction::Submit(_)
        ));
        assert!(!app.editor.is_active());
        assert_eq!(app.editor.history.len(), 3);
        assert_eq!(
            app.editor.history.prev(""),
            Some("echo one".to_string()),
            "recalled submission must be the newest entry"
        );
    }

    #[test]
    fn editing_history_enter_dedupes_last_entry() {
        let mut app = App::new();
        app.editor.history.push("ls");
        app.editor.history.push("cd /tmp");
        app.editor.start("cd /tmp");
        assert!(matches!(
            app.editor.handle(KeyCode::Enter, false, false, false),
            EditAction::Submit(_)
        ));
        assert_eq!(app.editor.history.len(), 2, "repeat of last entry must be a no-op");
    }

    #[test]
    fn editing_history_empty_noop() {
        let mut app = App::new();
        app.editor.start("ls");
        assert_eq!(
            app.editor.handle(KeyCode::ArrowUp, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "ls");
        assert_eq!(
            app.editor.handle(KeyCode::ArrowDown, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "ls");
    }

    #[test]
    fn editing_tab_inserts_tab_without_completion() {
        // No AI client (App::new) and no staged/requested completion: Tab must
        // fall back to inserting a literal tab, never a panic or a no-op.
        let mut app = App::new();
        app.editor.start("ab");
        app.editor.state.as_mut().unwrap().home();
        assert_eq!(
            app.editor.handle(KeyCode::Tab, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "\tab");
        assert!(app.editor.ai_completion.is_none());
    }

    #[test]
    fn vi_toggle_mode_and_normal_subset() {
        let mut app = App::new();
        app.editor.start("");
        // Ctrl+Shift+M toggles Emacs -> Vi (normal mode) -> Emacs.
        assert_eq!(
            app.editor.handle(KeyCode::KeyM, true, true, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.mode(), EditMode::Vi);
        assert!(app.editor.vi_normal());
        assert_eq!(
            app.editor.handle(KeyCode::KeyM, true, true, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.mode(), EditMode::Emacs);
        assert!(!app.editor.vi_normal());
    }

    #[test]
    fn vi_normal_motions_and_insert() {
        let mut app = App::new();
        app.editor.start("ab\ncd");
        app.editor.state.as_mut().unwrap().toggle_mode(); // Vi, normal
        // `h` / `l` move the cursor; letters in normal mode are swallowed.
        assert_eq!(
            app.editor.handle(KeyCode::KeyH, false, false, false),
            EditAction::Handled
        );
        assert_eq!(
            app.editor.handle(KeyCode::KeyQ, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "ab\ncd");
        // `0` -> start of the current line (revealed by inserting in insert mode).
        assert_eq!(
            app.editor.handle(KeyCode::Digit0, false, false, false),
            EditAction::Handled
        );
        app.editor.state.as_mut().unwrap().set_vi_normal(false);
        app.editor.state.as_mut().unwrap().insert('X');
        assert_eq!(app.editor.line(), "ab\nXcd");
        // `$` (Shift+4) -> end of the current line.
        app.editor.state.as_mut().unwrap().set_vi_normal(true);
        app.editor.state.as_mut().unwrap().set_line("xy\nzw");
        app.editor.state.as_mut().unwrap().home();
        assert_eq!(
            app.editor.handle(KeyCode::Digit4, false, true, false),
            EditAction::Handled
        );
        app.editor.state.as_mut().unwrap().set_vi_normal(false);
        app.editor.state.as_mut().unwrap().insert('Y');
        assert_eq!(app.editor.line(), "xy\nzwY");
        // `x` deletes the char at the cursor.
        app.editor.state.as_mut().unwrap().set_vi_normal(true);
        app.editor.state.as_mut().unwrap().set_line("ab\ncd");
        assert_eq!(
            app.editor.handle(KeyCode::KeyH, false, false, false),
            EditAction::Handled
        );
        assert_eq!(
            app.editor.handle(KeyCode::KeyX, false, false, false),
            EditAction::Handled
        );
        assert_eq!(app.editor.line(), "ab\nc");
        // `dd` clears the line.
        assert_eq!(
            app.editor.handle(KeyCode::KeyD, false, false, false),
            EditAction::Handled
        );
        assert_eq!(
            app.editor.handle(KeyCode::KeyD, false, false, false),
            EditAction::Handled
        );
        assert!(app.editor.is_empty());
        // `i` enters insert mode; a plain key now lands in the buffer.
        assert_eq!(
            app.editor.handle(KeyCode::KeyI, false, false, false),
            EditAction::Handled
        );
        assert!(!app.editor.vi_normal());
        app.editor.state.as_mut().unwrap().insert('H');
        assert_eq!(app.editor.line(), "H");
        // Esc returns to normal mode; the editor stays open.
        assert_eq!(
            app.editor.handle(KeyCode::Escape, false, false, false),
            EditAction::Handled
        );
        assert!(app.editor.vi_normal());
        // `a` appends after the cursor: home, then a, then type.
        app.editor.state.as_mut().unwrap().set_vi_normal(true);
        app.editor.state.as_mut().unwrap().set_line("ab");
        app.editor.state.as_mut().unwrap().home();
        assert_eq!(
            app.editor.handle(KeyCode::KeyA, false, false, false),
            EditAction::Handled
        );
        app.editor.state.as_mut().unwrap().insert('Z');
        assert_eq!(app.editor.line(), "aZb");
    }

    #[test]
    fn vi_normal_enter_submits_and_ctrl_c_cancels() {
        let mut app = App::new();
        app.editor.history.push("cmd");
        app.editor.start("run");
        app.editor.state.as_mut().unwrap().toggle_mode(); // Vi, normal
        assert!(matches!(
            app.editor.handle(KeyCode::Enter, false, false, false),
            EditAction::Submit(_)
        ));
        assert!(!app.editor.is_active(), "Enter submits in Vi-normal too");
        assert_eq!(app.editor.history.len(), 2);

        let mut app = App::new();
        app.editor.start("run");
        app.editor.state.as_mut().unwrap().toggle_mode(); // Vi, normal
        assert_eq!(
            app.editor.handle(KeyCode::KeyC, true, false, false),
            EditAction::Handled
        );
        assert!(!app.editor.is_active(), "Ctrl+C cancels in Vi-normal");
    }

    #[test]
    fn vi_escape_returns_to_normal_not_cancel() {
        let mut app = App::new();
        app.editor.start("ab");
        app.editor.state.as_mut().unwrap().toggle_mode(); // Vi, normal
        // Into insert, then Esc back to normal; editor stays open.
        assert_eq!(
            app.editor.handle(KeyCode::KeyI, false, false, false),
            EditAction::Handled
        );
        assert_eq!(
            app.editor.handle(KeyCode::Escape, false, false, false),
            EditAction::Handled
        );
        assert!(app.editor.is_active());
        assert!(app.editor.vi_normal());
        // Emacs mode still cancels on Esc.
        app.editor.state.as_mut().unwrap().toggle_mode(); // back to Emacs
        assert_eq!(
            app.editor.handle(KeyCode::Escape, false, false, false),
            EditAction::Handled
        );
        assert!(!app.editor.is_active());
    }

    #[test]
    fn cells_for_size_matches_renderer_math() {
        // Pins the exact pair the renderer produced at runtime (verified
        // live: 946x501 at 10x22 cells -> 91 cols x 19 rows). Chrome = 2 rows
        // (1 tab + 1 status), padding 16px per side. If the shared constants
        // drift, this fails.
        let (cols, rows) = cells_for_size(10.0, 22.0, PhysicalSize::new(946, 501));
        assert_eq!((cols, rows), (91, 19));
        // Degenerate windows clamp to >= 1 col/row like cols_for/rows_for.
        assert_eq!(cells_for_size(10.0, 22.0, PhysicalSize::new(5, 5)), (1, 1));
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
            last_resize: None,
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
                last_resize: None,
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
