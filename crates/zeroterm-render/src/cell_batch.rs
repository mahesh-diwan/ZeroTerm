//! Pure screen → `CellData` batch building.
//!
//! Carved out of `Renderer::update_cell_data` so the mapping rules — scrollback
//! row math, capacity clamping, cursor/selection highlighting, syntax colors,
//! block-divider overlays, attribute bit packing — are testable CPU transforms.
//! Nothing here touches wgpu: the only dependency is a glyph-provider closure
//! the caller supplies, so tests inject a fake and never need a GPU.

use bytemuck::Zeroable;
use zeroterm_core::cell::{CursorShape, UnderlineStyle};
use zeroterm_core::screen::Screen;

use crate::renderer::{CellData, Selection};
use crate::theme::Theme;

pub(crate) const ATTR_HAS_IMAGE: u32 = 0x400;
pub(crate) const ATTR_BLOCK_DIVIDER: u32 = 0x800;
pub(crate) const ATTR_DIM: u32 = 0x10;

const COPY_MARKER: &str = "[copy]";

/// Everything the shader needs to place one glyph, decoupled from the atlas
/// (which lives behind the caller's closure) so `CellBatch` stays GPU-free.
#[derive(Clone, Copy)]
pub(crate) struct GlyphQuad {
    pub(crate) uv_min: [f32; 2],
    pub(crate) uv_max: [f32; 2],
    pub(crate) size: [f32; 2],
    pub(crate) offset: [f32; 2],
}

pub(crate) struct CellBatch;

impl CellBatch {
    /// Clamp screen dimensions to a GPU storage-buffer capacity (window-sized).
    /// The one place this rule lives; both the batch builder and render_screen's
    /// instance count go through it so they can never disagree.
    pub(crate) fn clamp_dims(rows: usize, cols: usize, capacity: usize) -> (usize, usize) {
        let capacity = capacity.max(1);
        let mut rows = rows;
        let mut cols = cols;
        if rows.saturating_mul(cols) > capacity {
            rows = (capacity / cols.max(1)).min(rows);
            cols = (capacity / rows.max(1)).min(cols);
        }
        (rows, cols)
    }

    /// Build the GPU cell batch for a view of `screen` at `scroll_offset` with
    /// an optional selection. `capacity` is the GPU storage buffer's cell
    /// count (window-sized); the batch is clamped so a resize race or split
    /// pane math can never overrun it. `glyphs` maps a character to its quad.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        screen: &Screen,
        scroll_offset: usize,
        selection: Option<Selection>,
        capacity: usize,
        dirty_cells: &[(usize, usize)],
        blink_visible: bool,
        opacity: f32,
        theme: &Theme,
        mut glyphs: impl FnMut(char) -> GlyphQuad,
    ) -> Vec<CellData> {
        let buffer = screen.buffer();
        let (visible_rows, cols) = Self::clamp_dims(
            buffer.len(),
            buffer.first().map_or(0, Vec::len),
            capacity,
        );

        let cursor = screen.cursor();
        let cursor_col = cursor.col;
        let cursor_visible = cursor.visible;
        let cursor_shape = cursor.shape;

        let scrollback = screen.scrollback();
        let total_scrollback = scrollback.len();
        let total_rows = total_scrollback + visible_rows;

        let end = total_rows.saturating_sub(scroll_offset);
        let start = end.saturating_sub(visible_rows);

        // ponytail: block start_line is buffer-local; divider rows only line up
        // with view rows while scroll_offset == 0. Scrolled dividers are skipped.
        let mut divider_rows = std::collections::HashSet::new();
        let mut divider_meta: std::collections::HashMap<usize, Vec<char>> =
            std::collections::HashMap::new();
        for block in screen.blocks() {
            divider_rows.insert(block.start_line);
            divider_meta.insert(
                block.start_line,
                screen.block_metadata(block).chars().collect(),
            );
        }

        let mut batch = vec![CellData::zeroed(); visible_rows * cols];
        for &(dirty_row, dirty_col) in dirty_cells {
            if dirty_row >= visible_rows || dirty_col >= cols {
                continue;
            }

            let combined_idx = start + dirty_row;
            let line = if combined_idx < total_scrollback {
                &scrollback[total_scrollback - 1 - combined_idx]
            } else {
                &buffer[combined_idx - total_scrollback]
            };

            if dirty_col >= line.len() {
                continue;
            }

            let cell = &line[dirty_col];

            let mut fg = theme.map_cell_color(cell.fg);
            let bg = theme.map_cell_color(cell.bg);

            // Syntax classes are tagged into cells at write time (see Screen),
            // so scrollback rows carry their colors too — no scroll_offset gate.
            let mut cell_attrs = cell.attrs;
            if cell.syntax_color != 0 {
                if let Some(c) = highlight_color(cell.syntax_color, theme) {
                    fg = c;
                }
                if cell.syntax_color == zeroterm_core::highlight::HL_URL {
                    cell_attrs.underline = UnderlineStyle::Single;
                }
            }
            // OSC 8 hyperlinks render like URLs (accent color + underline).
            if cell.link_id != 0 {
                fg = theme.accent;
                cell_attrs.underline = UnderlineStyle::Single;
            }

            let is_cursor_cell = cursor_visible
                && blink_visible
                && scroll_offset == 0
                && dirty_row == cursor.row
                && dirty_col == cursor_col;
            let (fg, bg, cell_attrs) = if is_cursor_cell {
                match cursor_shape {
                    CursorShape::Block => (bg, fg, cell_attrs),
                    CursorShape::Underline => {
                        let mut a = cell_attrs;
                        a.underline = UnderlineStyle::Single;
                        (fg, bg, a)
                    }
                    CursorShape::Bar => (fg, bg, cell_attrs),
                }
            } else {
                (fg, bg, cell_attrs)
            };

            let is_selected = selection.is_some_and(|s| s.contains(combined_idx, dirty_col));

            let mut attrs = (cell_attrs.bold as u32)
                | ((cell_attrs.italic as u32) << 1)
                | (((cell_attrs.underline != UnderlineStyle::None) as u32) << 2)
                | ((cell_attrs.strikethrough as u32) << 3)
                | ((cell_attrs.dim as u32) << 4)
                | ((cell_attrs.blink as u32) << 5)
                | ((cell_attrs.reverse as u32) << 6)
                | ((cell_attrs.invisible as u32) << 7)
                | (if is_cursor_cell && matches!(cursor_shape, CursorShape::Bar) {
                    0x100u32
                } else {
                    0
                })
                | (if is_selected { 0x200u32 } else { 0 });
            if screen
                .image_cells()
                .contains_key(&(combined_idx, dirty_col))
            {
                attrs |= ATTR_HAS_IMAGE;
            }

            let fg_color = [
                fg.r as f32 / 255.0,
                fg.g as f32 / 255.0,
                fg.b as f32 / 255.0,
                1.0,
            ];
            let bg_color = [
                bg.r as f32 / 255.0,
                bg.g as f32 / 255.0,
                bg.b as f32 / 255.0,
                // Background carries the window opacity: the shader mixes
                // glyph alpha between bg (a=opacity) and fg (a=1), so text
                // stays opaque while the terminal background shows the
                // desktop through at (1-opacity).
                opacity,
            ];

            let mut ch = cell.ch;
            // block.start_line is buffer-local; view row == buffer row only at
            // scroll_offset 0, so scrolled dividers are skipped entirely (the
            // [copy]/metadata overlay would land on the wrong row otherwise).
            if scroll_offset == 0 && divider_rows.contains(&dirty_row) {
                attrs |= ATTR_BLOCK_DIVIDER;
                let meta = divider_meta.get(&dirty_row);
                let meta_len = meta.map_or(0, Vec::len);
                let copy_start = cols.saturating_sub(COPY_MARKER.len());
                let meta_start = copy_start.saturating_sub(meta_len);
                let overlay = if dirty_col >= copy_start {
                    COPY_MARKER
                        .as_bytes()
                        .get(dirty_col - copy_start)
                        .map(|&b| b as char)
                } else if dirty_col >= meta_start {
                    meta.and_then(|m| m.get(dirty_col - meta_start)).copied()
                } else {
                    None
                };
                if let Some(c) = overlay {
                    ch = c;
                    attrs |= ATTR_DIM;
                }
            }

            let g = glyphs(ch);
            batch[dirty_row * cols + dirty_col] = CellData {
                glyph_uv_min: g.uv_min,
                glyph_uv_max: g.uv_max,
                glyph_size: g.size,
                glyph_offset: g.offset,
                fg: fg_color,
                bg: bg_color,
                attrs,
                _pad1: [0; 3],
            };
        }

        batch
    }
}

/// Palette for `highlight` classes, mapped through the active theme's ANSI colors.
fn highlight_color(idx: u8, theme: &Theme) -> Option<zeroterm_core::cell::Color> {
    match idx {
        zeroterm_core::highlight::HL_KEYWORD => Some(theme.ansi[6]),
        zeroterm_core::highlight::HL_STRING => Some(theme.ansi[3]),
        zeroterm_core::highlight::HL_NUMBER => Some(theme.ansi[5]),
        zeroterm_core::highlight::HL_COMMENT => Some(theme.ansi[8]),
        zeroterm_core::highlight::HL_URL => Some(theme.accent),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::screen::Screen;

    fn fill(screen: &mut Screen, rows: &[&str]) {
        for (i, s) in rows.iter().enumerate() {
            screen.cursor_pos(i + 1, 1);
            for ch in s.chars() {
                screen.put_char(ch);
            }
        }
    }

    fn all_dirty(screen: &Screen) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for r in 0..screen.size().rows {
            for c in 0..screen.size().cols {
                v.push((r, c));
            }
        }
        v
    }

    /// Probe glyphs: a cell's char becomes the quad's size, so a batch can be
    /// read back as text. Spaces get a zero-size quad (no ink).
    fn probe_glyphs(ch: char) -> GlyphQuad {
        let sz = if ch == ' ' { 0.0 } else { ch as u32 as f32 };
        GlyphQuad {
            uv_min: [0.0, 0.0],
            uv_max: [0.0, 0.0],
            size: [sz, 1.0],
            offset: [0.0, 0.0],
        }
    }

    fn batch_text(batch: &[CellData], cols: usize, rows: usize) -> Vec<String> {
        let mut out = Vec::new();
        for r in 0..rows {
            let mut s = String::new();
            for c in 0..cols {
                let sz = batch[r * cols + c].glyph_size[0];
                s.push(if sz == 0.0 { ' ' } else { sz as u8 as char });
            }
            out.push(s);
        }
        out
    }

    #[test]
    fn scroll_offset_maps_scrollback_rows_into_the_view() {
        // 4 cols x 3 rows; scroll one line into scrollback, then view at
        // offset 1 must show [A] (scrollback), B, C top to bottom.
        let mut screen = Screen::new(4, 3);
        fill(&mut screen, &["A", "B", "C"]);
        screen.scroll_up(1); // scrollback = [A]; buffer = B, C, blank

        let batch = CellBatch::build(
            &screen,
            1,
            None,
            12,
            &all_dirty(&screen),
            true,
            1.0,
            &Theme::tokyo_night(),
            probe_glyphs,
        );
        let text = batch_text(&batch, 4, 3);
        assert_eq!(text[0].trim_end(), "A", "scrollback row must map to view row 0");
        assert_eq!(text[1].trim_end(), "B");
        assert_eq!(text[2].trim_end(), "C");
    }

    #[test]
    fn capacity_clamp_never_overruns_the_buffer() {
        // Window-sized storage of 4 cells vs a 4x3 screen: batch must clamp
        // to exactly capacity (1 row x 4 cols), not panic or overrun.
        let mut screen = Screen::new(4, 3);
        fill(&mut screen, &["A", "B", "C"]);
        let batch = CellBatch::build(
            &screen,
            0,
            None,
            4,
            &all_dirty(&screen),
            true,
            1.0,
            &Theme::tokyo_night(),
            probe_glyphs,
        );
        assert_eq!(batch.len(), 4);
    }

    #[test]
    fn selection_highlights_contained_cells_only() {
        let mut screen = Screen::new(4, 3);
        fill(&mut screen, &["AB", "CD", "EF"]);
        let sel = Selection {
            start_row: 1,
            start_col: 0,
            end_row: 1,
            end_col: 1,
            active: true,
        };
        let batch = CellBatch::build(
            &screen,
            0,
            Some(sel),
            12,
            &all_dirty(&screen),
            true,
            1.0,
            &Theme::tokyo_night(),
            probe_glyphs,
        );
        // Selection covers row 1, cols 0-1 (the 'C','D' row).
        assert_eq!(batch[0].attrs & 0x200, 0, "row0 col0 outside selection");
        assert_eq!(batch[1].attrs & 0x200, 0, "row0 col1 outside selection");
        assert_eq!(batch[4].attrs & 0x200, 0x200, "row1 col0 selected");
        assert_eq!(batch[5].attrs & 0x200, 0x200, "row1 col1 selected");
        assert_eq!(batch[6].attrs & 0x200, 0, "row1 col2 outside selection");
        assert_eq!(batch[8].attrs & 0x200, 0, "row2 col0 outside selection");
    }

    #[test]
    fn dim_attr_survives_into_the_batch() {
        let mut screen = Screen::new(4, 3);
        screen.cursor_pos(1, 1);
        screen.set_dim(true);
        screen.put_char('X');
        let batch = CellBatch::build(
            &screen,
            0,
            None,
            12,
            &all_dirty(&screen),
            true,
            1.0,
            &Theme::tokyo_night(),
            probe_glyphs,
        );
        // ATTR_DIM = bit 4
        assert_eq!(batch[0].attrs & ATTR_DIM, ATTR_DIM);
    }
}
