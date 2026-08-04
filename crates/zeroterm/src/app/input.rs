/// Readline word characters: alphanumerics and the underscore.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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

    /// Tab-title rendering: prompt + buffer with a block cursor at the edit
    /// position.
    pub fn display(&self) -> String {
        let mut s = String::from("[edit] ");
        s.push_str(&self.prompt);
        s.extend(self.buffer[..self.cursor].iter());
        s.push('▌');
        s.extend(self.buffer[self.cursor..].iter());
        s
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
    use super::{word_left, word_right, EditingState};

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
}
