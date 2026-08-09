//! Shared snapshot/restore support for overlays that draw destructively into
//! the terminal screen buffer (settings menu, search bar, SSH host picker, AI
//! panel). Each overlay used to hand-roll the same save -> draw -> restore
//! bookkeeping; `ScreenScratch` owns it so overlays keep only their content.

use zeroterm_core::cell::{Cell, Cursor};
use zeroterm_core::screen::Screen;

/// Lifecycle contract shared by every screen-drawn overlay (search bar,
/// settings menu, AI panel, SSH host picker). Each overlay owns its content
/// and state; this trait is the narrow seam App uses to draw/restore any of
/// them uniformly. `ScreenScratch` implements the save/restore backing.
///
/// Method names deliberately differ from each overlay's inherent methods
/// (`overlay_bytes` / `save_screen` / `restore_screen`) so a trait impl can
/// delegate to the inherent one without recursing, and so the trait keeps
/// working when a feature (e.g. ssh) compiles some inherent methods out.
pub trait Overlay {
    /// Whether the overlay is currently open (owns the screen region).
    fn is_open(&self) -> bool;
    /// CSI bytes that paint the overlay into a `cols x rows` screen.
    fn draw_bytes(&self, cols: usize, rows: usize) -> Vec<u8>;
    /// Snapshot the covered region before drawing (via ScreenScratch).
    fn snapshot(&mut self, screen: &Screen);
    /// Restore the covered region after closing (via ScreenScratch).
    fn restore(&mut self, screen: &mut Screen);
}

/// A saved screen region: the exact cells (and cursor) an overlay overwrites,
/// restored verbatim when the overlay closes.
#[derive(Debug, Default)]
pub struct ScreenScratch {
    saved: Option<Scratch>,
}

#[derive(Debug)]
struct Scratch {
    rows: Vec<Vec<Cell>>,
    top: usize,
    cursor: Cursor,
}

impl ScreenScratch {
    /// Snapshot `height` screen rows starting at `top`, plus the cursor.
    pub fn save_region(&mut self, screen: &Screen, top: usize, height: usize) {
        let buf = screen.buffer();
        self.saved = Some(Scratch {
            rows: (0..height)
                .map(|i| buf.get(top + i).cloned().unwrap_or_default())
                .collect(),
            top,
            cursor: screen.cursor(),
        });
    }

    /// Restore the snapshot (rows + cursor) into `screen`, then clear it.
    /// No-op when nothing has been saved.
    pub fn restore(&mut self, screen: &mut Screen) {
        if let Some(saved) = self.saved.take() {
            for (i, row) in saved.rows.iter().enumerate() {
                screen.set_cells(saved.top + i, row);
            }
            screen.cursor_pos(saved.cursor.row + 1, saved.cursor.col + 1);
            screen.set_cursor_visible(saved.cursor.visible);
        }
    }

    /// Whether a region is currently saved (i.e. an overlay is covering it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_active(&self) -> bool {
        self.saved.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::cell::Cell;

    fn row_of(screen: &Screen, row: usize) -> String {
        screen
            .buffer()
            .get(row)
            .map(|cells| cells.iter().map(|c| c.ch).collect::<String>())
            .unwrap_or_default()
            .trim_end()
            .to_string()
    }

    #[test]
    fn save_restore_region_round_trips_cells_and_cursor() {
        let mut screen = Screen::new(80, 10);
        for (i, text) in ["alpha", "beta"].iter().enumerate() {
            screen.set_cells(2 + i, &text.chars().map(Cell::new).collect::<Vec<_>>());
        }
        screen.cursor_pos(4, 3); // 1-based -> 0-based (3, 2)

        let mut scratch = ScreenScratch::default();
        scratch.save_region(&screen, 2, 2);
        assert!(scratch.is_active());

        // Overwrite the covered region (as an overlay would).
        screen.set_cells(2, &"XXXXX".chars().map(Cell::new).collect::<Vec<_>>());
        screen.set_cells(3, &"YYYYY".chars().map(Cell::new).collect::<Vec<_>>());

        scratch.restore(&mut screen);
        assert!(!scratch.is_active());
        assert_eq!(row_of(&screen, 2), "alpha");
        assert_eq!(row_of(&screen, 3), "beta");
        assert_eq!(screen.cursor().row, 3);
        assert_eq!(screen.cursor().col, 2);
    }

    #[test]
    fn restore_without_save_is_noop() {
        let mut screen = Screen::new(80, 10);
        let mut scratch = ScreenScratch::default();
        scratch.restore(&mut screen); // must not panic
        assert!(!scratch.is_active());
    }

    #[test]
    fn save_clamps_out_of_bounds_rows_to_blank() {
        let mut screen = Screen::new(4, 4);
        screen.set_cells(0, &"ab".chars().map(Cell::new).collect::<Vec<_>>());
        let mut scratch = ScreenScratch::default();
        // top=3, height=5 goes past the 4-row screen; extra rows become blank.
        scratch.save_region(&screen, 3, 5);
        scratch.restore(&mut screen);
        assert_eq!(row_of(&screen, 0), "ab");
    }
}
