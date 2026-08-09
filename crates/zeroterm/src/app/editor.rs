//! Local line editor (Alt+E): the readline-style Emacs and minimal Vi-normal
//! key rules over [`EditingState`], plus history navigation.
//!
//! The editor owns everything about editing — the buffer state and history
//! navigation. It never touches the App or the PTY: instead `handle()` returns
//! an [`EditAction`] describing the shell effect the App should execute (e.g.
//! [`EditAction::Submit`]), so every key rule here is testable without a
//! window, a session, or a display.

use winit::keyboard::KeyCode;

use super::input::{EditMode, EditingState, PromptHistory};

/// What the App should do after the editor consumed a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    /// Key consumed; the buffer may have changed (redraw the tab title).
    Handled,
    /// Submit `line` to the shell; the editor is now closed.
    Submit(String),
    /// Key not consumed — route it onward (plain text input, Alt+E, etc.).
    Pass,
}

/// The line editor session. Inactive (no editing) when `state` is `None`.
pub struct LineEditor {
    // `state` / `history` are `pub(crate)` as a TEST-ONLY seam: the crate's
    // unit tests drive the editor state directly. App code uses the methods
    // above, never the fields.
    /// Current editing session. `None` while not editing.
    pub(crate) state: Option<EditingState>,
    /// Command history for Up/Down recall.
    pub(crate) history: PromptHistory,
}

impl Default for LineEditor {
    fn default() -> Self {
        Self {
            state: None,
            history: PromptHistory::new(),
        }
    }
}

impl LineEditor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an editing session is currently active.
    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }

    /// Whether the active buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.state.as_ref().map_or(true, |s| s.is_empty())
    }

    /// Line text of the active buffer (empty when not editing).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn line(&self) -> String {
        self.state.as_ref().map(|s| s.line()).unwrap_or_default()
    }

    /// The mode of the active session (Emacs when not editing).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mode(&self) -> EditMode {
        self.state
            .as_ref()
            .map(|s| s.mode())
            .unwrap_or(EditMode::Emacs)
    }

    /// Whether the active session is in Vi-normal mode.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn vi_normal(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.vi_normal())
    }

    /// Begin an editing session seeded with `line` (the shell's current line).
    /// Resets history navigation so Up starts at the most recent entry.
    pub fn start(&mut self, line: &str) {
        let state = EditingState::from_line(line);
        self.history.reset();
        self.state = Some(state);
    }

    /// Close the session without submitting (buffer discarded).
    pub fn cancel(&mut self) {
        self.state = None;
    }

    /// Tab-title text for the active pane while editing: the live buffer, or
    /// empty when inactive.
    pub fn display_line(&self) -> String {
        self.state.as_ref().map(|s| s.display()).unwrap_or_default()
    }

    /// Insert printable text into the buffer (the IME/text-input path).
    pub fn insert_text(&mut self, text: &str) {
        if let Some(state) = self.state.as_mut() {
            for c in text.chars() {
                state.insert(c);
            }
        }
    }

    /// Route an editing key. Returns what the App should do.
    pub fn handle(&mut self, code: KeyCode, ctrl: bool, shift: bool, alt: bool) -> EditAction {
        let Some(state) = self.state.as_mut() else {
            return EditAction::Pass;
        };
        // Ctrl+Shift+M toggles the editing mode (Emacs <-> Vi). Free in the
        // global key dispatch, so it is safe to claim while editing.
        if ctrl && shift && !alt && code == KeyCode::KeyM {
            state.toggle_mode();
            return EditAction::Handled;
        }
        // Vi-normal mode interprets its own key subset; Vi-insert and Emacs
        // behave alike below.
        if state.mode() == EditMode::Vi && state.vi_normal() {
            return self.handle_vi(code, ctrl, alt, shift);
        }
        // Self-borrowing actions first (submit / history) so no editor borrow
        // is held across them.
        match code {
            KeyCode::Enter => return self.submit(),
            KeyCode::ArrowUp => return self.history_prev(),
            KeyCode::KeyP if ctrl && !alt => return self.history_prev(),
            KeyCode::ArrowDown => return self.history_next(),
            KeyCode::KeyN if ctrl && !alt => return self.history_next(),
            KeyCode::Tab => return self.tab(),
            _ => {}
        }
        let state = self.state.as_mut().unwrap();
        let mut action = EditAction::Handled;
        match code {
            // In Vi-insert mode Esc returns to normal instead of canceling.
            KeyCode::Escape => {
                let is_vi = state.mode() == EditMode::Vi;
                if is_vi {
                    state.set_vi_normal(true);
                } else {
                    self.state = None;
                }
            }
            // Word moves and deletes (readline M-b / M-f / M-d / M-backspace).
            KeyCode::KeyB if alt && !ctrl => state.word_left(),
            KeyCode::KeyF if alt && !ctrl => state.word_right(),
            KeyCode::KeyD if alt && !ctrl => state.delete_word_after(),
            KeyCode::Backspace if alt && !ctrl => state.delete_word_before(),
            // Cursor / kill chords (readline C-a / C-e / C-k).
            KeyCode::KeyA if ctrl && !alt => state.home(),
            KeyCode::KeyE if ctrl && !alt => state.end(),
            KeyCode::KeyK if ctrl && !alt => state.truncate_to_cursor(),
            // Cancel like Esc, discarding the buffer without touching the shell.
            KeyCode::KeyC if ctrl && !alt => self.state = None,
            KeyCode::KeyD if ctrl && !alt => {
                if state.is_empty() {
                    self.state = None;
                }
            }
            KeyCode::Backspace => state.backspace(),
            KeyCode::Delete => state.delete(),
            KeyCode::ArrowLeft => state.left(),
            KeyCode::ArrowRight => state.right(),
            KeyCode::Home => state.home(),
            KeyCode::End => state.end(),
            // Let Alt+E fall through so the same key exits edit mode.
            KeyCode::KeyE if alt && !ctrl => action = EditAction::Pass,
            // Swallow other ctrl/alt chords; plain keys fall through to the
            // text-input path which inserts them into the buffer.
            _ if ctrl || alt => {}
            _ => action = EditAction::Pass,
        }
        action
    }

    /// Submit the buffer to the shell: push to history and hand the line to
    /// the App. Empty buffers still submit a bare newline (the App's job).
    fn submit(&mut self) -> EditAction {
        let Some(state) = self.state.take() else {
            return EditAction::Handled;
        };
        let line = state.line();
        self.history.push(&line);
        EditAction::Submit(line)
    }

    /// Vi-normal mode: `i`/`a` enter insert, `h`/`l`/arrows move, `0`/`$`
    /// move to line start/end, `x` deletes the char at the cursor, `d d`
    /// clears the line. Everything is consumed so plain keys never leak into
    /// the buffer; Enter still submits and Ctrl+C still cancels.
    fn handle_vi(&mut self, code: KeyCode, ctrl: bool, alt: bool, shift: bool) -> EditAction {
        if code == KeyCode::KeyC && ctrl && !alt {
            self.state = None;
            return EditAction::Handled;
        }
        if ctrl || alt {
            return EditAction::Handled;
        }
        // Self-borrowing actions first.
        match code {
            KeyCode::Enter => return self.submit(),
            KeyCode::ArrowUp => return self.history_prev(),
            KeyCode::ArrowDown => return self.history_next(),
            _ => {}
        }
        let state = self.state.as_mut().unwrap();
        // `d` is a two-key prefix: `d d` clears the line.
        if state.vi_d_pending() {
            state.set_vi_d_pending(false);
            if code == KeyCode::KeyD {
                state.set_line("");
                return EditAction::Handled;
            }
        }
        match code {
            KeyCode::Escape => {} // already in normal mode
            KeyCode::KeyI => state.set_vi_normal(false),
            KeyCode::KeyA => {
                state.right();
                state.set_vi_normal(false);
            }
            KeyCode::KeyH | KeyCode::ArrowLeft => state.left(),
            KeyCode::KeyL | KeyCode::ArrowRight => state.right(),
            KeyCode::Digit0 => state.home(),
            // `$` is Shift+4 on the US physical layout (no KeyCode::Dollar).
            KeyCode::Digit4 if shift => state.end(),
            KeyCode::KeyX => state.delete(),
            KeyCode::KeyD => state.set_vi_d_pending(true),
            KeyCode::Backspace => state.backspace(),
            _ => {}
        }
        EditAction::Handled
    }

    /// Recall the previous history entry into the buffer.
    fn history_prev(&mut self) -> EditAction {
        let current = self
            .state
            .as_ref()
            .map(|s| s.buffer_text())
            .unwrap_or_default();
        if let Some(line) = self.history.prev(&current) {
            self.state.as_mut().unwrap().set_line(&line);
        }
        EditAction::Handled
    }

    /// Recall the next (more recent) history entry, or restore the stashed
    /// in-progress line at the top.
    fn history_next(&mut self) -> EditAction {
        if let Some(line) = self.history.next() {
            self.state.as_mut().unwrap().set_line(&line);
        }
        EditAction::Handled
    }

    /// Tab: insert a literal tab character.
    fn tab(&mut self) -> EditAction {
        self.state.as_mut().unwrap().insert('\t');
        EditAction::Handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(editor: &mut LineEditor, code: KeyCode, ctrl: bool, shift: bool, alt: bool) -> EditAction {
        editor.handle(code, ctrl, shift, alt)
    }

    #[test]
    fn inactive_editor_passes_all_keys() {
        let mut editor = LineEditor::new();
        assert_eq!(handle(&mut editor, KeyCode::KeyA, false, false, false), EditAction::Pass);
        assert_eq!(handle(&mut editor, KeyCode::Enter, false, false, false), EditAction::Pass);
        assert!(!editor.is_active());
    }

    #[test]
    fn submit_returns_line_and_pushes_history() {
        let mut editor = LineEditor::new();
        editor.start("echo hi");
        assert!(editor.is_active());
        assert_eq!(
            handle(&mut editor, KeyCode::Enter, false, false, false),
            EditAction::Submit("echo hi".to_string())
        );
        assert!(!editor.is_active());
        assert_eq!(editor.history.len(), 1);
    }

    #[test]
    fn plain_key_passes_to_text_input() {
        let mut editor = LineEditor::new();
        editor.start("ab");
        assert_eq!(handle(&mut editor, KeyCode::KeyX, false, false, false), EditAction::Pass);
        // ...and the text-input path lands it in the buffer.
        editor.insert_text("x");
        assert_eq!(editor.line(), "abx");
    }

    #[test]
    fn ctrl_chords_swallowed_but_cancels() {
        let mut editor = LineEditor::new();
        editor.start("hello");
        // Ctrl+K kills to end of line (buffer changes, key consumed).
        assert_eq!(handle(&mut editor, KeyCode::KeyK, true, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "hello");
        editor.start("hello");
        editor.state.as_mut().unwrap().home();
        assert_eq!(handle(&mut editor, KeyCode::KeyK, true, false, false), EditAction::Handled);
        assert!(editor.line().is_empty());
        // Ctrl+C cancels.
        editor.start("hello");
        assert_eq!(handle(&mut editor, KeyCode::KeyC, true, false, false), EditAction::Handled);
        assert!(!editor.is_active());
        // Alt+E passes through so the App can exit edit mode.
        editor.start("hi");
        assert_eq!(handle(&mut editor, KeyCode::KeyE, false, false, true), EditAction::Pass);
        assert!(editor.is_active());
    }

    #[test]
    fn history_up_down_and_enter_recalls() {
        let mut editor = LineEditor::new();
        editor.history.push("echo one");
        editor.history.push("echo two");
        editor.start("");
        // Up recalls the most recent entry; the in-progress line is stashed.
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "echo two");
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "echo one");
        assert_eq!(handle(&mut editor, KeyCode::ArrowDown, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "echo two");
        // Ctrl+P / Ctrl+N are the readline chords for Up / Down.
        assert_eq!(handle(&mut editor, KeyCode::KeyP, true, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "echo one");
        assert_eq!(handle(&mut editor, KeyCode::KeyN, true, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "echo two");
        // Up then Enter submits "echo one" (not the deduped last entry).
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert!(matches!(
            handle(&mut editor, KeyCode::Enter, false, false, false),
            EditAction::Submit(_)
        ));
        assert!(!editor.is_active());
        assert_eq!(editor.history.len(), 3);
    }

    #[test]
    fn vi_mode_toggle_and_normal_subset() {
        let mut editor = LineEditor::new();
        editor.start("ab\ncd");
        assert_eq!(handle(&mut editor, KeyCode::KeyM, true, true, false), EditAction::Handled);
        assert_eq!(editor.mode(), EditMode::Vi);
        assert!(editor.vi_normal());
        // `h` moves; plain letters are swallowed in normal mode.
        assert_eq!(handle(&mut editor, KeyCode::KeyH, false, false, false), EditAction::Handled);
        assert_eq!(handle(&mut editor, KeyCode::KeyQ, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "ab\ncd");
        // `0` -> start of line, then insert mode lets text land.
        assert_eq!(handle(&mut editor, KeyCode::Digit0, false, false, false), EditAction::Handled);
        assert_eq!(handle(&mut editor, KeyCode::KeyI, false, false, false), EditAction::Handled);
        assert!(!editor.vi_normal());
        editor.insert_text("X");
        assert_eq!(editor.line(), "ab\nXcd");
        // `d d` clears the line.
        assert_eq!(handle(&mut editor, KeyCode::KeyM, true, true, false), EditAction::Handled); // back to Emacs
        assert_eq!(handle(&mut editor, KeyCode::KeyM, true, true, false), EditAction::Handled); // to Vi normal
        assert_eq!(handle(&mut editor, KeyCode::KeyD, false, false, false), EditAction::Handled);
        assert_eq!(handle(&mut editor, KeyCode::KeyD, false, false, false), EditAction::Handled);
        assert!(editor.is_empty());
    }

    #[test]
    fn start_resets_history_navigation() {
        // Fresh session: Up starts at the most recent entry, not a stale cursor.
        let mut editor = LineEditor::new();
        editor.history.push("cmd");
        editor.history.push("last");
        editor.start("");
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "last");
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "cmd");
        // Empty history: Up must be a no-op, never a panic.
        let mut editor = LineEditor::new();
        editor.start("ls");
        assert_eq!(handle(&mut editor, KeyCode::ArrowUp, false, false, false), EditAction::Handled);
        assert_eq!(editor.line(), "ls");
    }
}
