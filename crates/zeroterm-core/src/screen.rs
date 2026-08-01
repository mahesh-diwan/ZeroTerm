//! Screen buffer with scrollback, cursor, and rendering support

use crate::cell::{Attributes, Cell, Color, Cursor, UnderlineStyle};
use crate::image_decode::{FrameData, MAX_ANIM_FRAMES};
use std::collections::{HashMap, VecDeque};
use unicode_width::UnicodeWidthChar;

const MAX_IMAGES: usize = 64;
const MAX_IMAGE_BYTES: usize = 100 << 20;

#[derive(Debug, Clone)]
pub struct CommandBlock {
    pub id: usize,
    pub start_line: usize,
    pub end_line: Option<usize>,
    pub command: String,
    pub exit_code: Option<i32>,
    pub timestamp: std::time::Instant,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// Frame 0 RGBA (back-compat; single-frame images like sixel).
    pub rgba_data: Vec<u8>,
    /// All decoded frames for animated images; empty for the fallback path.
    pub frames: Vec<FrameData>,
}

pub struct Screen {
    size: Size,
    buffer: Vec<Vec<Cell>>,
    alt_buffer: Option<Vec<Vec<Cell>>>,
    scrollback: VecDeque<Vec<Cell>>,
    scrollback_limit: usize,
    cursor: Cursor,
    saved_cursor: Option<Cursor>,
    attrs: Attributes,
    fg: Color,
    bg: Color,
    title: String,
    origin_mode: bool,
    autowrap: bool,
    insert_mode: bool,
    use_alt_screen: bool,
    cursor_keys_mode: bool,
    newline_mode: bool,
    keyboard_action_mode: bool,
    reverse_video: bool,
    scroll_top: usize,
    scroll_bottom: usize,
    auto_repeat: bool,
    mouse_tracking: bool,
    tabs: Vec<bool>,
    bell_callback: Option<Box<dyn Fn() + Send + Sync>>,
    #[allow(clippy::type_complexity)] // callback box type; a named alias adds noise
    title_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
    ident_callback: Option<Box<dyn Fn() + Send + Sync>>,
    status_callback: Option<Box<dyn Fn() + Send + Sync>>,
    cursor_callback: Option<Box<dyn Fn() + Send + Sync>>,
    blocks: Vec<CommandBlock>,
    block_id_counter: usize,
    current_block_command: String,
    pub image_registry: HashMap<u32, ImageData>,
    pub image_cells: HashMap<(usize, usize), u32>,
    next_image_id: u32,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut tabs = vec![false; cols];
        for i in (8..cols).step_by(8) {
            tabs[i] = true;
        }

        Self {
            size: Size { cols, rows },
            buffer: vec![vec![Cell::default(); cols]; rows],
            alt_buffer: None,
            scrollback: VecDeque::new(),
            scrollback_limit: 10000,
            cursor: Cursor::default(),
            saved_cursor: None,
            attrs: Attributes::default(),
            fg: Color::DEFAULT_FG,
            bg: Color::DEFAULT_BG,
            title: String::new(),
            origin_mode: false,
            autowrap: true,
            insert_mode: false,
            use_alt_screen: false,
            cursor_keys_mode: false,
            newline_mode: false,
            keyboard_action_mode: false,
            reverse_video: false,
            scroll_top: 0,
            scroll_bottom: 0,
            auto_repeat: true,
            mouse_tracking: false,
            tabs,
            bell_callback: None,
            title_callback: None,
            ident_callback: None,
            status_callback: None,
            cursor_callback: None,
            blocks: Vec::new(),
            block_id_counter: 0,
            current_block_command: String::new(),
            image_registry: HashMap::new(),
            image_cells: HashMap::new(),
            next_image_id: 0,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.size.cols && rows == self.size.rows {
            return;
        }

        let mut new_buffer = vec![vec![Cell::default(); cols]; rows];

        let row_start = self.size.rows.saturating_sub(rows);
        let col_end = cols.min(self.size.cols);

        for (r, row) in self.buffer.iter().skip(row_start).enumerate() {
            for (c, cell) in row.iter().take(col_end).enumerate() {
                new_buffer[r][c] = *cell;
            }
        }

        self.buffer = new_buffer;

        if let Some(ref mut alt) = self.alt_buffer {
            let mut new_alt = vec![vec![Cell::default(); cols]; rows];
            let row_start = self.size.rows.saturating_sub(rows);
            let col_end = cols.min(self.size.cols);

            for (r, row) in alt.iter().skip(row_start).enumerate() {
                for (c, cell) in row.iter().take(col_end).enumerate() {
                    new_alt[r][c] = *cell;
                }
            }
            *alt = new_alt;
        }

        let mut new_tabs = vec![false; cols];
        for i in (8..cols).step_by(8) {
            new_tabs[i] = true;
        }
        for (i, &tab) in self.tabs.iter().enumerate() {
            if i < cols {
                new_tabs[i] = tab;
            }
        }
        self.tabs = new_tabs;

        self.size = Size { cols, rows };
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.size.rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(self.size.cols.saturating_sub(1));
    }

    fn current_buffer(&self) -> &Vec<Vec<Cell>> {
        self.alt_buffer.as_ref().unwrap_or(&self.buffer)
    }

    fn current_buffer_mut(&mut self) -> &mut Vec<Vec<Cell>> {
        self.alt_buffer.as_mut().unwrap_or(&mut self.buffer)
    }

    pub fn set_cells(&mut self, row: usize, cells: &[Cell]) {
        let buffer = self.current_buffer_mut();
        if let Some(dst) = buffer.get_mut(row) {
            for (i, cell) in cells.iter().enumerate() {
                if let Some(c) = dst.get_mut(i) {
                    *c = *cell;
                }
            }
        }
    }

    pub fn buffer(&self) -> &[Vec<Cell>] {
        self.current_buffer()
    }

    pub fn put_char(&mut self, ch: char) {
        // char is a single Unicode scalar value, so it's a single grapheme
        self.put_grapheme(&ch.to_string());
    }

    fn put_grapheme(&mut self, g: &str) {
        let ch = g.chars().next().unwrap_or(' ');

        let cols = self.size.cols;
        let cursor_col = self.cursor.col;

        if cursor_col >= cols {
            if self.autowrap {
                self.linefeed();
                self.carriage_return();
            } else {
                self.cursor.col = cols.saturating_sub(1);
            }
        }

        let mut cell = Cell::new(ch);
        cell.fg = self.fg;
        cell.bg = self.bg;
        cell.attrs = self.attrs;

        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;

        if self.insert_mode && cursor_col < self.size.cols {
            let row = self.current_buffer_mut().get_mut(cursor_row);
            if let Some(row) = row {
                row.insert(cursor_col, cell);
                row.pop();
            }
        } else {
            let row = self.current_buffer_mut().get_mut(cursor_row);
            if let Some(row) = row {
                if let Some(cell_ref) = row.get_mut(cursor_col) {
                    *cell_ref = cell;
                }
            }
        }

        let width = ch.width().unwrap_or(1).max(1);
        self.cursor.col = (self.cursor.col + width).min(self.size.cols);
    }

    pub fn cursor_up(&mut self, n: usize) {
        let min_row = if self.origin_mode {
            self.scroll_top()
        } else {
            0
        };
        self.cursor.row = self.cursor.row.saturating_sub(n).max(min_row);
    }

    pub fn cursor_down(&mut self, n: usize) {
        let max_row = if self.origin_mode {
            self.scroll_bottom()
        } else {
            self.size.rows - 1
        };
        self.cursor.row = (self.cursor.row + n).min(max_row);
    }

    pub fn cursor_right(&mut self, n: usize) {
        self.cursor.col = (self.cursor.col + n).min(self.size.cols - 1);
    }

    pub fn cursor_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
    }

    pub fn cursor_left_n(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
    }

    pub fn cursor_pos(&mut self, row: usize, col: usize) {
        let row = if self.origin_mode {
            (self.scroll_top() + row.saturating_sub(1)).min(self.scroll_bottom())
        } else {
            row.saturating_sub(1).min(self.size.rows - 1)
        };
        let col = col.saturating_sub(1).min(self.size.cols - 1);
        self.cursor.row = row;
        self.cursor.col = col;
    }

    pub fn cursor_col(&mut self, col: usize) {
        self.cursor.col = col.saturating_sub(1).min(self.size.cols - 1);
    }

    pub fn cursor_row(&mut self, row: usize) {
        let row = if self.origin_mode {
            (self.scroll_top() + row.saturating_sub(1)).min(self.scroll_bottom())
        } else {
            row.saturating_sub(1).min(self.size.rows - 1)
        };
        self.cursor.row = row;
    }

    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    pub fn linefeed(&mut self) {
        if self.cursor.row + 1 > self.scroll_bottom() {
            self.scroll_up(1);
        } else {
            self.cursor.row += 1;
        }
    }

    pub fn reverse_linefeed(&mut self) {
        if self.cursor.row == self.scroll_top() {
            self.scroll_down(1);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
    }

    fn scroll_top(&self) -> usize {
        if self.scroll_top == 0 && self.scroll_bottom == 0 {
            0
        } else {
            self.scroll_top
        }
    }

    fn scroll_bottom(&self) -> usize {
        if self.scroll_top == 0 && self.scroll_bottom == 0 {
            self.size.rows - 1
        } else {
            self.scroll_bottom
        }
    }

    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        if top == 0 && bottom == 0 {
            self.scroll_top = 0;
            self.scroll_bottom = 0; // means full screen
        } else {
            self.scroll_top = top.saturating_sub(1).min(self.size.rows.saturating_sub(1));
            self.scroll_bottom = bottom
                .saturating_sub(1)
                .min(self.size.rows.saturating_sub(1));
            if self.scroll_top > self.scroll_bottom {
                self.scroll_bottom = self.scroll_top;
            }
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top();
        let bottom = self.scroll_bottom();
        let cols = self.size.cols;
        // Scrollback is only touched when the WHOLE screen scrolls; a DECSTBM
        // region scroll discards the line instead (real terminal behavior).
        let full_screen = top == 0 && bottom == self.size.rows.saturating_sub(1);

        // ponytail: scrollback is capped, so more iterations than the cap are wasted
        let n = n.min(self.scrollback_limit);

        for _ in 0..n {
            if top < bottom {
                let line = self.current_buffer_mut().remove(top);
                if full_screen {
                    self.scrollback.push_front(line);
                    if self.scrollback.len() > self.scrollback_limit {
                        self.scrollback.pop_back();
                    }
                }
                self.current_buffer_mut()
                    .insert(bottom, vec![Cell::default(); cols]);
            }
        }
    }

    pub fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top();
        let bottom = self.scroll_bottom();
        let cols = self.size.cols;
        // Mirror of scroll_up: partial-region reverse scroll never pulls lines
        // back out of scrollback.
        let full_screen = top == 0 && bottom == self.size.rows.saturating_sub(1);

        // ponytail: scrollback is capped, so more iterations than the cap are wasted
        let n = n.min(self.scrollback_limit);

        for _ in 0..n {
            if top < bottom {
                let line = if full_screen {
                    self.scrollback.pop_front()
                } else {
                    None
                };
                self.current_buffer_mut().remove(bottom);
                let blank = vec![Cell::default(); cols];
                match line {
                    Some(line) => self.current_buffer_mut().insert(top, line),
                    None => self.current_buffer_mut().insert(top, blank),
                }
            }
        }
    }

    pub fn erase_display(&mut self, mode: i64) {
        let cols = self.size.cols;
        let bottom = self.scroll_bottom();
        let top = self.scroll_top();
        match mode {
            0 => {
                let row = self.cursor.row;
                let col = self.cursor.col;
                for r in row..=bottom {
                    let start = if r == row { col } else { 0 };
                    if let Some(row_buf) = self.current_buffer_mut().get_mut(r) {
                        for cell in row_buf.iter_mut().take(cols).skip(start) {
                            *cell = Cell::default();
                        }
                    }
                }
            }
            1 => {
                let row = self.cursor.row;
                let col = self.cursor.col;
                for r in top..=row {
                    let end = if r == row { col + 1 } else { cols };
                    if let Some(row_buf) = self.current_buffer_mut().get_mut(r) {
                        for cell in row_buf.iter_mut().take(end) {
                            *cell = Cell::default();
                        }
                    }
                }
            }
            2 | 3 => {
                for r in top..=bottom {
                    if let Some(row_buf) = self.current_buffer_mut().get_mut(r) {
                        for cell in row_buf.iter_mut().take(cols) {
                            *cell = Cell::default();
                        }
                    }
                }
                self.scrollback.clear();
            }
            _ => {}
        }
    }

    pub fn erase_line(&mut self, mode: i64) {
        let row = self.cursor.row;
        let col = self.cursor.col;
        let cols = self.size.cols;
        if let Some(row_buf) = self.current_buffer_mut().get_mut(row) {
            match mode {
                0 => {
                    for cell in row_buf.iter_mut().take(cols).skip(col) {
                        *cell = Cell::default();
                    }
                }
                1 => {
                    for cell in row_buf.iter_mut().take(col + 1) {
                        *cell = Cell::default();
                    }
                }
                2 => {
                    for cell in row_buf.iter_mut().take(cols) {
                        *cell = Cell::default();
                    }
                }
                _ => {}
            }
        }
    }

    pub fn insert_lines(&mut self, n: usize) {
        let top = self.scroll_top();
        let bottom = self.scroll_bottom();
        let cols = self.size.cols;
        // More iterations than the region holds are no-ops; clamp so a hostile
        // "ESC [ 999999999999 L" cannot spin forever.
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            if top < bottom {
                self.current_buffer_mut().remove(bottom);
                self.current_buffer_mut()
                    .insert(top, vec![Cell::default(); cols]);
            }
        }
    }

    pub fn delete_lines(&mut self, n: usize) {
        let top = self.scroll_top();
        let bottom = self.scroll_bottom();
        let cols = self.size.cols;
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            if top < bottom {
                self.current_buffer_mut().remove(top);
                self.current_buffer_mut()
                    .insert(bottom, vec![Cell::default(); cols]);
            }
        }
    }

    pub fn insert_chars(&mut self, n: usize) {
        let row = self.cursor.row;
        let col = self.cursor.col;
        let cols = self.size.cols;
        let n = n.min(cols.saturating_sub(col));
        let buffer = self.current_buffer_mut();
        if let Some(row_buf) = buffer.get_mut(row) {
            for _ in 0..n {
                row_buf.insert(col, Cell::default());
                row_buf.pop();
            }
        }
    }

    pub fn delete_chars(&mut self, n: usize) {
        let row = self.cursor.row;
        let col = self.cursor.col;
        let cols = self.size.cols;
        let n = n.min(cols.saturating_sub(col));
        let buffer = self.current_buffer_mut();
        if let Some(row_buf) = buffer.get_mut(row) {
            for _ in 0..n {
                if col < row_buf.len() {
                    row_buf.remove(col);
                    row_buf.push(Cell::default());
                }
            }
        }
    }

    pub fn tab(&mut self) {
        let cols = self.size.cols;
        let mut col = self.cursor.col + 1;
        while col < cols {
            if self.tabs[col] {
                self.cursor.col = col;
                return;
            }
            col += 1;
        }
        self.cursor.col = cols - 1;
    }

    pub fn tab_set(&mut self) {
        if self.cursor.col < self.size.cols {
            self.tabs[self.cursor.col] = true;
        }
    }

    pub fn tab_clear(&mut self) {
        if self.cursor.col < self.size.cols {
            self.tabs[self.cursor.col] = false;
        }
    }

    pub fn tab_clear_all(&mut self) {
        for tab in &mut self.tabs {
            *tab = false;
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.size.cols, self.size.rows);
    }

    /// DECALN — screen alignment test: fill the display with 'E' and home the cursor.
    pub fn decaln(&mut self) {
        for row in self.current_buffer_mut() {
            for cell in row.iter_mut() {
                *cell = Cell::new('E');
            }
        }
        self.cursor.row = 0;
        self.cursor.col = 0;
    }

    pub fn reset_attributes(&mut self) {
        self.attrs = Attributes::default();
        self.fg = Color::DEFAULT_FG;
        self.bg = Color::DEFAULT_BG;
    }

    pub fn set_bold(&mut self, v: bool) {
        self.attrs.bold = v;
    }
    pub fn set_dim(&mut self, v: bool) {
        self.attrs.dim = v;
    }
    pub fn set_italic(&mut self, v: bool) {
        self.attrs.italic = v;
    }
    pub fn set_underline(&mut self, style: UnderlineStyle) {
        self.attrs.underline = style;
    }
    pub fn set_blink(&mut self, v: bool) {
        self.attrs.blink = v;
    }
    pub fn set_reverse(&mut self, v: bool) {
        self.attrs.reverse = v;
    }
    pub fn set_invisible(&mut self, v: bool) {
        self.attrs.invisible = v;
    }
    pub fn set_strikethrough(&mut self, v: bool) {
        self.attrs.strikethrough = v;
    }

    pub fn set_fg_ansi(&mut self, idx: u8) {
        self.fg = Color::from_ansi_16(idx);
    }

    pub fn set_bg_ansi(&mut self, idx: u8) {
        self.bg = Color::from_ansi_16(idx);
    }

    pub fn set_fg_256(&mut self, idx: u8) {
        self.fg = Color::from_ansi_256(idx);
    }

    pub fn set_bg_256(&mut self, idx: u8) {
        self.bg = Color::from_ansi_256(idx);
    }

    pub fn set_fg_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.fg = Color { r, g, b };
    }

    pub fn set_bg_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.bg = Color { r, g, b };
    }

    pub fn set_fg_default(&mut self) {
        self.fg = Color::DEFAULT_FG;
    }
    pub fn set_bg_default(&mut self) {
        self.bg = Color::DEFAULT_BG;
    }

    pub fn set_fg_ansi_bright_fg_ansi(&mut self, idx: u8) {
        self.fg = Color::from_ansi_16(idx + 8);
    }

    pub fn set_bright_bg_ansi(&mut self, idx: u8) {
        self.bg = Color::from_ansi_16(idx + 8);
    }

    pub fn shift_out(&mut self) {
        // SO - shift out (not implemented)
    }

    pub fn shift_in(&mut self) {
        // SI - shift in (not implemented)
    }

    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }

    pub fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved_cursor {
            self.cursor = saved;
            self.clamp_cursor();
        }
    }

    pub fn identify(&mut self) {
        if let Some(cb) = &self.ident_callback {
            cb();
        }
    }

    pub fn bell(&mut self) {
        if let Some(cb) = &self.bell_callback {
            cb();
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        if let Some(cb) = &self.title_callback {
            cb(title);
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn report_status(&mut self) {
        if let Some(cb) = &self.status_callback {
            cb();
        }
    }

    pub fn report_cursor(&mut self) {
        if let Some(cb) = &self.cursor_callback {
            cb();
        }
    }

    pub fn set_columns_132(&mut self, _enabled: bool) {
        // DECCOLM - ignored
    }

    pub fn set_autowrap(&mut self, v: bool) {
        self.autowrap = v;
    }
    pub fn set_origin_mode(&mut self, v: bool) {
        self.origin_mode = v;
    }
    pub fn set_reverse_video(&mut self, v: bool) {
        self.reverse_video = v;
    }
    pub fn set_auto_repeat(&mut self, v: bool) {
        self.auto_repeat = v;
    }
    pub fn set_mouse_tracking(&mut self, v: bool) {
        self.mouse_tracking = v;
    }
    pub fn set_cursor_visible(&mut self, v: bool) {
        self.cursor.visible = v;
    }
    pub fn set_cursor_keys_mode(&mut self, v: bool) {
        self.cursor_keys_mode = v;
    }
    pub fn set_newline_mode(&mut self, v: bool) {
        self.newline_mode = v;
    }
    pub fn set_keyboard_action_mode(&mut self, v: bool) {
        self.keyboard_action_mode = v;
    }
    pub fn set_insert_mode(&mut self, v: bool) {
        self.insert_mode = v;
    }
    pub fn set_send_receive_mode(&mut self, _v: bool) { /* ignored */
    }
    pub fn set_alternate_screen(&mut self, enable: bool) {
        if enable && !self.use_alt_screen {
            self.alt_buffer = Some(vec![vec![Cell::default(); self.size.cols]; self.size.rows]);
            self.use_alt_screen = true;
        } else if !enable && self.use_alt_screen {
            self.alt_buffer = None;
            self.use_alt_screen = false;
        }
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.current_buffer().get(row)?.get(col)
    }

    pub fn visible_rows(&self) -> &[Vec<Cell>] {
        &self.current_buffer()[self.scroll_top()..=self.scroll_bottom()]
    }

    pub fn scrollback(&self) -> &VecDeque<Vec<Cell>> {
        &self.scrollback
    }

    pub fn on_bell(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.bell_callback = Some(Box::new(f));
    }

    pub fn on_title(&mut self, f: impl Fn(&str) + Send + Sync + 'static) {
        self.title_callback = Some(Box::new(f));
    }

    pub fn on_ident(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.ident_callback = Some(Box::new(f));
    }

    pub fn on_status(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.status_callback = Some(Box::new(f));
    }

    pub fn on_cursor_report(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.cursor_callback = Some(Box::new(f));
    }

    pub fn mark_block_boundary(&mut self) {
        if let Some(block) = self.blocks.last_mut() {
            if block.end_line.is_none() {
                block.end_line = Some(self.cursor.row);
            }
            block.duration_ms = Some(block.timestamp.elapsed().as_millis() as u64);
        }
        let id = self.block_id_counter;
        self.block_id_counter += 1;
        self.blocks.push(CommandBlock {
            id,
            start_line: self.cursor.row,
            end_line: None,
            command: std::mem::take(&mut self.current_block_command),
            exit_code: None,
            timestamp: std::time::Instant::now(),
            duration_ms: None,
        });
    }

    pub fn blocks(&self) -> &[CommandBlock] {
        &self.blocks
    }

    pub fn block_metadata(&self, block: &CommandBlock) -> String {
        let exit = block
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string());
        match block.duration_ms {
            Some(ms) => format!("exit:{} \u{00b7} {}ms", exit, ms),
            None => format!("exit:{}", exit),
        }
    }

    pub fn set_block_command(&mut self, cmd: &str) {
        self.current_block_command = cmd.to_string();
    }

    pub fn set_block_exit_code(&mut self, code: i32) {
        if let Some(block) = self.blocks.last_mut() {
            block.exit_code = Some(code);
        }
    }

    pub fn place_image(&mut self, rgba_data: Vec<u8>, width: u32, height: u32) -> u32 {
        let frames = vec![FrameData {
            width,
            height,
            rgba: rgba_data,
            delay_ms: 0,
        }];
        self.place_image_frames(frames, width, height)
    }

    pub fn place_image_frames(
        &mut self,
        mut frames: Vec<FrameData>,
        width: u32,
        height: u32,
    ) -> u32 {
        let id = self.next_image_id;
        self.next_image_id += 1;
        let total_bytes: u64 = frames.iter().map(|f| f.rgba.len() as u64).sum();
        if width == 0
            || height == 0
            || (width as u64) * (height as u64) * 4 > MAX_IMAGE_BYTES as u64
            || total_bytes > MAX_IMAGE_BYTES as u64
        {
            return id;
        }
        if frames.len() > MAX_ANIM_FRAMES {
            frames.truncate(MAX_ANIM_FRAMES);
        }
        while self.image_registry.len() >= MAX_IMAGES {
            let oldest = self.image_registry.keys().min().copied();
            match oldest {
                Some(old) => {
                    self.image_registry.remove(&old);
                    self.image_cells.retain(|_, v| *v != old);
                }
                None => break,
            }
        }
        let rgba_data = match frames.first() {
            Some(f) => f.rgba.clone(),
            None => Vec::new(),
        };
        self.image_registry.insert(
            id,
            ImageData {
                id,
                width,
                height,
                rgba_data,
                frames,
            },
        );
        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;
        self.image_cells.insert((cursor_row, cursor_col), id);
        id
    }

    pub fn image_registry(&self) -> &HashMap<u32, ImageData> {
        &self.image_registry
    }

    pub fn image_cells(&self) -> &HashMap<(usize, usize), u32> {
        &self.image_cells
    }
}
