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

/// Local single-line editor state (not readline): a char buffer plus a cursor,
/// shown in the active pane's tab title while active. Enter submits
/// `prompt + buffer` to the shell; Esc discards.
pub struct EditingState {
    buffer: Vec<char>,
    cursor: usize,
    prompt: String,
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
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Truncate the buffer to the cursor (readline C-k "kill to end").
    pub fn truncate_to_cursor(&mut self) {
        self.buffer.truncate(self.cursor);
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
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
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

    /// Insert `suffix` at the cursor and advance past it (accept a completion).
    #[cfg_attr(not(feature = "ai"), allow(dead_code))]
    pub fn accept_suffix(&mut self, suffix: &str) {
        for c in suffix.chars() {
            self.buffer.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Tab-title rendering: prompt + buffer with a block cursor at the edit
    /// position, optionally followed by a ghost suffix (pending completion).
    pub fn display_with_suffix(&self, suffix: &str) -> String {
        let mut s = String::from("[edit] ");
        s.push_str(&self.prompt);
        s.extend(self.buffer[..self.cursor].iter());
        s.push('▌');
        s.extend(self.buffer[self.cursor..].iter());
        s.push_str(suffix);
        s
    }

    /// Tab-title rendering: prompt + buffer with a block cursor at the edit
    /// position.
    pub fn display(&self) -> String {
        self.display_with_suffix("")
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
    use super::{word_left, word_right, EditingState, PromptHistory, HISTORY_CAP};

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
}
