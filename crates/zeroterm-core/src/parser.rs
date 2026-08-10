//! VT100/ANSI parser - hand-written state machine

use crate::cell::{CursorShape, UnderlineStyle};
use crate::image_decode::{self, FrameData};

const MAX_CSI_PARAMS: usize = 32;
const MAX_CSI_PARAM_DIGITS: usize = 32;
const MAX_CSI_INTERMEDIATES: usize = 2;
const MAX_OSC_BUFFER: usize = 1 << 20;
const MAX_DCS_BUFFER: usize = 1 << 22;
// kitty graphics chunks carry up to 4096 raw image bytes (~5.5KB of base64)
// and many emitters send the whole image in one APC; 4096 CHARS truncated
// every real image. Memory stays bounded by the decode/dimension caps below.
const MAX_APC_BUFFER: usize = 1 << 22;
const MAX_SIXEL_W: u32 = 8192;
const MAX_SIXEL_H: u32 = 8192;
const MAX_SIXEL_BYTES: usize = 100 << 20;

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
    /// Decoded RGBA frames; empty when the payload is not a decodable image.
    pub frames: Vec<FrameData>,
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
        if self.current.len() < MAX_CSI_PARAM_DIGITS {
            self.current.push(digit);
        }
    }

    fn commit_param(&mut self) {
        let val = self.current.parse().ok();
        if self.params.len() < MAX_CSI_PARAMS {
            self.params.push(val);
        }
        self.current.clear();
    }

    fn add_intermediate(&mut self, ch: char) {
        if self.intermediates.len() < MAX_CSI_INTERMEDIATES {
            self.intermediates.push(ch);
        }
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
    osc_buffer: Vec<u8>,
    dcs_buffer: Vec<u8>,
    escape_intermediates: Vec<char>,
    screen: crate::screen::Screen,
    after_newline: bool,
    command_buf: String,
    collecting_command: bool,
    mouse_tracking: MouseTrackingMode,
    bracketed_paste: bool,
    sync_output: bool,
    pub pending_images: Vec<ImageFragment>,
    apc_buffer: String,
    clipboard_text: Option<String>,
    /// Kitty keyboard protocol: current enhancement flags (bit 0 = disambiguate)
    /// plus a push/pop stack. Apps enable the protocol with `CSI > flags u`;
    /// while bit 0 is set the app expects CSI-u key encodings.
    kitty_flags: u32,
    kitty_stack: Vec<u32>,
    /// Whether this terminal advertises kitty keyboard support in its reply
    /// to `CSI ? u`. Off by default; main.rs enables it per config so old
    /// apps that expect silence for unknown queries keep working.
    kitty_supported: bool,
    /// Response bytes the app asked for (kitty `CSI ? u` reply, cursor/DA
    /// reports). Drained by the app and written back to the pty.
    pending_response: Option<Vec<u8>>,
    /// OSC 9 desktop-notification text, drained by the app.
    notification: Option<String>,
    /// Latched when the parser erases the visible screen (ED 2) or clears
    /// scrollback (ED 3). The app drains this to snap the scrollback view to
    /// the bottom — without it, `clear`/`Ctrl+L` leave the viewport stranded in
    /// the (now-blank or deleted) history, unlike kitty or other terminals.
    clear_flag: bool,
    // Incomplete UTF-8 sequence accumulation (streaming PTY reads split code points)
    utf8_pending: Vec<u8>,
}

impl Parser {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            state: ParserState::Ground,
            csi: CsiParams::new(),
            osc_buffer: Vec::new(),
            dcs_buffer: Vec::new(),
            escape_intermediates: Vec::new(),
            screen: crate::screen::Screen::new(cols, rows),
            after_newline: false,
            command_buf: String::new(),
            collecting_command: false,
            mouse_tracking: MouseTrackingMode::Off,
            bracketed_paste: false,
            sync_output: false,
            pending_images: Vec::new(),
            apc_buffer: String::new(),
            clipboard_text: None,
            kitty_flags: 0,
            kitty_stack: Vec::new(),
            kitty_supported: false,
            pending_response: None,
            notification: None,
            clear_flag: false,
            utf8_pending: Vec::new(),
        }
    }

    /// Advertise kitty keyboard protocol support (config-gated in the app).
    pub fn set_kitty_supported(&mut self, supported: bool) {
        self.kitty_supported = supported;
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

    pub fn sync_output(&self) -> bool {
        self.sync_output
    }

    /// Kitty keyboard disambiguation active (app pushed `CSI > 1 u`)? When
    /// true the app expects functional keys to carry modifier params and
    /// ctrl/alt letters to arrive as CSI-u sequences.
    pub fn kitty_disambiguate(&self) -> bool {
        self.kitty_flags & 1 != 0
    }

    pub fn take_response(&mut self) -> Option<Vec<u8>> {
        self.pending_response.take()
    }

    /// Drain the latched clear flag (set when ED 2 or ED 3 erases the screen or
    /// scrollback). The app resets its scroll offset when this is true so the
    /// view snaps to the bottom instead of staying stranded in cleared history.
    pub fn take_clear_flag(&mut self) -> bool {
        std::mem::take(&mut self.clear_flag)
    }

    pub fn take_notification(&mut self) -> Option<String> {
        self.notification.take()
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
            0x00..=0x17 | 0x19 | 0x1C..=0x1F | 0x18 | 0x1A | 0x7F => {
                self.utf8_pending.clear();
                self.execute_control(byte);
            }
            0x1B => {
                self.utf8_pending.clear();
                self.state = ParserState::Escape;
            }
            0x20..=0x7E => {
                self.utf8_pending.clear();
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
            // UTF-8 lead/continuation bytes: accumulate, emit on complete sequence.
            0x80..=0xFF => {
                self.utf8_pending.push(byte);
                if self.utf8_pending.len() > 4 {
                    // 5+ bytes can never form one code point — invalid input.
                    self.utf8_pending.clear();
                    self.emit_char('\u{FFFD}');
                    return;
                }
                match std::str::from_utf8(&self.utf8_pending) {
                    Ok(s) => {
                        // May hold several code points when continuation bytes
                        // piled up behind a completed one; emit them all.
                        let chars: Vec<char> = s.chars().collect();
                        self.utf8_pending.clear();
                        for ch in chars {
                            self.emit_char(ch);
                        }
                    }
                    Err(e) if e.error_len().is_some() => {
                        // A genuinely invalid sequence (bad lead, bad
                        // continuation, overlong): emit U+FFFD and resync
                        // instead of letting the bad bytes swallow later text.
                        self.utf8_pending.clear();
                        self.emit_char('\u{FFFD}');
                    }
                    // Incomplete prefix of a valid code point — keep buffering.
                    Err(_) => {}
                }
            }
        }
    }

    /// Route one decoded character to the screen, keeping block-command
    /// capture in sync. Multibyte characters never match prompt sigils, so
    /// only the after-newline flag and command buffer need handling here.
    fn emit_char(&mut self, ch: char) {
        self.after_newline = false;
        if self.collecting_command {
            self.command_buf.push(ch);
        }
        self.screen.put_char(ch);
    }

    fn handle_escape(&mut self, byte: u8) {
        match byte {
            0x20..=0x2F => {
                // Intermediate bytes (e.g. ESC ( B, ESC # 8): collect, stay in
                // Escape until the final byte 0x30..=0x7E arrives.
                if self.escape_intermediates.len() < MAX_CSI_INTERMEDIATES {
                    self.escape_intermediates.push(byte as char);
                }
            }
            0x5B => {
                // '['
                self.state = ParserState::CsiEntry;
                self.escape_intermediates.clear();
                self.csi.reset();
            }
            0x5D => {
                // ']'
                self.state = ParserState::OscString;
                self.escape_intermediates.clear();
                self.osc_buffer.clear();
            }
            0x50 => {
                // 'P'
                self.state = ParserState::DcsEntry;
                self.escape_intermediates.clear();
                self.dcs_buffer.clear();
                self.csi.reset();
            }
            0x58 | 0x5E | 0x5F => {
                // 'X' '^' '_' - SOS, PM, APC
                self.state = ParserState::SosPmApcString;
                self.escape_intermediates.clear();
                self.apc_buffer.clear();
            }
            0x30..=0x4F => {
                // Final byte of an escape sequence (single-char or after intermediates)
                let intermediates = std::mem::take(&mut self.escape_intermediates);
                self.handle_escape_sequence(byte as char, &intermediates);
                self.state = ParserState::Ground;
            }
            0x60..=0x7E => {
                // Final byte of an escape sequence (single-char or after intermediates)
                let intermediates = std::mem::take(&mut self.escape_intermediates);
                self.handle_escape_sequence(byte as char, &intermediates);
                self.state = ParserState::Ground;
            }
            _ => {
                self.escape_intermediates.clear();
                self.state = ParserState::Ground;
            }
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
            0x07 => {
                let buffer = std::mem::take(&mut self.osc_buffer);
                let osc = String::from_utf8_lossy(&buffer);
                self.handle_osc(&osc);
                self.state = ParserState::Ground;
            }
            0x1B => {
                // ST terminator (ESC \): consume the OSC, then stay in Escape so
                // the trailing '\' is swallowed instead of printed as text.
                let buffer = std::mem::take(&mut self.osc_buffer);
                let osc = String::from_utf8_lossy(&buffer);
                self.handle_osc(&osc);
                self.state = ParserState::Escape;
            }
            0x20..=0xFF if self.osc_buffer.len() < MAX_OSC_BUFFER => {
                self.osc_buffer.push(byte);
            }
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
                self.dcs_buffer.push(byte);
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
                self.dcs_buffer.push(byte);
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
                self.dcs_buffer.push(byte);
                self.state = ParserState::DcsPassthrough;
            }
            _ => self.state = ParserState::Ground,
        }
    }

    fn handle_dcs_passthrough(&mut self, byte: u8) {
        match byte {
            0x1B => {
                // ST terminator (ESC \): finish the DCS, stay in Escape so the
                // trailing '\' is swallowed.
                let buffer = std::mem::take(&mut self.dcs_buffer);
                let dcs = String::from_utf8_lossy(&buffer);
                self.handle_dcs_string(&dcs);
                self.state = ParserState::Escape;
            }
            _ => {
                if self.dcs_buffer.len() < MAX_DCS_BUFFER {
                    self.dcs_buffer.push(byte);
                }
            }
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
                // ST terminator (ESC \): finish the string, stay in Escape so the
                // trailing '\' is swallowed.
                let buffer = std::mem::take(&mut self.apc_buffer);
                self.handle_apc(&buffer);
                self.state = ParserState::Escape;
            }
            0x20..=0x7E if self.apc_buffer.len() < MAX_APC_BUFFER => {
                self.apc_buffer.push(byte as char);
            }
            _ => {}
        }
    }

    fn execute_control(&mut self, byte: u8) {
        match byte {
            0x07 => self.screen.bell(),        // BEL
            0x08 => self.screen.cursor_left(), // BS
            0x09 => self.screen.tab(),         // HT
            0x0A..=0x0C => {
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

    fn handle_escape_sequence(&mut self, ch: char, intermediates: &[char]) {
        match (ch, intermediates) {
            ('7', []) => self.screen.save_cursor(),    // DECSC
            ('8', []) => self.screen.restore_cursor(), // DECRC
            ('8', ['#']) => self.screen.decaln(),      // DECALN
            ('D', []) => self.screen.linefeed(),       // IND
            ('E', []) => {
                self.screen.carriage_return();
                self.screen.linefeed();
            } // NEL
            ('H', []) => self.screen.tab_set(),        // HTS
            ('M', []) => self.screen.reverse_linefeed(), // RI
            ('Z', []) => self.screen.identify(),       // DECID
            ('c', []) => self.screen.reset(),          // RIS
            // '(' ')' '*' '+' '$' '%' '#' prefixes select charsets / UTF-8 /
            // line attributes — consumed and ignored (cells are Unicode-native).
            _ => {}
        }
    }

    fn handle_csi(&mut self, final_byte: char, intermediates: &[char]) {
        let marker = intermediates
            .iter()
            .find(|&&c| c == '?' || c == '>' || c == '<' || c == '=')
            .copied();
        let private = marker.is_some();
        let intermediate: String = intermediates
            .iter()
            .filter(|&&c| c != '?' && c != '>' && c != '<' && c != '=')
            .collect();

        // Kitty keyboard protocol: `CSI ? u` (query flags), `CSI > f u` (push
        // flags), `CSI < n u` (pop n), `CSI = f;m u` (set flags with mode).
        if final_byte == 'u' && marker.is_some() {
            let p0 = self.csi.get(0, 0);
            let p1 = self.csi.get(1, 1);
            self.handle_kitty_csi(marker.unwrap(), p0, p1);
            return;
        }

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

            // DECSCUSR — cursor style: 0/1 blink block, 2 steady block,
            // 3 blink underline, 4 steady underline, 5 blink bar, 6 steady
            // bar. nvim/vi flip between block (normal) and bar (insert);
            // without this the cursor stayed a block everywhere.
            ('q', false, " ") => {
                match self.csi.get(0, 0) {
                    0..=2 => self.screen.set_cursor_shape(CursorShape::Block),
                    3 | 4 => self.screen.set_cursor_shape(CursorShape::Underline),
                    5 | 6 => self.screen.set_cursor_shape(CursorShape::Bar),
                    _ => {}
                }
            }

            // Scrolling
            ('S', false, "") => self.screen.scroll_up(self.csi.get(0, 1) as usize), // SU
            ('T', false, "") => self.screen.scroll_down(self.csi.get(0, 1) as usize), // SD

            // Erasing
            ('J', false, "") => {
                // ED 2 clears the visible screen, ED 3 clears scrollback:
                // either means the viewport is no longer valid, so latch a
                // flag the app drains to snap scroll to the bottom.
                self.screen.erase_display(self.csi.get(0, 0));
                self.clear_flag = true;
            }
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

            // Scroll region
            ('r', false, "") => {
                let top = self.csi.get(0, 0) as usize;
                let bottom = self.csi.get(1, 0) as usize;
                self.screen.set_scroll_region(top, bottom);
            } // DECSTBM

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
                            5 if i + 2 < self.csi.param_count() => {
                                // 256-color
                                let idx = self.csi.get(i + 2, 0) as u8;
                                self.screen.set_fg_256(idx);
                                i += 2;
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
                            5 if i + 2 < self.csi.param_count() => {
                                // 256-color
                                let idx = self.csi.get(i + 2, 0) as u8;
                                self.screen.set_bg_256(idx);
                                i += 2;
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
                2026 => self.sync_output = set,
                1049 => self.screen.set_alternate_screen(set),
                _ => {}
            }
        }
    }

    /// Kitty keyboard protocol state machine (CSI ?/>/</= u). The query reply
    /// advertises disambiguation (bit 0); push/pop/set track the app's
    /// requested enhancements so key encoding can follow.
    fn handle_kitty_csi(&mut self, marker: char, p0: i64, p1: i64) {
        if !self.kitty_supported {
            return;
        }
        match marker {
            '?' => {
                let flags = self.kitty_flags | 1; // advertise disambiguate
                self.pending_response = Some(format!("\x1b[?{}u", flags).into_bytes());
            }
            '>' => {
                self.kitty_stack.push(self.kitty_flags);
                self.kitty_flags = p0 as u32;
            }
            '<' => {
                for _ in 0..p0.max(1) {
                    if let Some(prev) = self.kitty_stack.pop() {
                        self.kitty_flags = prev;
                    } else {
                        self.kitty_flags = 0;
                    }
                }
            }
            '=' => {
                let flags = p0 as u32;
                self.kitty_flags = match p1 {
                    2 => self.kitty_flags | flags,
                    3 => self.kitty_flags & !flags,
                    _ => flags,
                };
            }
            _ => {}
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
            // OSC 8 hyperlink: `8;params;uri`. An empty uri closes the link.
            let rest = &osc[2..];
            let uri = match rest.split_once(';') {
                Some((_, uri)) => uri,
                None => rest, // no params: `8;uri`
            };
            self.screen.set_hyperlink(uri);
        } else if let Some(payload) = osc.strip_prefix("9;") {
            // OSC 9 desktop notification. Windows Terminal uses `9;0;text`
            // (urgency 0..2); strip an optional leading urgency digit.
            let text = if payload.len() >= 2
                && payload.as_bytes()[0].is_ascii_digit()
                && payload.as_bytes()[1] == b';'
            {
                &payload[2..]
            } else {
                payload
            };
            if !text.is_empty() {
                self.notification = Some(text.to_string());
            }
        } else if let Some(payload) = osc.strip_prefix("52;") {
            if let Some(semicolon) = payload.find(';') {
                let base64_data = &payload[semicolon + 1..];
                if !base64_data.is_empty() {
                    if let Ok(data) = decode_base64(base64_data) {
                        self.clipboard_text = Some(String::from_utf8_lossy(&data).to_string());
                    }
                }
            }
        } else if let Some(payload) = osc.strip_prefix("1337;") {
            // iTerm2 inline image protocol
            self.handle_iterm_image(payload);
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
        if let Ok(decoded) = decode_base64(data_part) {
            if decoded.is_empty() {
                return;
            }
            let frames = image_decode::decode_frames(&decoded)
                .map(|d| d.frames)
                .unwrap_or_default();
            let id = if frames.is_empty() {
                self.screen.place_image(decoded.clone(), width, height)
            } else {
                self.screen
                    .place_image_frames(frames.clone(), width, height)
            };
            self.pending_images.push(ImageFragment {
                id,
                data: decoded,
                width,
                height,
                frames,
            });
        }
    }

    fn handle_iterm_image(&mut self, payload: &str) {
        let body = match payload.strip_prefix("File=") {
            Some(b) => b,
            None => return,
        };
        let (header, data_part) = match body.split_once(':') {
            Some(hd) => hd,
            None => return,
        };
        let mut width = 0u32;
        let mut height = 0u32;
        let mut inline = true;
        for pair in header.split(';') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "width" => width = v.parse().unwrap_or(0),
                    "height" => height = v.parse().unwrap_or(0),
                    // inline=0 means download-only in iTerm; display nothing
                    "inline" => inline = v == "1",
                    _ => {}
                }
            }
        }
        if !inline || data_part.is_empty() {
            return;
        }
        if let Ok(decoded) = decode_base64(data_part) {
            if decoded.is_empty() {
                return;
            }
            // Decode into RGBA frames; on failure fall back to dimension
            // sniffing only (pre-existing behavior for non-image payloads).
            let (mut width, mut height, frames) = match image_decode::decode_frames(&decoded) {
                Ok(d) => {
                    let dims = d
                        .frames
                        .first()
                        .map(|f| (f.width, f.height))
                        .unwrap_or((0, 0));
                    let (w, h) = if width == 0 || height == 0 {
                        dims
                    } else {
                        (width, height)
                    };
                    (w, h, d.frames)
                }
                Err(_) => {
                    let (w, h) = if width == 0 || height == 0 {
                        png_dimensions(&decoded)
                    } else {
                        (width, height)
                    };
                    (w, h, Vec::new())
                }
            };
            width = width.max(1);
            height = height.max(1);
            let id = if frames.is_empty() {
                self.screen.place_image(decoded.clone(), width, height)
            } else {
                self.screen
                    .place_image_frames(frames.clone(), width, height)
            };
            self.pending_images.push(ImageFragment {
                id,
                data: decoded,
                width,
                height,
                frames,
            });
        }
    }

    fn handle_dcs_string(&mut self, data: &str) {
        if let Some(data) = data.strip_prefix("q") {
            self.handle_sixel(data);
        }
    }

    fn handle_sixel(&mut self, data: &str) {
        let mut palette: [(u8, u8, u8); 256] = [(0, 0, 0); 256];
        let mut pal_idx: u8 = 0;
        let mut pixels: Vec<Vec<u32>> = Vec::new();
        let mut x: u32 = 0;
        let mut y: u32 = 0;
        let mut max_x: u32 = 0;
        let mut last_data: Option<u8> = None;
        let mut written: u64 = 0;
        let bytes: Vec<char> = data.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                // `Pc;r;g;b` — set color register c (r,g,b are 0-100 percent).
                'P' => {
                    if i + 1 < bytes.len() {
                        i += 1;
                        let mut idx: u8 = 0;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            idx = idx.saturating_mul(10).saturating_add(bytes[i] as u8 - b'0');
                            i += 1;
                        }
                        let mut comps: [u32; 3] = [0; 3];
                        let mut ci = 0;
                        while ci < 3 && i < bytes.len() {
                            if bytes[i] == ';' {
                                i += 1;
                            }
                            if i < bytes.len() && bytes[i] == '#' {
                                // Hex form `Pc;#RRGGBB`
                                let mut hex = String::new();
                                i += 1;
                                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                                    hex.push(bytes[i]);
                                    i += 1;
                                }
                                comps[ci] = u32::from_str_radix(&hex, 16).unwrap_or(0);
                                ci += 1;
                                continue;
                            }
                            let mut v = 0u32;
                            while i < bytes.len() && bytes[i].is_ascii_digit() {
                                v = v
                                    .saturating_mul(10)
                                    .saturating_add(bytes[i] as u32 - '0' as u32);
                                i += 1;
                            }
                            comps[ci] = v;
                            ci += 1;
                        }
                        let to8 = |v: u32| (v * 255 / 100).min(255) as u8;
                        palette[idx as usize] = (to8(comps[0]), to8(comps[1]), to8(comps[2]));
                    }
                }
                // `#n` — select palette register n.
                '#' => {
                    i += 1;
                    let mut idx: u8 = 0;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        idx = idx.saturating_mul(10).saturating_add(bytes[i] as u8 - b'0');
                        i += 1;
                    }
                    pal_idx = idx;
                }
                ';' | ':' => i += 1,
                '$' | '-' => {
                    x = 0;
                    y += 6;
                    i += 1;
                }
                '!' => {
                    i += 1;
                    let mut n: u64 = 0;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        n = n
                            .saturating_mul(10)
                            .saturating_add(bytes[i] as u64 - '0' as u64);
                        i += 1;
                    }
                    let n = n.min(MAX_SIXEL_W as u64);
                    if let Some(rep) = last_data {
                        for _ in 0..n {
                            draw_sixel(
                                &mut pixels,
                                &mut x,
                                &mut max_x,
                                &mut written,
                                y,
                                rep,
                                &palette,
                                pal_idx,
                            );
                        }
                    }
                }
                c if ('?'..='~').contains(&c) => {
                    last_data = Some(c as u8);
                    draw_sixel(
                        &mut pixels,
                        &mut x,
                        &mut max_x,
                        &mut written,
                        y,
                        c as u8,
                        &palette,
                        pal_idx,
                    );
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let height = pixels.len() as u32;
        let width = max_x.min(MAX_SIXEL_W);
        if width == 0 || height == 0 {
            return;
        }
        if (width as u64) * (height as u64) * 4 > MAX_SIXEL_BYTES as u64 {
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
            frames: Vec::new(),
        });
    }

    pub fn take_pending_images(&mut self) -> Vec<ImageFragment> {
        std::mem::take(&mut self.pending_images)
    }
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.clipboard_text.take()
    }
}

// 8 positional args is the natural sixel-state signature; clippy's 7-arg cap
// isn't worth wrapping these in a struct here.
#[allow(clippy::too_many_arguments)]
fn draw_sixel(
    pixels: &mut Vec<Vec<u32>>,
    x: &mut u32,
    max_x: &mut u32,
    written: &mut u64,
    y: u32,
    data_byte: u8,
    palette: &[(u8, u8, u8); 256],
    pal_idx: u8,
) {
    let sixel = data_byte - 63;
    for bit in 0..6 {
        if (sixel >> bit) & 1 != 0 {
            let py = y + bit;
            if py < MAX_SIXEL_H && *x < MAX_SIXEL_W && *written < (MAX_SIXEL_BYTES as u64 / 4) {
                while pixels.len() <= py as usize {
                    pixels.push(Vec::new());
                }
                let row = &mut pixels[py as usize];
                while row.len() <= *x as usize {
                    row.push(0);
                }
                let (r, g, b) = palette[pal_idx as usize];
                row[*x as usize] = 0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                *written += 1;
            }
        }
    }
    *x += 1;
    if *x > *max_x {
        *max_x = *x;
    }
}

fn png_dimensions(data: &[u8]) -> (u32, u32) {
    const SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    // IHDR: 8-byte signature + 4-byte length + 4-byte "IHDR", then width/height
    if data.len() < 24 || data[..8] != SIG {
        return (0, 0);
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (w, h)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, ()> {
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
    Ok(output)
}
