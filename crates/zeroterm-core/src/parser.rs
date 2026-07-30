//! VT100/ANSI parser - hand-written state machine

use crate::cell::UnderlineStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
    _CsiIgnore,
    CsiIntermediate,
    OscString,
    DcsEntry,
    DcsParam,
    _DcsIgnore,
    DcsIntermediate,
    DcsPassthrough,
    SosPmApcString,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MouseTrackingMode {
    #[default]
    Off,
    Normal,
    ButtonEvents,
    AnyEvent,
}

#[derive(Debug, Clone)]
pub struct ImageFragment {
    pub id: u32,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Default)]
struct CsiParams {
    params: Vec<Option<i64>>,
    current: String,
    intermediates: Vec<char>,
}

impl CsiParams {
    fn new() -> Self {
        Self::default()
    }

    fn add_digit(&mut self, digit: char) {
        self.current.push(digit);
    }

    fn commit_param(&mut self) {
        let val = self.current.parse().ok();
        self.params.push(val);
        self.current.clear();
    }

    fn add_intermediate(&mut self, ch: char) {
        self.intermediates.push(ch);
    }

    fn get(&self, idx: usize, default: i64) -> i64 {
        self.params.get(idx).and_then(|v| *v).unwrap_or(default)
    }

    fn param_count(&self) -> usize {
        self.params.len()
    }

    fn reset(&mut self) {
        self.params.clear();
        self.current.clear();
        self.intermediates.clear();
    }
}

pub struct Parser {
    state: ParserState,
    csi: CsiParams,
    osc_buffer: String,
    dcs_buffer: String,
    screen: crate::screen::Screen,
    after_newline: bool,
    command_buf: String,
    collecting_command: bool,
    mouse_tracking: MouseTrackingMode,
    bracketed_paste: bool,
    pub pending_images: Vec<ImageFragment>,
    apc_buffer: String,
    clipboard_text: Option<String>,
}

impl Parser {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            state: ParserState::Ground,
            csi: CsiParams::new(),
            osc_buffer: String::new(),
            dcs_buffer: String::new(),
            screen: crate::screen::Screen::new(cols, rows),
            after_newline: false,
            command_buf: String::new(),
            collecting_command: false,
            mouse_tracking: MouseTrackingMode::Off,
            bracketed_paste: false,
            pending_images: Vec::new(),
            apc_buffer: String::new(),
            clipboard_text: None,
        }
    }

    pub fn parse(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.handle_byte(byte);
        }
    }

    pub fn screen(&self) -> &crate::screen::Screen {
        &self.screen
    }

    pub fn screen_mut(&mut self) -> &mut crate::screen::Screen {
        &mut self.screen
    }

    pub fn set_exit_code(&mut self, code: i32) {
        self.screen.set_block_exit_code(code);
    }

    pub fn mouse_tracking(&self) -> MouseTrackingMode {
        self.mouse_tracking
    }

    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }

    fn handle_byte(&mut self, byte: u8) {
        match self.state {
            ParserState::Ground => self.handle_ground(byte),
            ParserState::Escape => self.handle_escape(byte),
            ParserState::CsiEntry => self.handle_csi_entry(byte),
            ParserState::CsiParam => self.handle_csi_param(byte),
            ParserState::_CsiIgnore => self.handle_csi_ignore(byte),
            ParserState::CsiIntermediate => self.handle_csi_intermediate(byte),
            ParserState::OscString => self.handle_osc_string(byte),
            ParserState::DcsEntry => self.handle_dcs_entry(byte),
            ParserState::DcsParam => self.handle_dcs_param(byte),
            ParserState::_DcsIgnore => self.handle_dcs_ignore(byte),
            ParserState::DcsIntermediate => self.handle_dcs_intermediate(byte),
            ParserState::DcsPassthrough => self.handle_dcs_passthrough(byte),
            ParserState::SosPmApcString => self.handle_sos_pm_apc(byte),
        }
    }

    fn handle_ground(&mut self, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F | 0x18 | 0x1A | 0x7F => self.execute_control(byte),
            0x1B => self.state = ParserState::Escape,
            0x20..=0x7E => {
                let ch = byte as char;
                if self.after_newline {
                    if matches!(ch, '$' | '%' | '#' | '>') {
                        self.screen.mark_block_boundary();
                        self.collecting_command = true;
                        self.command_buf.clear();
                    }
                    self.after_newline = false;
                }
                if self.collecting_command {
                    self.command_buf.push(ch);
                }
                self.screen.put_char(ch);
            }
            0x80..=0xFF => {} // UTF-8 continuation bytes handled by caller
        }
    }

    fn handle_escape(&mut self, byte: u8) {
        match byte {
            0x5B => {
                // '['
                self.state = ParserState::CsiEntry;
                self.csi.reset();
            }
            0x5D => {
                // ']'
                self.state = ParserState::OscString;
                self.osc_buffer.clear();
            }
            0x50 => {
                // 'P'
                self.state = ParserState::DcsEntry;
                self.dcs_buffer.clear();
                self.csi.reset();
            }
            0x58 | 0x5E | 0x5F => {
                // 'X' '^' '_' - SOS, PM, APC
                self.state = ParserState::SosPmApcString;
            }
            0x30..=0x4F => {
                // Single-char escape sequences
                self.handle_escape_sequence(byte as char);
                self.state = ParserState::Ground;
            }
            0x60..=0x7E => {
                // Single-char escape sequences
                self.handle_escape_sequence(byte as char);
                self.state = ParserState::Ground;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_csi_entry(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 => {
                self.state = ParserState::CsiParam;
                self.csi.add_digit(byte as char);
            }
            0x3A | 0x3B => {
                self.state = ParserState::CsiParam;
                self.csi.commit_param();
            }
            0x3C..=0x3F => {
                self.state = ParserState::CsiParam;
                self.csi.add_intermediate(byte as char);
            }
            0x20..=0x2F => {
                self.state = ParserState::CsiIntermediate;
                self.csi.add_intermediate(byte as char);
            }
            0x40..=0x7E => {
                let intermediates = self.csi.intermediates.clone();
                self.csi.commit_param();
                self.handle_csi(byte as char, &intermediates);
                self.state = ParserState::Ground;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_csi_param(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 => self.csi.add_digit(byte as char),
            0x3A | 0x3B => self.csi.commit_param(),
            0x3C..=0x3F => self.csi.add_intermediate(byte as char),
            0x20..=0x2F => {
                self.state = ParserState::CsiIntermediate;
                self.csi.add_intermediate(byte as char);
            }
            0x40..=0x7E => {
                let intermediates = self.csi.intermediates.clone();
                self.csi.commit_param();
                self.handle_csi(byte as char, &intermediates);
                self.state = ParserState::Ground;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_csi_intermediate(&mut self, byte: u8) {
        match byte {
            0x20..=0x2F => self.csi.add_intermediate(byte as char),
            0x40..=0x7E => {
                let intermediates = self.csi.intermediates.clone();
                self.csi.commit_param();
                self.handle_csi(byte as char, &intermediates);
                self.state = ParserState::Ground;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_csi_ignore(&mut self, byte: u8) {
        if (0x40..=0x7E).contains(&byte) {
            self.state = ParserState::Ground;
        }
    }

    fn handle_osc_string(&mut self, byte: u8) {
        match byte {
            0x07 | 0x1B => {
                let buffer = std::mem::take(&mut self.osc_buffer);
                self.handle_osc(&buffer);
                self.state = ParserState::Ground;
            }
            0x20..=0x7E => self.osc_buffer.push(byte as char),
            _ => {}
        }
    }

    fn handle_dcs_entry(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 => {
                self.state = ParserState::DcsParam;
                self.csi.add_digit(byte as char);
            }
            0x3A | 0x3B => {
                self.state = ParserState::DcsParam;
                self.csi.commit_param();
            }
            0x3C..=0x3F => {
                self.state = ParserState::DcsParam;
                self.csi.add_intermediate(byte as char);
            }
            0x20..=0x2F => {
                self.state = ParserState::DcsIntermediate;
                self.csi.add_intermediate(byte as char);
            }
            0x40..=0x7E => {
                self.csi.commit_param();
                self.state = ParserState::DcsPassthrough;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_dcs_param(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 => self.csi.add_digit(byte as char),
            0x3A | 0x3B => self.csi.commit_param(),
            0x3C..=0x3F => self.csi.add_intermediate(byte as char),
            0x20..=0x2F => {
                self.state = ParserState::DcsIntermediate;
                self.csi.add_intermediate(byte as char);
            }
            0x40..=0x7E => {
                self.csi.commit_param();
                self.state = ParserState::DcsPassthrough;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_dcs_intermediate(&mut self, byte: u8) {
        match byte {
            0x20..=0x2F => self.csi.add_intermediate(byte as char),
            0x40..=0x7E => {
                self.csi.commit_param();
                self.state = ParserState::DcsPassthrough;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_dcs_passthrough(&mut self, byte: u8) {
        match byte {
            0x1B => {
                let buffer = std::mem::take(&mut self.dcs_buffer);
                self.handle_dcs_string(&buffer);
                self.state = ParserState::Ground;
            }
            _ => self.dcs_buffer.push(byte as char),
        }
    }

    fn handle_dcs_ignore(&mut self, byte: u8) {
        if byte == 0x1B {
            self.state = ParserState::Ground;
        }
    }

    fn handle_sos_pm_apc(&mut self, byte: u8) {
        match byte {
            0x1B => {
                let buffer = std::mem::take(&mut self.apc_buffer);
                self.handle_apc(&buffer);
                self.state = ParserState::Ground;
            }
            0x20..=0x7E => self.apc_buffer.push(byte as char),
            _ => {}
        }
    }

    fn execute_control(&mut self, byte: u8) {
        match byte {
            0x07 => self.screen.bell(),        // BEL
            0x08 => self.screen.cursor_left(), // BS
            0x09 => self.screen.tab(),         // HT
            0x0A | 0x0B | 0x0C => {
                self.screen.linefeed(); // LF, VT, FF
                self.after_newline = true;
            }
            0x0D => {
                self.screen.carriage_return(); // CR
                self.after_newline = true;
                if self.collecting_command {
                    self.screen.set_block_command(&self.command_buf);
                    self.command_buf.clear();
                    self.collecting_command = false;
                }
            }
            0x0E => self.screen.shift_out(),                 // SO
            0x0F => self.screen.shift_in(),                  // SI
            0x18 | 0x1A => self.state = ParserState::Ground, // CAN, SUB
            _ => {}
        }
    }

    fn handle_escape_sequence(&mut self, ch: char) {
        match ch {
            '7' => self.screen.save_cursor(),    // DECSC
            '8' => self.screen.restore_cursor(), // DECRC
            'D' => self.screen.linefeed(),       // IND
            'E' => {
                self.screen.carriage_return();
                self.screen.linefeed();
            } // NEL
            'H' => self.screen.tab_set(),        // HTS
            'M' => self.screen.reverse_linefeed(), // RI
            'Z' => self.screen.identify(),       // DECID
            'c' => self.screen.reset(),          // RIS
            '(' | ')' | '*' | '+' => {}          // Designate charset - ignored
            _ => {}
        }
    }

    fn handle_csi(&mut self, final_byte: char, intermediates: &[char]) {
        let private = intermediates
            .iter()
            .any(|&c| c == '?' || c == '>' || c == '<' || c == '=');
        let intermediate: String = intermediates.iter().collect();

        match (final_byte, private, intermediate.as_str()) {
            // Cursor movement
            ('A', false, "") => self.screen.cursor_up(self.csi.get(0, 1) as usize), // CUU
            ('B', false, "") => self.screen.cursor_down(self.csi.get(0, 1) as usize), // CUD
            ('C', false, "") => self.screen.cursor_right(self.csi.get(0, 1) as usize), // CUF
            ('D', false, "") => self.screen.cursor_left_n(self.csi.get(0, 1) as usize), // CUB
            ('E', false, "") => {
                self.screen.cursor_down(self.csi.get(0, 1) as usize);
                self.screen.carriage_return();
            } // CNL
            ('F', false, "") => {
                self.screen.cursor_up(self.csi.get(0, 1) as usize);
                self.screen.carriage_return();
            } // CPL
            ('G', false, "") => self.screen.cursor_col(self.csi.get(0, 1) as usize), // CHA
            ('H', false, "") | ('f', false, "") => {
                let row = self.csi.get(0, 1) as usize;
                let col = self.csi.get(1, 1) as usize;
                self.screen.cursor_pos(row, col);
            } // CUP, HVP
            ('d', false, "") => self.screen.cursor_row(self.csi.get(0, 1) as usize), // VPA

            // Scrolling
            ('S', false, "") => self.screen.scroll_up(self.csi.get(0, 1) as usize), // SU
            ('T', false, "") => self.screen.scroll_down(self.csi.get(0, 1) as usize), // SD

            // Erasing
            ('J', false, "") => self.screen.erase_display(self.csi.get(0, 0)), // ED
            ('K', false, "") => self.screen.erase_line(self.csi.get(0, 0)),    // EL

            // Insert/Delete
            ('L', false, "") => self.screen.insert_lines(self.csi.get(0, 1) as usize), // IL
            ('M', false, "") => self.screen.delete_lines(self.csi.get(0, 1) as usize), // DL
            ('@', false, "") => self.screen.insert_chars(self.csi.get(0, 1) as usize), // ICH
            ('P', false, "") => self.screen.delete_chars(self.csi.get(0, 1) as usize), // DCH

            // Attributes
            ('m', false, "") => self.handle_sgr(), // SGR

            // Mode setting
            ('h', true, "") => self.handle_dec_mode(true), // DECSM
            ('l', true, "") => self.handle_dec_mode(false), // DECRM
            ('h', false, "") => self.handle_mode(true),    // SM
            ('l', false, "") => self.handle_mode(false),   // RM

            // Reports
            ('n', false, "") => self.handle_dsr(), // DSR
            ('c', false, "") | ('c', true, "") => self.screen.identify(), // DA

            // Title (OSC handled separately)
            _ => {}
        }
    }

    fn handle_sgr(&mut self) {
        let mut i = 0;
        while i < self.csi.param_count() {
            let param = self.csi.get(i, -1);
            match param {
                0 => self.screen.reset_attributes(), // Reset
                1 => self.screen.set_bold(true),
                2 => self.screen.set_dim(true),
                3 => self.screen.set_italic(true),
                4 => self.screen.set_underline(UnderlineStyle::Single),
                5 => self.screen.set_blink(true), // Slow blink
                6 => self.screen.set_blink(true), // Fast blink
                7 => self.screen.set_reverse(true),
                8 => self.screen.set_invisible(true),
                9 => self.screen.set_strikethrough(true),
                10..=19 => {} // Font selection - ignored
                21 => self.screen.set_bold(false),
                22 => {
                    self.screen.set_bold(false);
                    self.screen.set_dim(false);
                }
                23 => self.screen.set_italic(false),
                24 => self.screen.set_underline(UnderlineStyle::None),
                25 => self.screen.set_blink(false),
                27 => self.screen.set_reverse(false),
                28 => self.screen.set_invisible(false),
                29 => self.screen.set_strikethrough(false),
                30..=37 => self.screen.set_fg_ansi(param as u8 - 30),
                38 => {
                    // Extended foreground
                    if i + 1 < self.csi.param_count() {
                        let mode = self.csi.get(i + 1, 0);
                        match mode {
                            2 => {
                                // RGB
                                if i + 3 < self.csi.param_count() {
                                    let r = self.csi.get(i + 2, 0) as u8;
                                    let g = self.csi.get(i + 3, 0) as u8;
                                    let b = self.csi.get(i + 4, 0) as u8;
                                    self.screen.set_fg_rgb(r, g, b);
                                    i += 4;
                                }
                            }
                            5 => {
                                // 256-color
                                if i + 2 < self.csi.param_count() {
                                    let idx = self.csi.get(i + 2, 0) as u8;
                                    self.screen.set_fg_256(idx);
                                    i += 2;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                39 => self.screen.set_fg_default(),
                40..=47 => self.screen.set_bg_ansi(param as u8 - 40),
                48 => {
                    // Extended background
                    if i + 1 < self.csi.param_count() {
                        let mode = self.csi.get(i + 1, 0);
                        match mode {
                            2 => {
                                // RGB
                                if i + 3 < self.csi.param_count() {
                                    let r = self.csi.get(i + 2, 0) as u8;
                                    let g = self.csi.get(i + 3, 0) as u8;
                                    let b = self.csi.get(i + 4, 0) as u8;
                                    self.screen.set_bg_rgb(r, g, b);
                                    i += 4;
                                }
                            }
                            5 => {
                                // 256-color
                                if i + 2 < self.csi.param_count() {
                                    let idx = self.csi.get(i + 2, 0) as u8;
                                    self.screen.set_bg_256(idx);
                                    i += 2;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                49 => self.screen.set_bg_default(),
                90..=97 => self.screen.set_fg_ansi_bright_fg_ansi(param as u8 - 90),
                100..=107 => self.screen.set_bright_bg_ansi(param as u8 - 100),
                _ => {}
            }
            i += 1;
        }
    }

    fn handle_mode(&mut self, set: bool) {
        let param = self.csi.get(0, 0);
        match param {
            2 => self.screen.set_keyboard_action_mode(set), // KAM
            4 => self.screen.set_insert_mode(set),          // IRM
            12 => self.screen.set_send_receive_mode(set),   // SRM
            20 => self.screen.set_newline_mode(set),        // LNM
            _ => {}
        }
    }

    fn handle_dec_mode(&mut self, set: bool) {
        // ponytail: always Off on deselect, fine-grained tracking if needed
        for i in 0..self.csi.param_count() {
            let param = self.csi.get(i, 0);
            match param {
                1 => self.screen.set_cursor_keys_mode(set),
                3 => self.screen.set_columns_132(set),
                5 => self.screen.set_reverse_video(set),
                6 => self.screen.set_origin_mode(set),
                7 => self.screen.set_autowrap(set),
                8 => self.screen.set_auto_repeat(set),
                9 => self.screen.set_mouse_tracking(set),
                25 => self.screen.set_cursor_visible(set),
                47 => self.screen.set_alternate_screen(set),
                1000 => {
                    self.mouse_tracking = if set {
                        MouseTrackingMode::Normal
                    } else {
                        MouseTrackingMode::Off
                    };
                    self.screen.set_mouse_tracking(set);
                }
                1001 => {
                    self.mouse_tracking = if set {
                        MouseTrackingMode::ButtonEvents
                    } else {
                        MouseTrackingMode::Off
                    };
                    self.screen.set_mouse_tracking(set);
                }
                1002 => {
                    self.mouse_tracking = if set {
                        MouseTrackingMode::AnyEvent
                    } else {
                        MouseTrackingMode::Off
                    };
                    self.screen.set_mouse_tracking(set);
                }
                1003 => {
                    self.mouse_tracking = if set {
                        MouseTrackingMode::AnyEvent
                    } else {
                        MouseTrackingMode::Off
                    };
                    self.screen.set_mouse_tracking(set);
                }
                2004 => self.bracketed_paste = set,
                1049 => self.screen.set_alternate_screen(set),
                _ => {}
            }
        }
    }

    fn handle_dsr(&mut self) {
        let param = self.csi.get(0, 0);
        match param {
            5 => self.screen.report_status(), // Status report
            6 => self.screen.report_cursor(), // Cursor position report
            _ => {}
        }
    }

    fn handle_osc(&mut self, osc: &str) {
        if osc.starts_with("0;") || osc.starts_with("2;") {
            let title = &osc[2..];
            self.screen.set_title(title);
        } else if osc.starts_with("4;") {
            // Color palette - ignored for now
        } else if osc.starts_with("8;") {
            // Hyperlink - ignored for now
        } else if osc.starts_with("52;") {
            let payload = &osc[3..];
            if let Some(semicolon) = payload.find(';') {
                let base64_data = &payload[semicolon + 1..];
                if !base64_data.is_empty() {
                    if let Some(decoded) = decode_base64(base64_data) {
                        if let Ok(text) = String::from_utf8(decoded) {
                            self.clipboard_text = Some(text);
                        }
                    }
                }
            }
        }
    }

    fn handle_apc(&mut self, apc: &str) {
        if let Some(payload) = apc.strip_prefix('G') {
            self.handle_kitty(payload);
        }
    }

    fn handle_kitty(&mut self, payload: &str) {
        let (kv_part, data_part) = match payload.split_once(';') {
            Some((k, d)) => (k, d),
            None => (payload, ""),
        };
        let mut action = 'T';
        let mut width = 0u32;
        let mut height = 0u32;
        for pair in kv_part.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "a" => action = v.chars().next().unwrap_or('T'),
                    "s" => width = v.parse().unwrap_or(0),
                    "v" => height = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        if action != 'T' || data_part.is_empty() || width == 0 || height == 0 {
            return;
        }
        if let Some(decoded) = decode_base64(data_part) {
            let id = self.screen.place_image(decoded.clone(), width, height);
            self.pending_images.push(ImageFragment {
                id,
                data: decoded,
                width,
                height,
            });
        }
    }

    fn handle_dcs_string(&mut self, data: &str) {
        if data.starts_with("q") || data.starts_with("0;1;q") {
            self.handle_sixel(data);
        }
    }

    fn handle_sixel(&mut self, data: &str) {
        let palette: [(u8, u8, u8); 256] = [(0, 0, 0); 256];
        let mut pal_idx: u8 = 0;
        let mut pixels: Vec<Vec<u32>> = Vec::new();
        let mut x: u32 = 0;
        let mut y: u32 = 0;
        let mut max_x: u32 = 0;
        // ponytail: bare-bones stair-step parser, skips palette init cmds
        for ch in data.chars() {
            match ch {
                '#' => {
                    pal_idx = 0;
                }
                'P' => {
                    pal_idx = 0;
                }
                ';' | ':' | '$' => {
                    if ch == '$' {
                        x = 0;
                        y += 6;
                    }
                }
                '-' => {
                    x = 0;
                    y += 6;
                }
                c if ('?'..='~').contains(&c) => {
                    let sixel = (c as u8) - 63;
                    for bit in 0..6 {
                        if (sixel >> bit) & 1 != 0 {
                            let py = y + bit;
                            while pixels.len() <= py as usize {
                                pixels.push(Vec::new());
                            }
                            let row = &mut pixels[py as usize];
                            while row.len() <= x as usize {
                                row.push(0);
                            }
                            let (r, g, b) = palette[pal_idx as usize];
                            row[x as usize] =
                                0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                        }
                    }
                    x += 1;
                    if x > max_x {
                        max_x = x;
                    }
                }
                _ => {}
            }
        }
        let height = pixels.len() as u32;
        let width = max_x;
        if width == 0 || height == 0 {
            return;
        }
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                if let Some(row) = pixels.get(y as usize) {
                    if let Some(&pixel) = row.get(x as usize) {
                        rgba.push(((pixel >> 16) & 0xFF) as u8);
                        rgba.push(((pixel >> 8) & 0xFF) as u8);
                        rgba.push((pixel & 0xFF) as u8);
                        rgba.push(0xFF);
                    } else {
                        rgba.extend_from_slice(&[0, 0, 0, 0]);
                    }
                } else {
                    rgba.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        let id = self.screen.place_image(rgba.clone(), width, height);
        self.pending_images.push(ImageFragment {
            id,
            data: rgba,
            width,
            height,
        });
    }

    pub fn take_pending_images(&mut self) -> Vec<ImageFragment> {
        std::mem::take(&mut self.pending_images)
    }

    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.clipboard_text.take()
    }
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let input = input.trim_end_matches('=');
    let mut output = Vec::with_capacity(input.len() * 3 / 4 + 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for ch in input.chars() {
        let val = match ch {
            'A'..='Z' => ch as u32 - 65,
            'a'..='z' => ch as u32 - 71,
            '0'..='9' => ch as u32 + 4,
            '+' => 62,
            '/' => 63,
            _ => continue,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(output)
}
