//! ZeroTerm - Main entry point

use anyhow::Result;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes};

use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::Size as PtySize;
use zeroterm_core::Parser;
use zeroterm_mux::TabManager;
use zeroterm_render::Renderer;

enum PtyCommand {
    Write(Vec<u8>),
    Resize(PtySize),
    Kill,
}

#[allow(dead_code)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    parser: Option<Parser>,
    tab_manager: TabManager,
    pty_rx: Option<Receiver<Vec<u8>>>,
    pty_tx: Option<Sender<PtyCommand>>,
    modifiers: ModifiersState,
    scroll_offset: usize,
}

#[allow(dead_code)]
impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            parser: None,
            tab_manager: TabManager::new(),
            pty_rx: None,
            pty_tx: None,
            modifiers: ModifiersState::empty(),
            scroll_offset: 0,
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        info!("Initializing ZeroTerm");

        let window_attrs = WindowAttributes::default()
            .with_title("ZeroTerm")
            .with_inner_size(winit::dpi::LogicalSize::new(1200, 800))
            .with_resizable(true);

        let window = Arc::new(event_loop.create_window(window_attrs)?);

        let font_size = 14.0;
        let renderer = pollster::block_on(Renderer::new(window.clone(), font_size))?;

        let size = window.inner_size();
        let cell_w = font_size * 0.6;
        let cell_h = font_size * 1.2;
        let cols = (size.width as f32 / cell_w) as usize;
        let rows = (size.height as f32 / cell_h) as usize;

        let parser = Parser::new(cols, rows);

        // Detect default shell
        let shell = if std::path::Path::new("/bin/zsh").exists() {
            "/bin/zsh"
        } else if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash"
        } else {
            "/bin/sh"
        };

        // Spawn PTY — resize BEFORE moving into thread
        let mut backend = PortablePtyBackend::new()?;
        let mut process = backend.spawn(shell, &[], None)?;
        process.resize(PtySize { cols, rows })?;

        // Channels: output_tx→pty_rx (PTY→main), pty_tx→input_rx (main→PTY)
        let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
        let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

        // Reader thread owns the process
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                // Check for pending commands
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
                // Read PTY output
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

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.parser = Some(parser);
        self.pty_rx = Some(pty_rx);
        self.pty_tx = Some(pty_tx);

        info!("ZeroTerm initialized: {}x{} ({})", cols, rows, shell);
        Ok(())
    }

    fn drain_pty(&mut self) -> bool {
        let mut got_data = false;
        if let (Some(parser), Some(rx)) = (&mut self.parser, &self.pty_rx) {
            while let Ok(data) = rx.try_recv() {
                parser.parse(&data);
                got_data = true;
            }
        }
        got_data
    }

    fn render(&mut self) -> Result<()> {
        if let (Some(renderer), Some(parser)) = (&mut self.renderer, &self.parser) {
            renderer.render(parser.screen(), self.scroll_offset)?;
        }
        Ok(())
    }

    fn write_pty(&self, data: &[u8]) {
        if let Some(tx) = &self.pty_tx {
            let _ = tx.send(PtyCommand::Write(data.to_vec()));
        }
    }

    fn resize_pty(&self, cols: usize, rows: usize) {
        if let Some(tx) = &self.pty_tx {
            let _ = tx.send(PtyCommand::Resize(PtySize { cols, rows }));
        }
    }

    fn max_scroll_offset(&self) -> usize {
        if let Some(parser) = &self.parser {
            let screen = parser.screen();
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
                if let Some(tx) = &self.pty_tx {
                    let _ = tx.send(PtyCommand::Kill);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let font_size = 14.0;
                let cell_w = font_size * 0.6;
                let cell_h = font_size * 1.2;
                let cols = (size.width as f32 / cell_w) as usize;
                let rows = (size.height as f32 / cell_h) as usize;

                self.resize_pty(cols, rows);
                if let Some(parser) = &mut self.parser {
                    parser.screen_mut().resize(cols, rows);
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
                let alt = self.modifiers.alt_key();

                match &event.physical_key {
                    PhysicalKey::Code(code) => {
                        use winit::keyboard::KeyCode;

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
                // Drain PTY before each render
                self.drain_pty();

                if let Err(e) = self.render() {
                    error!("Render error: {}", e);
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
                        let lines = (pos.y as f32 / 20.0).round() as usize;
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
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app)?;

    Ok(())
}
