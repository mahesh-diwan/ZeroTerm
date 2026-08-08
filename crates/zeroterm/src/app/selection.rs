//! Pure selection math: rect normalization and text extraction.
//!
//! The old `copy_selection` welded 49 lines of scrollback-index math into an
//! App method that could never be tested (it needed a live pane + screen). The
//! screen itself is just data — a scrollback deque + a visible buffer — so the
//! extraction is a pure function over `&Screen`, unit-testable without a PTY
//! or a wgpu device. `normalize` also owns the reversed-rect contract the
//! renderer's `Selection::contains` and the drag-copy check rely on.

use zeroterm_core::screen::Screen;
use zeroterm_render::Selection;

/// Normalize a selection rect so start <= end (both axes), matching the
/// copy path and `Selection::contains`.
pub fn normalize(sel: &Selection) -> (usize, usize, usize, usize) {
    if sel.start_row < sel.end_row
        || (sel.start_row == sel.end_row && sel.start_col <= sel.end_col)
    {
        (sel.start_row, sel.start_col, sel.end_row, sel.end_col)
    } else {
        (sel.end_row, sel.end_col, sel.start_row, sel.start_col)
    }
}

/// The copied text for a selection over a screen's scrollback + visible
/// buffer. Global row 0 = top of scrollback (newest at the front of the
/// scrollback deque, mirroring `Screen`'s layout). Line ends join with '\n'.
/// The caller trims trailing whitespace (the terminal renders trailing pads).
pub fn selection_text(sel: &Selection, screen: &Screen) -> String {
    let scrollback = screen.scrollback();
    let buffer = screen.buffer();
    let visible_rows = buffer.len();
    let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

    let (start_row, start_col, end_row, end_col) = normalize(sel);
    let mut text = String::new();
    let total_scrollback = scrollback.len();
    let total_rows = total_scrollback + visible_rows;

    for r in start_row..=end_row.min(total_rows.saturating_sub(1)) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::Parser;

    /// Parser whose visible buffer holds the given rows (one per line, LF/CRLF
    /// separated). The screen scrolls into scrollback only when the viewport
    /// is at least 2 rows tall (a 1-row screen never meets `top < bottom`).
    fn parser_with(rows: usize, text: &[&str]) -> Parser {
        let cols = text.iter().map(|r| r.chars().count()).max().unwrap_or(0).max(1);
        let mut parser = Parser::new(cols, rows.max(2));
        let mut bytes: Vec<u8> = Vec::new();
        for (i, row) in text.iter().enumerate() {
            if i > 0 {
                bytes.push(b'\r');
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(row.as_bytes());
        }
        parser.parse(&bytes);
        parser
    }

    /// Build a screen with the given visible rows, then shrink it to `rows`:
    /// the lines that fall off the top land in scrollback exactly as they do
    /// on a real window shrink (newest at the front).
    fn screen_with_scrollback(all: &[&str], visible: usize) -> Parser {
        let mut parser = parser_with(all.len().max(2), all);
        let cols = parser.screen().size().cols;
        parser.screen_mut().resize(cols, visible);
        parser
    }

    #[test]
    fn single_row_selection() {
        let parser = parser_with(1, &["hello"]);
        let sel = Selection {
            start_row: 0,
            start_col: 1,
            end_row: 0,
            end_col: 3,
            active: true,
        };
        assert_eq!(selection_text(&sel, parser.screen()), "ell");
    }

    #[test]
    fn multi_row_selection_joins_with_newline() {
        let parser = parser_with(2, &["abc", "def"]);
        let sel = Selection {
            start_row: 0,
            start_col: 1,
            end_row: 1,
            end_col: 2,
            active: true,
        };
        assert_eq!(selection_text(&sel, parser.screen()), "bc\ndef");
    }

    #[test]
    fn reversed_selection_normalizes() {
        let parser = parser_with(2, &["abc", "def"]);
        let sel = Selection {
            start_row: 1,
            start_col: 2,
            end_row: 0,
            end_col: 1,
            active: true,
        };
        assert_eq!(selection_text(&sel, parser.screen()), "bc\ndef");
    }

    #[test]
    fn scrollback_rows_map_in_order() {
        // Three visible rows shrink to one: line0+line1 scroll off, newest at
        // the front (line1), and "visible" is the sole remaining row.
        let parser = screen_with_scrollback(&["line0", "line1", "visible"], 1);
        let sel = Selection {
            start_row: 0,
            start_col: 0,
            end_row: 2,
            end_col: 6,
            active: true,
        };
        // Rows are padded to the screen width (7 = "visible"); the caller
        // trims trailing whitespace before copying, matching legacy behavior.
        assert_eq!(selection_text(&sel, parser.screen()), "line0  \nline1  \nvisible");
    }

    #[test]
    fn selection_beyond_buffer_clamps() {
        let parser = parser_with(2, &["abc", "def"]);
        let sel = Selection {
            start_row: 0,
            start_col: 0,
            end_row: 99,
            end_col: 99,
            active: true,
        };
        // Clamps to the last real row; no panic, no garbage rows. Trailing
        // newline matches the legacy behavior (the caller trims before copy).
        assert_eq!(selection_text(&sel, parser.screen()), "abc\ndef\n");
    }

    #[test]
    fn normalize_flips_reversed_rects() {
        let sel = Selection {
            start_row: 5,
            start_col: 3,
            end_row: 2,
            end_col: 1,
            active: true,
        };
        assert_eq!(normalize(&sel), (2, 1, 5, 3));
    }

    #[test]
    fn normalize_keeps_forward_rects() {
        let sel = Selection {
            start_row: 1,
            start_col: 1,
            end_row: 4,
            end_col: 0,
            active: true,
        };
        assert_eq!(normalize(&sel), (1, 1, 4, 0));
    }
}
