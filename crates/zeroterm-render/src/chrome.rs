//! Chrome geometry: the fixed tab-bar/status-bar rows and grid padding that
//! every size computation shares. Single source of truth for BOTH the spawn
//! estimate (app crate's `cells_for_size`) and the renderer layout
//! (`cols_for`/`rows_for`, `resize_panes_to_rects`), so the two can never
//! drift. A drift here used to make the PTY resize twice at startup and bash
//! reprint its prompt (the "prompt printed twice" bug class).

/// Chrome rows reserved above the grid: the tab bar. Hidden with a single
/// tab (kitty `tab_bar_min_tabs = 2`); `content_dims` takes the effective
/// count as a parameter.
pub const TAB_BAR_ROWS: usize = 1;
/// Chrome rows reserved below the grid: the status bar. Always present.
pub const STATUS_BAR_ROWS: usize = 1;
/// Grid padding in physical pixels, ordered [left, right, top, bottom].
pub const PADDING: [f32; 4] = [16.0, 16.0, 16.0, 16.0];

/// Columns that fit `width` px of usable (post-padding) space at `cell_w`.
/// Floor + clamp to >= 1, matching the old inline math exactly.
pub fn cols_for(cell_w: f32, width: f32) -> usize {
    let usable = width - PADDING[1] - PADDING[3];
    (usable / cell_w).floor().max(1.0) as usize
}

/// Rows that fit `height` px of usable (post-padding) space at `cell_h`.
/// Floor + clamp to >= 1, matching the old inline math exactly.
pub fn rows_for(cell_h: f32, height: f32) -> usize {
    let usable = height - PADDING[0] - PADDING[2];
    (usable / cell_h).floor().max(1.0) as usize
}

/// (cols, rows) for a window of `size` at `cell_w` x `cell_h`: chrome rows
/// (`tab_rows` + status bar) subtracted from the height, padding subtracted
/// from both axes, floor + clamp >= 1. This is the exact function the app
/// crate's `cells_for_size` used to inline — now shared so the spawn
/// estimate can never disagree with the renderer's own layout.
pub fn content_dims(
    cell_w: f32,
    cell_h: f32,
    tab_rows: usize,
    size: [f32; 2],
) -> (usize, usize) {
    let chrome = (tab_rows + STATUS_BAR_ROWS) as f32 * cell_h;
    let content_h = (size[1] - chrome).max(0.0);
    (cols_for(cell_w, size[0]), rows_for(cell_h, content_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cols_rows_match_padding_subtracted_math() {
        // 946x501 window at 10x22 cells with the tab bar visible (1 row):
        // chrome = (1 + 1) * 22 = 44; content_h = 501 - 44 = 457.
        // cols = (946 - 32) / 10 = 91.4 -> 91; rows = (457 - 32) / 22 = 19.3 -> 19.
        let (cols, rows) = content_dims(10.0, 22.0, 1, [946.0, 501.0]);
        assert_eq!(cols, 91);
        assert_eq!(rows, 19);
    }

    #[test]
    fn hidden_tab_bar_grants_an_extra_row() {
        // kitty tab_bar_min_tabs=2: with one tab (tab_rows=0) the grid gains
        // a row vs the same window with the bar visible.
        let (_, rows_hidden) = content_dims(10.0, 22.0, 0, [946.0, 501.0]);
        let (_, rows_visible) = content_dims(10.0, 22.0, 1, [946.0, 501.0]);
        assert_eq!(rows_hidden, rows_visible + 1);
    }

    #[test]
    fn tiny_window_clamps_to_one_cell() {
        assert_eq!(content_dims(10.0, 22.0, 1, [5.0, 5.0]), (1, 1));
        assert_eq!(cols_for(10.0, 0.0), 1);
        assert_eq!(rows_for(22.0, 0.0), 1);
    }

    #[test]
    fn content_dims_is_exactly_cols_for_plus_rows_for() {
        // The renderer's resize path computes cols/rows separately on the
        // post-chrome height; content_dims must equal that decomposition.
        let (cols, rows) = content_dims(9.0, 19.0, 0, [800.0, 600.0]);
        assert_eq!(cols, cols_for(9.0, 800.0));
        assert_eq!(rows, rows_for(19.0, 600.0 - 19.0)); // status bar only
    }
}
