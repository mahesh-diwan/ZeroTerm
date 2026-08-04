//! Search overlay: bottom-bar prompt that scans scrollback + visible buffer
//! and jumps the viewport to each match (Ctrl+Shift+F).
//!
//! The bar is drawn into the active pane's parser screen buffer via synthetic
//! CSI sequences. Because that is destructive to the cells it overwrites, the
//! covered region is snapshotted on open and restored on close (same pattern
//! as the settings overlay).

use zeroterm_core::cell::{Cell, Cursor};
use zeroterm_core::screen::Screen;

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Global row indices (0 = top of scrollback) that match `query`.
    pub matches: Vec<usize>,
    /// Index into `matches` of the currently highlighted match.
    pub current: usize,
    saved_cells: Option<Vec<Cell>>,
    saved_cursor: Option<Cursor>,
}

impl SearchState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.matches.clear();
            self.current = 0;
        }
    }

    pub fn backspace(&mut self) {
        self.query.pop();
    }

    pub fn append(&mut self, c: char) {
        self.query.push(c);
    }

    /// Re-scan the screen for `query` and point `current` at the first match.
    pub fn find(&mut self, screen: &Screen) {
        self.matches.clear();
        self.current = 0;
        if self.query.is_empty() {
            return;
        }
        let q = self.query.to_lowercase();
        // Exclude the last row: the search bar itself is drawn there.
        let total_rows = (screen.scrollback().len() + screen.buffer().len()).saturating_sub(1);
        for r in 0..total_rows {
            let line = self.row_text(screen, r);
            if line.to_lowercase().contains(&q) {
                self.matches.push(r);
            }
        }
    }

    pub fn next(&mut self) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        self.current = (self.current + 1) % self.matches.len();
        true
    }

    pub fn prev(&mut self) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        true
    }

    /// Global row of the currently highlighted match, if any.
    pub fn current_row(&self) -> Option<usize> {
        self.matches.get(self.current).copied()
    }

    fn row_text(&self, screen: &Screen, global_row: usize) -> String {
        let scrollback = screen.scrollback();
        let total = scrollback.len();
        let row = if global_row < total {
            scrollback.get(total - 1 - global_row)
        } else {
            screen.buffer().get(global_row - total)
        };
        row.map(|line| line.iter().map(|c| c.ch).collect())
            .unwrap_or_default()
    }

    pub fn overlay_bytes(&self, cols: usize, rows: usize) -> Vec<u8> {
        let count = self.matches.len();
        let pos = if count == 0 { 0 } else { self.current + 1 };
        let line = format!(
            " / {}   {}/{}   esc: close  enter/arrow: next  shift+enter: prev ",
            self.query, pos, count
        );
        let bar_bg = (40, 44, 52);
        let bar_fg = (197, 200, 198);
        let text: String = line.chars().take(cols).collect();
        let pad = cols.saturating_sub(text.chars().count());
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[?25l");
        out.extend_from_slice(format!("\x1b[{};1H", rows).as_bytes());
        out.extend_from_slice(
            format!(
                "\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m",
                bar_bg.0, bar_bg.1, bar_bg.2, bar_fg.0, bar_fg.1, bar_fg.2
            )
            .as_bytes(),
        );
        out.extend_from_slice(text.as_bytes());
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(b"\x1b[0m");
        out
    }

    pub fn save_screen(&mut self, screen: &Screen) {
        let rows = screen.size().rows;
        let buf = screen.buffer();
        self.saved_cells = Some(buf.get(rows - 1).cloned().unwrap_or_else(Vec::new));
        self.saved_cursor = Some(screen.cursor());
    }

    pub fn restore_screen(&mut self, screen: &mut Screen) {
        if let (Some(cells), Some(cursor)) = (&self.saved_cells, &self.saved_cursor) {
            let rows = screen.size().rows;
            screen.set_cells(rows - 1, cells);
            screen.cursor_pos(cursor.row + 1, cursor.col + 1);
            screen.set_cursor_visible(cursor.visible);
        }
        self.saved_cells = None;
        self.saved_cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::screen::Screen;

    fn screen_with(lines: &[&str]) -> Screen {
        let mut screen = Screen::new(80, lines.len());
        for (i, line) in lines.iter().enumerate() {
            screen.set_cells(i, &line.chars().map(Cell::new).collect::<Vec<_>>());
        }
        screen
    }

    #[test]
    fn find_scans_visible_buffer_and_tracks_matches() {
        let mut s = SearchState::default();
        // Last row is the search bar (excluded from the scan).
        let screen = screen_with(&["hello world", "goodbye", "HELLO there", "/ search bar"]);
        s.query = "hello".into();
        s.find(&screen);
        assert_eq!(s.matches, vec![0, 2]);
        assert_eq!(s.current_row(), Some(0));
        assert!(s.next());
        assert_eq!(s.current_row(), Some(2));
        assert!(s.next());
        assert_eq!(s.current_row(), Some(0));
    }

    #[test]
    fn empty_query_and_no_match() {
        let mut s = SearchState::default();
        let screen = screen_with(&["aaa", "bbb"]);
        s.query = "".into();
        s.find(&screen);
        assert!(s.matches.is_empty());
        s.query = "zzz".into();
        s.find(&screen);
        assert!(s.matches.is_empty());
        assert!(!s.next());
    }
}
