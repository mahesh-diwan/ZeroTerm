//! The pane: one PTY-backed terminal viewport — its parser, its pty channels,
//! and the typed events draining produces. Carved out of `session.rs`, which
//! used to hold three unrelated clusters (pane, spawn, session) in one file;
//! the SessionManager now only holds panes by id.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use zeroterm_core::parser::Parser;
use zeroterm_core::screen::Size as PtySize;

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
    /// Latched bell activity for an inactive pane (kitty renders 🔔 on the
    /// tab). Drained from the screen each drain_pty and cleared when the tab
    /// gains focus.
    pub bell_rung: bool,
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

/// What one drain pass of a pane produced, typed so the app's `drain_pty`
/// loop can apply side effects without reaching into the parser. One event per
/// side effect: the app decides (config gates, active-pane guards) what to do
/// with each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneEvent {
    /// OSC 52 clipboard text, for the system clipboard.
    Clipboard(String),
    /// Kitty protocol reply (CSI ? u), relayed back to the pane's pty.
    Response(Vec<u8>),
    /// OSC 9 desktop-notification message.
    Notification(String),
    /// The bell rang: latch bell activity + flash the visual bell.
    Bell,
    /// ED 2/3 cleared screen/scrollback: the viewport must snap to bottom.
    ClearViewport,
    /// The pane title changed (new title).
    Title(String),
    /// Sync-output mode state (active = output batches render atomically).
    SyncOutput(bool),
    /// The pty disconnected: the pane's process exited (sticky, emitted once).
    Exited(Option<i32>),
}

impl PaneState {
    /// Drain available pty output and classify what happened into typed
    /// events. Returns `(got_data, events)` — `got_data` mirrors `drain()`
    /// (any bytes parsed); `events` also carries `Exited` when the pty
    /// disconnected even with no bytes in this pass.
    pub fn drain_events(&mut self) -> (bool, Vec<PaneEvent>) {
        let got = self.drain();
        if !got && !self.pty_dead {
            return (false, Vec::new());
        }
        let mut events = Vec::new();
        if let Some(text) = self.parser.take_clipboard_text() {
            events.push(PaneEvent::Clipboard(text));
        }
        if let Some(resp) = self.parser.take_response() {
            events.push(PaneEvent::Response(resp));
        }
        if let Some(msg) = self.parser.take_notification() {
            events.push(PaneEvent::Notification(msg));
        }
        if self.parser.screen_mut().take_bell() {
            self.bell_rung = true;
            events.push(PaneEvent::Bell);
        }
        if self.parser.take_clear_flag() {
            events.push(PaneEvent::ClearViewport);
        }
        let new_title = self.parser.screen().title().to_string();
        if new_title != self.title {
            self.title = new_title.clone();
            events.push(PaneEvent::Title(new_title));
        }
        events.push(PaneEvent::SyncOutput(self.parser.sync_output()));
        if self.pty_dead {
            events.push(PaneEvent::Exited(self.parser.screen().last_exit()));
        }
        (got, events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pane_with(rx: Receiver<Vec<u8>>) -> PaneState {
        PaneState {
            parser: Parser::new(80, 24),
            pty_rx: rx,
            pty_tx: mpsc::channel().0,
            title: String::new(),
            pane_cmd: String::new(),
            pty_dead: false,
            bell_rung: false,
            last_resize: None,
        }
    }

    #[test]
    fn drain_events_emits_bell_event_and_latches() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut pane = pane_with(rx);
        tx.send(b"\x07".to_vec()).unwrap(); // BEL
        let (got, events) = pane.drain_events();
        assert!(got);
        assert!(events.contains(&PaneEvent::Bell));
        assert!(pane.bell_rung, "bell latched on the pane");
    }

    #[test]
    fn drain_events_emits_exited_on_disconnect() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut pane = pane_with(rx);
        drop(tx); // disconnect
        let (got, events) = pane.drain_events();
        assert!(!got);
        assert!(
            events.contains(&PaneEvent::Exited(None)),
            "disconnect must emit Exited"
        );
        assert!(pane.pty_dead, "pane marked dead on disconnect");
    }

    #[test]
    fn drain_events_empty_channel_produces_no_events() {
        let (_tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut pane = pane_with(rx);
        let (got, events) = pane.drain_events();
        assert!(!got);
        assert!(events.is_empty());
    }
}
