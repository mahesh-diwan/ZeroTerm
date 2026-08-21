//! Search overlay: bottom-bar prompt that scans scrollback + visible buffer
//! and jumps the viewport to each match (Ctrl+Shift+F).
//!
//! The bar is drawn into the active pane's parser screen buffer via synthetic
//! CSI sequences. Because that is destructive to the cells it overwrites, the
//! covered region is snapshotted on open and restored on close (same pattern
//! as the settings overlay).

use zeroterm_core::screen::Screen;
use zeroterm_render::SearchMatch;

use crate::overlay::{Overlay, ScreenScratch};

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Every in-buffer occurrence of `query`: global row + column span (0 =
    /// top of scrollback). The renderer tints these in place while search is
    /// open (kitty-style), so the user sees all matches, not just the viewport
    /// jump.
    pub matches: Vec<SearchMatch>,
    /// Index into `matches` of the currently highlighted match.
    pub current: usize,
    /// Snapshot of the covered (bottom) screen row, restored on close.
    scratch: ScreenScratch,
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
    /// Records every occurrence (kitty highlights all of them in place), not
    /// just the first per row: a row with three hits contributes three spans.
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
            let line = self.row_text(screen, r).to_lowercase();
            let mut search_from = 0;
            while let Some(rel) = line[search_from..].find(&q) {
                let start = search_from + rel;
                self.matches.push(SearchMatch {
                    row: r,
                    start,
                    end: start + q.len(),
                });
                // Advance past this occurrence (a zero-length query already
                // bailed above, so this always progresses).
                search_from = start + q.len().max(1);
                if start >= line.len() {
                    break;
                }
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
        self.matches.get(self.current).map(|m| m.row)
    }

    /// Current match + its index, for the renderer's in-place highlight.
    /// None while search is closed or nothing matches.
    pub fn highlight(&self) -> Option<(&[SearchMatch], usize)> {
        if self.open && !self.query.is_empty() && !self.matches.is_empty() {
            Some((&self.matches, self.current))
        } else {
            None
        }
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
        self.scratch.save_region(screen, rows.saturating_sub(1), 1);
    }

    pub fn restore_screen(&mut self, screen: &mut Screen) {
        self.scratch.restore(screen);
    }
}

impl Overlay for SearchState {
    fn is_open(&self) -> bool {
        self.open
    }
    fn draw_bytes(&self, cols: usize, rows: usize) -> Vec<u8> {
        self.overlay_bytes(cols, rows)
    }
    fn snapshot(&mut self, screen: &Screen) {
        self.save_screen(screen);
    }
    fn restore(&mut self, screen: &mut Screen) {
        self.restore_screen(screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::cell::Cell;
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
        assert_eq!(
            s.matches,
            vec![
                SearchMatch {
                    row: 0,
                    start: 0,
                    end: 5
                },
                SearchMatch {
                    row: 2,
                    start: 0,
                    end: 5
                },
            ]
        );
        assert_eq!(s.current_row(), Some(0));
        assert!(s.next());
        assert_eq!(s.current_row(), Some(2));
        assert!(s.next());
        assert_eq!(s.current_row(), Some(0));
    }

    #[test]
    fn find_records_every_occurrence_per_row() {
        let mut s = SearchState::default();
        let screen = screen_with(&["aa aa aa", "bbb", "/ search bar"]);
        s.query = "aa".into();
        s.find(&screen);
        // Three occurrences in row 0 at cols 0, 3, 6; nothing in row 1.
        assert_eq!(
            s.matches,
            vec![
                SearchMatch {
                    row: 0,
                    start: 0,
                    end: 2
                },
                SearchMatch {
                    row: 0,
                    start: 3,
                    end: 5
                },
                SearchMatch {
                    row: 0,
                    start: 6,
                    end: 8
                },
            ]
        );
        assert_eq!(s.matches.len(), 3);
        // highlight() is the render gate: it requires the overlay to be open.
        assert!(s.highlight().is_none());
        s.open = true;
        assert!(s.highlight().is_some());
        assert_eq!(s.highlight().unwrap().1, 0);
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
