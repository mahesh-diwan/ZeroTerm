//! Window geometry + hit-testing, carved out of main.rs. Every pixel →
//! (pane / tab / divider / cell) mapping used to re-derive the tab-bar and
//! status-bar geometry in five separate methods; `Layout` owns the geometry
//! and the tab-strip layout contract (which must match `draw_tab_bar` exactly,
//! or hover and clicks land on the wrong cells).

use zeroterm_core::screen::Screen;
use zeroterm_mux::split::SplitDir;
use zeroterm_render::tab_span;

/// Geometry for one window: cell size plus the chrome bar heights.
pub(crate) struct Layout {
    cell_w: f32,
    cell_h: f32,
    tab_h: f32,
    status_h: f32,
}

impl Layout {
    pub(crate) fn new(cell: [f32; 2], tab_h: f32, status_h: f32) -> Self {
        Self {
            cell_w: cell[0],
            cell_h: cell[1],
            tab_h,
            status_h,
        }
    }

    pub(crate) fn tab_h(&self) -> f32 {
        self.tab_h
    }

    /// Content-area height in a window of `win_h` px (never negative).
    pub(crate) fn content_h(&self, win_h: f32) -> f32 {
        (win_h - self.tab_h - self.status_h).max(0.0)
    }

    /// Normalized content coordinates for a window point, or None when the
    /// point is over the tab/status bars or outside the content area.
    pub(crate) fn content_normalized(
        &self,
        x: f32,
        y: f32,
        win_w: f32,
        win_h: f32,
    ) -> Option<(f32, f32)> {
        let content_h = self.content_h(win_h);
        if content_h <= 0.0 || y < self.tab_h || y >= self.tab_h + content_h {
            return None;
        }
        Some((x / win_w, (y - self.tab_h) / content_h))
    }

    /// Pane whose tab is under a window-space point, or None. `tabs` is the
    /// sorted (pane id, title) list. Layout must match draw_tab_bar: starts at
    /// col 1, span = truncated title + 2 padding cells, col += span + 1.
    pub(crate) fn tab_at(&self, x: f32, y: f32, tabs: &[(usize, String)]) -> Option<usize> {
        if y < 0.0 || y >= self.tab_h || tabs.is_empty() {
            return None;
        }
        let mut col = 1usize;
        for (id, title) in tabs {
            let span = tab_span(title, 20);
            let start = col as f32 * self.cell_w;
            if x >= start && x < (col + span) as f32 * self.cell_w {
                return Some(*id);
            }
            col += span + 1;
        }
        None
    }

    /// Like `tab_at`, plus whether the point is over the tab's close button
    /// (the right padding cell of the span).
    pub(crate) fn tab_bar_hover(
        &self,
        x: f32,
        y: f32,
        tabs: &[(usize, String)],
    ) -> Option<(usize, bool)> {
        if y < 0.0 || y >= self.tab_h || tabs.is_empty() {
            return None;
        }
        let mut col = 1usize;
        for (id, title) in tabs {
            let span = tab_span(title, 20);
            let start = col as f32 * self.cell_w;
            if x >= start && x < (col + span) as f32 * self.cell_w {
                let close_start = (col + span - 1) as f32 * self.cell_w;
                return Some((*id, x >= close_start));
            }
            col += span + 1;
        }
        None
    }

    /// Pane divider near a window point (within `tolerance` px), with its
    /// direction. `dividers` is the session's normalized-boundary list.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn divider_at(
        &self,
        x: f32,
        y: f32,
        tolerance: f32,
        win_w: f32,
        win_h: f32,
        has_other_leaves: bool,
        dividers: &[(SplitDir, f32, usize)],
    ) -> Option<(usize, SplitDir)> {
        if !has_other_leaves || y < self.tab_h {
            return None;
        }
        let content_h = self.content_h(win_h);
        for (dir, boundary, target) in dividers {
            let (px, py) = match dir {
                SplitDir::Vertical => (boundary * win_w, y),
                SplitDir::Horizontal => (x, self.tab_h + boundary * content_h),
            };
            let dx = (px - x).abs();
            let dy = (py - y).abs();
            let hit = match dir {
                SplitDir::Vertical => dx <= tolerance && y >= self.tab_h,
                SplitDir::Horizontal => dy <= tolerance,
            };
            if hit {
                return Some((*target, *dir));
            }
        }
        None
    }

    /// Global (scrollback-aware) cell under a window point within a pane's
    /// pixel rect, or None when outside the rect or past the buffer.
    pub(crate) fn screen_to_cell(
        &self,
        x: f32,
        y: f32,
        rect_px: (f32, f32, f32, f32),
        screen: &Screen,
        scroll_offset: usize,
    ) -> Option<(usize, usize)> {
        let (px, py, pw, ph) = rect_px;
        let (lx, ly) = (x - px, y - py);
        if lx < 0.0 || ly < 0.0 || lx >= pw || ly >= ph {
            return None;
        }
        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };
        let col = (lx / self.cell_w).floor() as usize;
        let row = (ly / self.cell_h).floor() as usize;
        if row >= visible_rows || col >= cols {
            return None;
        }
        let scrollback = screen.scrollback().len();
        let total_rows = scrollback + visible_rows;
        let end = total_rows.saturating_sub(scroll_offset);
        let start = end.saturating_sub(visible_rows);
        Some((start + row, col))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::screen::Screen;

    fn layout() -> Layout {
        // 9x19 cells, 2-row tab bar (38px), 1-row status bar (19px).
        Layout::new([9.0, 19.0], 38.0, 19.0)
    }

    fn tabs() -> Vec<(usize, String)> {
        vec![(1, "alpha".into()), (2, "beta".into())]
    }

    #[test]
    fn tab_at_matches_draw_tab_bar_layout() {
        let l = layout();
        let tabs = tabs();
        // tab "alpha": span 5+2=7 -> cells 1..8 -> x 9..72; the separator
        // cell 8 (x 72..81) belongs to no tab; tab "beta" starts at col 9.
        assert_eq!(l.tab_at(9.0, 10.0, &tabs), Some(1));
        assert_eq!(l.tab_at(71.0, 10.0, &tabs), Some(1));
        assert_eq!(l.tab_at(72.0, 10.0, &tabs), None); // separator cell
        assert_eq!(l.tab_at(81.0, 10.0, &tabs), Some(2));
        // beyond the last tab
        assert_eq!(l.tab_at(500.0, 10.0, &tabs), None);
        // below the tab bar
        assert_eq!(l.tab_at(9.0, 39.0, &tabs), None);
    }

    #[test]
    fn tab_bar_hover_flags_the_close_cell() {
        let l = layout();
        let tabs = tabs();
        // "alpha": span 7, close cell = col 1+7-1 = 7 -> x 63..72
        let (id, close) = l.tab_bar_hover(65.0, 10.0, &tabs).unwrap();
        assert_eq!(id, 1);
        assert!(close);
        let (_, close) = l.tab_bar_hover(20.0, 10.0, &tabs).unwrap();
        assert!(!close);
    }

    #[test]
    fn content_normalized_excludes_the_bars() {
        let l = layout();
        // y=38 is the first content pixel; y=37 is the last tab-bar pixel.
        assert!(l.content_normalized(50.0, 37.0, 200.0, 200.0).is_none());
        assert_eq!(l.content_normalized(50.0, 38.0, 200.0, 200.0), Some((0.25, 0.0)));
    }

    #[test]
    fn screen_to_cell_maps_through_scrollback() {
        let l = layout();
        let mut screen = Screen::new(4, 3);
        screen.cursor_pos(1, 1);
        screen.put_char('A');
        screen.scroll_up(1); // scrollback = [A]; buffer rows blank
        // Pane rect at px (0, 38), 36x57 (4 cols x 3 rows at 9x19).
        // Global row for view row 0 with scroll_offset 1 = scrollback row 0.
        let (row, col) = l.screen_to_cell(4.0, 38.0, (0.0, 38.0, 36.0, 57.0), &screen, 1).unwrap();
        assert_eq!(col, 0);
        assert_eq!(row, 0); // scrollback index 0
    }
}
