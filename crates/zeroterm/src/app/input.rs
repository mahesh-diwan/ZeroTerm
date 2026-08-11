use std::collections::VecDeque;

/// Readline word characters: alphanumerics and the underscore.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// ponytail: fixed ring cap; persistence and dedup-fuzz deferred.
const HISTORY_CAP: usize = 500;

/// Readline-style command history for the local line editor: a fixed ring of
/// submitted lines (oldest evicted past the cap). prev() walks back from the
/// most-recent entry, stashing the in-progress line on the first call so
/// next() can restore it. No persistence.
pub struct PromptHistory {
    entries: VecDeque<String>,
    /// Steps walked back from the most-recent entry (0 = live position).
    pos: usize,
    /// The in-progress line captured on the first prev(), returned by next()
    /// when navigation reaches the top of the ring.
    stashed: Option<String>,
}

impl PromptHistory {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            pos: 0,
            stashed: None,
        }
    }

    /// Test/observability accessors; navigation uses `prev`/`next`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append a submitted line; ignores empty lines and repeats of the most
    /// recent entry. Resets in-progress navigation.
    pub fn push(&mut self, line: &str) {
        if line.is_empty() || self.entries.back().is_some_and(|last| last == line) {
            return;
        }
        self.entries.push_back(line.to_string());
        while self.entries.len() > HISTORY_CAP {
            self.entries.pop_front();
        }
        self.pos = 0;
        self.stashed = None;
    }

    /// Walk one step toward the oldest entry. The first call stashes
    /// `current_line` (the in-progress buffer) so Down can return to it.
    /// Repeated calls clamp at the oldest entry.
    pub fn prev(&mut self, current_line: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        if self.pos == 0 {
            self.stashed = Some(current_line.to_string());
            self.pos = 1;
        } else {
            self.pos = (self.pos + 1).min(self.entries.len());
        }
        self.entries.get(self.entries.len() - self.pos).cloned()
    }

    /// Walk one step toward the most-recent entry. At the top, returns the
    /// stashed in-progress line once; further calls return None.
    pub fn next(&mut self) -> Option<String> {
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        if self.pos == 0 {
            self.stashed.take()
        } else {
            self.entries.get(self.entries.len() - self.pos).cloned()
        }
    }

    /// The entry navigation currently points at, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn peek(&self) -> Option<String> {
        if self.pos == 0 {
            None
        } else {
            self.entries.get(self.entries.len() - self.pos).cloned()
        }
    }

    /// Forget in-progress navigation; the next prev() starts at the most
    /// recent entry again.
    pub fn reset(&mut self) {
        self.pos = 0;
        self.stashed = None;
    }
}

/// Editing keybinding mode. Emacs is the default; Vi runs a minimal normal /
/// insert split (see `vi_normal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Emacs,
    Vi,
}

/// Local line editor state (not readline): a char buffer plus a cursor,
/// shown in the active pane's tab title while active. The buffer is
/// multiline — `'\n'` is an ordinary buffer char — so cursoring and kills
/// treat line boundaries explicitly. Enter submits `prompt + buffer` to the
/// shell; Esc discards.
pub struct EditingState {
    buffer: Vec<char>,
    cursor: usize,
    prompt: String,
    mode: EditMode,
    vi_normal: bool,
    vi_d_pending: bool,
}

impl EditingState {
    /// Start editing the given line, cursor at the end.
    pub fn from_line(line: &str) -> Self {
        let buffer: Vec<char> = line.chars().collect();
        let cursor = buffer.len();
        Self {
            buffer,
            cursor,
            prompt: String::new(),
            mode: EditMode::Emacs,
            vi_normal: false,
            vi_d_pending: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Kill text after the cursor on the current line (readline C-k). When
    /// the cursor already sits at the end of the line, kill the rest of the
    /// buffer instead (zsh-style fallback).
    pub fn truncate_to_cursor(&mut self) {
        let i = self.cursor.min(self.buffer.len());
        let end = self.line_end(i);
        if end > i {
            // Text after the cursor on this line: drop it, keep the trailing
            // newline and any following lines.
            self.buffer.drain(i..end);
        } else {
            // Cursor at the line end: kill everything after it.
            self.buffer.truncate(i);
        }
    }

    pub fn insert(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.buffer.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.buffer.len());
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end(self.cursor);
    }

    /// Char offset of the start of the line containing `col`.
    fn line_start(&self, col: usize) -> usize {
        let i = col.min(self.buffer.len());
        self.buffer[..i]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |p| p + 1)
    }

    /// Char offset of the end of the line containing `col` (before the next
    /// newline, or the buffer end). When `col` points at a newline, the line
    /// ends there.
    fn line_end(&self, col: usize) -> usize {
        let i = col.min(self.buffer.len());
        if self.buffer.get(i) == Some(&'\n') {
            return i;
        }
        self.buffer[i..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(self.buffer.len(), |off| i + off)
    }

    /// Start of the current or previous word (readline M-b).
    pub fn word_left(&mut self) {
        self.cursor = self.word_boundary_backward(self.cursor);
    }

    /// End of the next word (readline M-f).
    pub fn word_right(&mut self) {
        self.cursor = self.word_boundary_forward(self.cursor);
    }

    /// Delete the word after the cursor (readline M-d).
    pub fn delete_word_after(&mut self) {
        let end = self.word_boundary_forward(self.cursor);
        self.buffer.drain(self.cursor..end);
    }

    /// Delete the word before the cursor (readline M-backspace / M-h).
    pub fn delete_word_before(&mut self) {
        let start = self.word_boundary_backward(self.cursor);
        self.buffer.drain(start..self.cursor);
        self.cursor = start;
    }

    fn word_boundary_backward(&self, col: usize) -> usize {
        let mut i = col.min(self.buffer.len());
        while i > 0 && !is_word_char(self.buffer[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word_char(self.buffer[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_boundary_forward(&self, col: usize) -> usize {
        let mut i = col.min(self.buffer.len());
        while i < self.buffer.len() && !is_word_char(self.buffer[i]) {
            i += 1;
        }
        while i < self.buffer.len() && is_word_char(self.buffer[i]) {
            i += 1;
        }
        i
    }

    /// Full line (prompt + buffer), what Enter submits to the shell.
    pub fn line(&self) -> String {
        let mut s = self.prompt.clone();
        s.extend(self.buffer.iter());
        s
    }

    /// Buffer content only (the partial command line, no prompt prefix).
    pub fn buffer_text(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Replace the whole buffer with `line`, cursor at end (history recall).
    pub fn set_line(&mut self, line: &str) {
        self.buffer = line.chars().collect();
        self.cursor = self.buffer.len();
    }

    /// Insert `suffix` at the cursor and advance past it. Retained as a test
    /// utility (the editor inserts chars one at a time via `insert`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn accept_suffix(&mut self, suffix: &str) {
        for c in suffix.chars() {
            self.buffer.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Tab-title rendering: prompt + buffer with a block cursor at the edit
    /// position, optionally followed by a `suffix` rendered past the cursor
    /// (production callers pass an empty suffix; tests preview text past it).
    /// Embedded newlines render as the visible escape `\n` (one line fits the
    /// tab title). Vi-normal mode tags the title `[vi]`.
    pub fn display_with_suffix(&self, suffix: &str) -> String {
        let mut s = if self.mode == EditMode::Vi && self.vi_normal {
            String::from("[vi] ")
        } else {
            String::from("[edit] ")
        };
        s.push_str(&self.prompt);
        for (i, c) in self.buffer.iter().enumerate() {
            if i == self.cursor {
                s.push('▌');
            }
            if *c == '\n' {
                s.push_str("\\n");
            } else {
                s.push(*c);
            }
        }
        if self.cursor == self.buffer.len() {
            s.push('▌');
        }
        s.push_str(suffix);
        s
    }

    /// Tab-title rendering: prompt + buffer with a block cursor at the edit
    /// position.
    pub fn display(&self) -> String {
        self.display_with_suffix("")
    }

    /// Current editing mode.
    pub fn mode(&self) -> EditMode {
        self.mode
    }

    /// Flip Emacs <-> Vi. Entering Vi starts in normal mode; leaving it resets
    /// any pending vi command prefix.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            EditMode::Emacs => EditMode::Vi,
            EditMode::Vi => EditMode::Emacs,
        };
        self.vi_normal = self.mode == EditMode::Vi;
        self.vi_d_pending = false;
    }

    /// Whether Vi-normal mode is active (motions instead of text input).
    pub fn vi_normal(&self) -> bool {
        self.vi_normal
    }

    pub fn set_vi_normal(&mut self, normal: bool) {
        self.vi_normal = normal;
        if !normal {
            self.vi_d_pending = false;
        }
    }

    /// Whether a `d` in Vi-normal mode is waiting for a second `d`.
    pub fn vi_d_pending(&self) -> bool {
        self.vi_d_pending
    }

    pub fn set_vi_d_pending(&mut self, pending: bool) {
        self.vi_d_pending = pending;
    }
}

/// Start of the current or previous word in a raw char line (readline M-b).
pub fn word_left(chars: &[char], col: usize) -> usize {
    let mut i = col.saturating_sub(1);
    while i > 0 && chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i -= 1;
    }
    while i > 0 && chars.get(i - 1).is_some_and(|c| !c.is_whitespace()) {
        i -= 1;
    }
    i
}

/// End of the next word in a raw char line (readline M-f).
pub fn word_right(chars: &[char], col: usize, cols: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::{word_left, word_right, EditMode, EditingState, PromptHistory, HISTORY_CAP};

    #[test]
    fn word_movement_bounds() {
        let line: Vec<char> = "  hello world  ".chars().collect();
        assert_eq!(word_left(&line, 7), 2);
        assert_eq!(word_left(&line, 12), 8);
        assert_eq!(word_right(&line, 2, 20), 6);
        assert_eq!(word_right(&line, 8, 20), 12);
    }

    #[test]
    fn editor_words() {
        let mut e = EditingState::from_line("one two");
        e.word_left();
        e.word_right();
        e.end();
        assert_eq!(e.line(), "one two");
        e.home();
        e.delete_word_after();
        assert_eq!(e.line(), " two");
    }

    #[test]
    fn buffer_text_and_accept_suffix() {
        let mut e = EditingState::from_line("hello wor");
        assert_eq!(e.buffer_text(), "hello wor");
        e.accept_suffix("ld");
        assert_eq!(e.buffer_text(), "hello world");
        assert_eq!(e.line(), "hello world");
    }

    #[test]
    fn accept_suffix_inserts_at_cursor() {
        let mut e = EditingState::from_line("ab");
        e.home();
        e.accept_suffix("X");
        assert_eq!(e.line(), "Xab");
    }

    #[test]
    fn display_with_suffix_shows_ghost_after_cursor() {
        let e = EditingState::from_line("hel");
        assert!(e.display_with_suffix("lo").contains("hel▌lo"));
        assert!(e.display().contains("hel▌"));
    }

    #[test]
    fn set_line_replaces_buffer_and_cursor() {
        let mut e = EditingState::from_line("old line");
        e.home();
        e.set_line("new");
        assert_eq!(e.line(), "new");
        // Cursor at end: typing appends, backspace eats the last char.
        e.insert('!');
        assert_eq!(e.line(), "new!");
        e.backspace();
        assert_eq!(e.line(), "new");
    }

    #[test]
    fn history_push_ignores_empty_and_duplicate() {
        let mut h = PromptHistory::new();
        h.push("ls");
        h.push("");
        h.push("ls");
        assert_eq!(h.len(), 1);
        assert!(!h.is_empty());
    }

    #[test]
    fn history_prev_next_walks_and_stashes() {
        let mut h = PromptHistory::new();
        h.push("a");
        h.push("b");
        h.push("c");
        // prev walks back from most-recent, stashing the in-progress line.
        assert_eq!(h.prev("typed"), Some("c".to_string()));
        assert_eq!(h.prev("typed"), Some("b".to_string()));
        assert_eq!(h.prev("typed"), Some("a".to_string()));
        // Clamps at the oldest entry.
        assert_eq!(h.prev("typed"), Some("a".to_string()));
        // next walks forward, returning the stash at the top.
        assert_eq!(h.next(), Some("b".to_string()));
        assert_eq!(h.next(), Some("c".to_string()));
        assert_eq!(h.next(), Some("typed".to_string()));
        assert_eq!(h.next(), None);
    }

    #[test]
    fn history_prev_next_empty_returns_none() {
        let mut h = PromptHistory::new();
        assert_eq!(h.prev("typed"), None);
        assert_eq!(h.next(), None);
    }

    #[test]
    fn history_evicts_oldest_at_cap() {
        let mut h = PromptHistory::new();
        for i in 0..HISTORY_CAP + 10 {
            h.push(&format!("cmd{}", i));
        }
        assert_eq!(h.len(), HISTORY_CAP);
        assert_eq!(h.peek(), None);
        assert_eq!(h.prev(""), Some("cmd509".to_string()));
        assert_eq!(h.next(), Some("".to_string()));
        assert_eq!(h.peek(), None);
    }

    #[test]
    fn history_reset_forgets_navigation() {
        let mut h = PromptHistory::new();
        h.push("ls");
        h.prev("typed");
        h.reset();
        assert_eq!(h.peek(), None);
        assert_eq!(h.prev("again"), Some("ls".to_string()));
        assert_eq!(h.next(), Some("again".to_string()));
    }

    #[test]
    fn multiline_insert_and_cursor_cross_newline() {
        let mut e = EditingState::from_line("ab");
        e.insert('\n');
        e.insert('c');
        assert_eq!(e.buffer_text(), "ab\nc");
        assert_eq!(e.line(), "ab\nc");
        // Cursor at the end (4); walk left across the newline.
        e.left();
        e.left();
        e.left();
        assert_eq!(e.line(), "ab\nc");
        // A backspace right after the newline joins the lines.
        e.right();
        e.right();
        e.backspace();
        assert_eq!(e.line(), "abc");
    }

    #[test]
    fn multiline_home_end_stay_on_current_line() {
        let mut e = EditingState::from_line("ab\ncd\nef");
        e.end(); // cursor at buffer end (line "ef")
        e.left();
        assert_eq!(e.line(), "ab\ncd\nef");
        e.home();
        assert_eq!(e.line(), "ab\ncd\nef");
        e.insert('X');
        assert_eq!(e.line(), "ab\ncd\nXef");
        e.right();
        e.right();
        e.end();
        e.insert('Y');
        assert_eq!(e.line(), "ab\ncd\nXefY");
        // Home from the middle of a line goes to that line's start.
        e.set_line("one\ntwo");
        e.end();
        e.left();
        e.left();
        e.home();
        e.insert('!');
        assert_eq!(e.line(), "one\n!two");
    }

    #[test]
    fn multiline_home_end_boundary_at_newline() {
        let mut e = EditingState::from_line("ab\ncd");
        // Cursor right before the '\n' (end of line "ab").
        e.home();
        e.left();
        e.end();
        assert_eq!(e.line(), "ab\ncd");
        e.insert('Z');
        assert_eq!(e.line(), "abZ\ncd");
    }

    #[test]
    fn ctrl_k_kills_to_line_end_then_rest_of_buffer() {
        let mut e = EditingState::from_line("ab\ncd\nef");
        // Cursor onto 'd' (middle line); C-k kills that line's tail only.
        e.end();
        e.left();
        e.left();
        e.left();
        e.left();
        e.truncate_to_cursor();
        assert_eq!(e.line(), "ab\nc\nef");
        // Cursor is now at the end of line "c"; C-k kills the buffer tail.
        e.truncate_to_cursor();
        assert_eq!(e.line(), "ab\nc");
    }

    #[test]
    fn ctrl_k_single_line_matches_old_behavior() {
        let mut e = EditingState::from_line("hello world");
        e.home();
        e.word_right();
        e.truncate_to_cursor();
        assert_eq!(e.line(), "hello");
    }

    #[test]
    fn display_escapes_newlines_and_tags_vi_mode() {
        let mut e = EditingState::from_line("a\nb");
        assert!(e.display().contains("a\\nb"));
        e.toggle_mode();
        assert!(e.display().contains("[vi] "));
        assert!(!e.display().contains("[edit] "));
        e.toggle_mode();
        assert!(e.display().contains("[edit] "));
    }

    #[test]
    fn toggle_mode_flips_and_vi_starts_normal() {
        let mut e = EditingState::from_line("x");
        assert_eq!(e.mode(), EditMode::Emacs);
        assert!(!e.vi_normal());
        e.toggle_mode();
        assert_eq!(e.mode(), EditMode::Vi);
        assert!(e.vi_normal());
        e.set_vi_normal(false);
        assert!(!e.vi_normal());
        e.toggle_mode();
        assert_eq!(e.mode(), EditMode::Emacs);
        assert!(!e.vi_normal());
    }

    #[test]
    fn vi_primitives_move_and_edit() {
        let mut e = EditingState::from_line("ab\ncd");
        assert!(!e.vi_normal());
        e.set_vi_normal(true);
        assert!(e.vi_normal());
        // h / l walk the cursor across embedded newlines.
        e.end();
        e.left();
        e.left();
        e.left();
        e.home();
        e.insert('!');
        assert_eq!(e.line(), "!ab\ncd");
        // x deletes the char at the cursor (now one past the inserted '!').
        e.set_vi_normal(false);
        e.delete();
        assert_eq!(e.line(), "!b\ncd");
        // dd clears the line (set_line to empty).
        e.set_line("");
        assert!(e.is_empty());
    }
}
