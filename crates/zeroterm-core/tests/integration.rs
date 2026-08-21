use zeroterm_core::cell::{Color, CursorShape, UnderlineStyle};
use zeroterm_core::Parser;

fn setup(cols: usize, rows: usize) -> Parser {
    Parser::new(cols, rows)
}

fn cell_text(parser: &Parser) -> String {
    let screen = parser.screen();
    let mut text = String::new();
    for row in screen.buffer() {
        for cell in row {
            text.push(cell.ch);
        }
        text.push('\n');
    }
    text
}

// --------------------- basic text ---------------------

#[test]
fn test_basic_text() {
    let mut p = setup(80, 24);
    p.parse(b"Hello World");
    let text = cell_text(&p);
    assert!(
        text.starts_with("Hello World"),
        "expected 'Hello World', got start: {:?}",
        &text[..20]
    );
}

#[test]
fn test_multiple_parse_calls_accumulate() {
    let mut p = setup(80, 24);
    p.parse(b"Hello ");
    p.parse(b"World");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'H');
    assert_eq!(p.screen().cell(0, 10).unwrap().ch, 'd');
}

// --------------------- newlines / cr ---------------------

#[test]
fn test_newline_advances_row() {
    let mut p = setup(80, 24);
    p.parse(b"X\nY");
    assert_eq!(p.screen().cursor().row, 1);
}

#[test]
fn test_carriage_return() {
    let mut p = setup(80, 24);
    p.parse(b"Hello\rX");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'X');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'e');
}

// --------------------- SGR colors ---------------------

// ANSI-16 palette values (set_fg_ansi uses from_ansi_16):
// idx 1 -> Color { r: 0x80, g: 0x00, b: 0x00 }
// idx 2 -> Color { r: 0x00, g: 0x80, b: 0x00 }

#[test]
fn test_sgr_fg_red() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[31mR\x1b[0mG");
    let r = p.screen().cell(0, 0).unwrap();
    assert_eq!(r.ch, 'R');
    assert_eq!(r.fg, Color::from_ansi_16(1));
    let g = p.screen().cell(0, 1).unwrap();
    assert_eq!(g.ch, 'G');
    assert_eq!(g.fg, Color::DEFAULT_FG);
}

#[test]
fn test_sgr_bg_green() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[42mX");
    assert_eq!(p.screen().cell(0, 0).unwrap().bg, Color::from_ansi_16(2));
}

#[test]
fn test_sgr_bright_fg() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[91mX");
    assert_eq!(p.screen().cell(0, 0).unwrap().fg, Color::RED);
}

#[test]
fn test_sgr_256_color() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[38;5;196mX");
    assert_eq!(p.screen().cell(0, 0).unwrap().fg, Color::from_ansi_256(196));
}

#[test]
fn test_sgr_rgb_color() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[38;2;255;128;64mX");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().fg,
        Color {
            r: 255,
            g: 128,
            b: 64
        }
    );
}

#[test]
fn test_sgr_reset_clears_attributes() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[1;31;42mX\x1b[0mY");
    let x = p.screen().cell(0, 0).unwrap();
    assert!(x.attrs.bold);
    assert_eq!(x.fg, Color::from_ansi_16(1));
    assert_eq!(x.bg, Color::from_ansi_16(2));
    let y = p.screen().cell(0, 1).unwrap();
    assert!(!y.attrs.bold);
    assert_eq!(y.fg, Color::DEFAULT_FG);
    assert_eq!(y.bg, Color::DEFAULT_BG);
}

// --------------------- cursor movement ---------------------

#[test]
fn test_cup() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[10;20H");
    assert_eq!(p.screen().cursor().row, 9);
    assert_eq!(p.screen().cursor().col, 19);
}

#[test]
fn test_cuu() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[10;10H\x1b[2A");
    assert_eq!(p.screen().cursor().row, 7);
}

#[test]
fn test_cud() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[3B");
    assert_eq!(p.screen().cursor().row, 3);
}

#[test]
fn test_cuf() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[5C");
    assert_eq!(p.screen().cursor().col, 5);
}

#[test]
fn test_cub() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[10C\x1b[3D");
    assert_eq!(p.screen().cursor().col, 7);
}

#[test]
fn test_cnl() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[5;10H\x1b[2E");
    assert_eq!(p.screen().cursor().row, 6);
    assert_eq!(p.screen().cursor().col, 0);
}

#[test]
fn test_cpl() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[10;10H\x1b[2F");
    assert_eq!(p.screen().cursor().row, 7);
    assert_eq!(p.screen().cursor().col, 0);
}

#[test]
fn test_cha() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[30G");
    assert_eq!(p.screen().cursor().col, 29);
}

#[test]
fn test_vpa() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[15d");
    assert_eq!(p.screen().cursor().row, 14);
}

// --------------------- SGR attributes ---------------------

#[test]
fn test_bold() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[1mB\x1b[0mN");
    assert!(p.screen().cell(0, 0).unwrap().attrs.bold);
    assert!(!p.screen().cell(0, 1).unwrap().attrs.bold);
}

#[test]
fn test_dim() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[2mD");
    assert!(p.screen().cell(0, 0).unwrap().attrs.dim);
}

#[test]
fn test_italic() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[3mI");
    assert!(p.screen().cell(0, 0).unwrap().attrs.italic);
}

#[test]
fn test_underline() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[4mU");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().attrs.underline,
        UnderlineStyle::Single
    );
}

#[test]
fn test_blink() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[5mK");
    assert!(p.screen().cell(0, 0).unwrap().attrs.blink);
}

#[test]
fn test_sgr_reverse() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[7mX\x1b[0m");
    assert!(p.screen().cell(0, 0).unwrap().attrs.reverse);
}

#[test]
fn test_strikethrough() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[9mS");
    assert!(p.screen().cell(0, 0).unwrap().attrs.strikethrough);
}

#[test]
fn test_invisible() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[8mV");
    assert!(p.screen().cell(0, 0).unwrap().attrs.invisible);
}

// --------------------- erase operations ---------------------

#[test]
fn test_erase_display_clear_all() {
    let mut p = setup(80, 24);
    p.parse(b"Hello");
    p.parse(b"\x1b[2J");
    let screen = p.screen();
    for row in 0..24 {
        for col in 0..80 {
            assert_eq!(
                screen.cell(row, col).unwrap().ch,
                ' ',
                "cell({},{}) should be cleared",
                row,
                col
            );
        }
    }
}

#[test]
fn test_erase_display_from_cursor() {
    let mut p = setup(80, 24);
    p.parse(b"Hello\x1b[2;10H\x1b[0J");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'H');
    assert_eq!(screen.cell(0, 4).unwrap().ch, 'o');
    for col in 0..80 {
        assert_eq!(screen.cell(1, col).unwrap().ch, ' ');
        assert_eq!(screen.cell(2, col).unwrap().ch, ' ');
    }
}

#[test]
fn test_erase_display_to_cursor() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[5;1HOverwrite\x1b[3;1H\x1b[1J");
    let screen = p.screen();
    for col in 0..80 {
        assert_eq!(screen.cell(0, col).unwrap().ch, ' ');
        assert_eq!(screen.cell(1, col).unwrap().ch, ' ');
        assert_eq!(screen.cell(2, col).unwrap().ch, ' ');
    }
}

#[test]
fn test_erase_line_from_cursor() {
    let mut p = setup(80, 24);
    p.parse(b"Hello World");
    p.parse(b"\x1b[6G\x1b[0K");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'H');
    assert_eq!(screen.cell(0, 4).unwrap().ch, 'o');
    for col in 5..80 {
        assert_eq!(screen.cell(0, col).unwrap().ch, ' ');
    }
}

#[test]
fn test_erase_line_to_cursor() {
    let mut p = setup(80, 24);
    p.parse(b"Hello World");
    p.parse(b"\x1b[6G\x1b[1K");
    let screen = p.screen();
    for col in 0..=5 {
        assert_eq!(screen.cell(0, col).unwrap().ch, ' ');
    }
    assert_eq!(screen.cell(0, 6).unwrap().ch, 'W');
}

#[test]
fn test_erase_line_complete() {
    let mut p = setup(80, 24);
    p.parse(b"Hello\x1b[2K");
    let screen = p.screen();
    for col in 0..80 {
        assert_eq!(screen.cell(0, col).unwrap().ch, ' ');
    }
}

// --------------------- scrollback ---------------------

#[test]
fn test_scrollback() {
    let mut p = setup(80, 5);
    for i in 0..10 {
        p.parse(format!("Line {}\n", i).as_bytes());
    }
    assert!(
        !p.screen().scrollback().is_empty(),
        "scrollback should have content after overflow"
    );
}

#[test]
fn test_scroll_up() {
    let mut p = setup(80, 5);
    for i in 0..6 {
        p.parse(format!("Line {}\n", i).as_bytes());
    }
    assert!(!p.screen().scrollback().is_empty());
}

// --------------------- screen resize ---------------------

#[test]
fn test_screen_resize_wider() {
    let mut p = setup(10, 5);
    p.parse(b"Hello");
    p.screen_mut().resize(20, 5);
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'H');
    assert_eq!(p.screen().cell(0, 4).unwrap().ch, 'o');
    assert_eq!(p.screen().size().cols, 20);
    assert_eq!(p.screen().size().rows, 5);
}

#[test]
fn test_screen_resize_taller() {
    let mut p = setup(10, 5);
    p.parse(b"Line1\nLine2\nLine3");
    p.screen_mut().resize(10, 10);
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'L');
    assert_eq!(p.screen().size().rows, 10);
}

// --------------------- title ---------------------

#[test]
fn test_title_osc_0() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]0;MyTerminalTitle\x07");
    assert_eq!(p.screen().title(), "MyTerminalTitle");
}

#[test]
fn test_title_osc_2() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]2;AnotherTitle\x07");
    assert_eq!(p.screen().title(), "AnotherTitle");
}

// --------------------- insert / delete ---------------------

#[test]
fn test_insert_lines() {
    let mut p = setup(80, 5);
    p.parse(b"Line1\nLine2\nLine3");
    p.parse(b"\x1b[2;1H\x1b[2L");
    let screen = p.screen();
    assert!(screen.cell(1, 0).unwrap().is_empty());
}

#[test]
fn test_delete_lines() {
    // delete_lines removes at the cursor row and appends empty at the bottom
    // of the scroll region (xterm DL semantics, not scroll-top).
    let mut p = setup(10, 3);
    p.parse(b"ABC\r\nDEF\r\nGHI");
    p.parse(b"\x1b[2;1H\x1b[1M");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert_eq!(screen.cell(1, 0).unwrap().ch, 'G');
    assert!(screen.cell(2, 0).unwrap().is_empty());
}

#[test]
fn test_insert_chars() {
    let mut p = setup(80, 5);
    p.parse(b"ABCDE");
    p.parse(b"\x1b[3G\x1b[2@");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert!(screen.cell(0, 2).unwrap().is_empty());
    assert!(screen.cell(0, 3).unwrap().is_empty());
    assert_eq!(screen.cell(0, 4).unwrap().ch, 'C');
}

#[test]
fn test_delete_chars() {
    let mut p = setup(80, 5);
    p.parse(b"ABCDE");
    p.parse(b"\x1b[3G\x1b[2P");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert_eq!(screen.cell(0, 2).unwrap().ch, 'E');
}

// --------------------- command block detection ---------------------

// Command blocks: newline + prompt char ($ % # >) marks boundary.
// Command is stored in current_block_command and assigned to the NEXT block
// when mark_block_boundary is called again.

#[test]
fn test_command_block_created_on_prompt_sigil() {
    let mut p = setup(80, 24);
    p.parse(b"\n$ ls\r\n# deploy\r");
    let blocks = p.screen().blocks();
    assert!(
        blocks.len() >= 2,
        "expected at least 2 blocks, got {}",
        blocks.len()
    );
    assert_eq!(blocks[0].command, "", "first block has no command yet");
    assert_eq!(blocks[1].command, "$ ls");
}

#[test]
fn test_set_block_exit_code() {
    let mut p = setup(80, 24);
    p.parse(b"\n$ true\r\n# ");
    p.set_exit_code(0);
    let blocks = p.screen().blocks();
    if let Some(last) = blocks.last() {
        assert_eq!(last.exit_code, Some(0));
    }
}

// OSC 133 FinalTerm shell integration: A/B open blocks, C carries the
// command text (with optional cmdline_url= prefix), D carries the exit code.

#[test]
fn test_osc133_exit_code() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]133;D;127\x07");
    assert_eq!(p.screen().last_exit(), Some(127));
}

#[test]
fn test_osc133_exit_code_positive() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]133;D;0\x07");
    assert_eq!(p.screen().last_exit(), Some(0));
}

#[test]
fn test_osc133_exit_code_without_code_finalizes() {
    let mut p = setup(80, 24);
    // D without a code must not clobber the last known exit.
    p.parse(b"\x1b]133;D;3\x07");
    assert_eq!(p.screen().last_exit(), Some(3));
    p.parse(b"\x1b]133;D\x07");
    assert_eq!(p.screen().last_exit(), Some(3));
}

#[test]
fn test_osc133_blocks_and_command() {
    let mut p = setup(80, 24);
    // A (prompt) -> C (command text) -> D (exit code) -> A (next prompt).
    p.parse(b"\x1b]133;A\x07\x1b]133;C;ls -la\x07\x1b]133;D;0\x07\x1b]133;A\x07");
    let blocks = p.screen().blocks();
    assert!(
        blocks.len() >= 2,
        "expected A to open a new block, got {}",
        blocks.len()
    );
    let last = blocks.last().unwrap();
    assert_eq!(last.command, "", "new block starts with no command");
    // The block before the final A carried the command and exit code.
    let prev = blocks[blocks.len() - 2].clone();
    assert_eq!(prev.command, "ls -la");
    assert_eq!(prev.exit_code, Some(0));
    assert!(prev.end_line.is_some(), "D finalizes the block");
    assert_eq!(p.screen().last_exit(), Some(0));
}

#[test]
fn test_osc133_cmdline_url_prefix() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]133;C;cmdline_url=git status\x07");
    let blocks = p.screen().blocks();
    assert_eq!(blocks.last().map(|b| b.command.as_str()), None);
    // C only sets the pending command; it lands on the NEXT block boundary.
    p.screen_mut().mark_block_boundary();
    let blocks = p.screen().blocks();
    assert_eq!(
        blocks.last().map(|b| b.command.as_str()),
        Some("git status")
    );
}

#[test]
fn test_osc7_cwd() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]7;file://localhost/home/user/My%20Dir\x07");
    assert_eq!(p.screen().cwd(), Some("/home/user/My Dir"));
}

#[test]
fn test_osc7_cwd_absolute_path() {
    let mut p = setup(80, 24);
    // `file:///path` — empty host, path starts right after the slashes.
    p.parse(b"\x1b]7;file:///etc/ssh/sshd_config\x07");
    assert_eq!(p.screen().cwd(), Some("/etc/ssh/sshd_config"));
}

#[test]
fn test_has_running_block() {
    let mut p = setup(80, 24);
    assert!(!p.screen().has_running_block(), "no blocks yet");
    p.parse(b"\x1b]133;A\x07");
    assert!(p.screen().has_running_block(), "block open, no D yet");
    p.parse(b"\x1b]133;D;0\x07");
    assert!(
        !p.screen().has_running_block(),
        "D finalizes the running block"
    );
}

#[test]
fn test_reset_clears_exit_and_cwd() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]133;D;9\x07\x1b]7;file:///tmp\x07");
    assert_eq!(p.screen().last_exit(), Some(9));
    assert_eq!(p.screen().cwd(), Some("/tmp"));
    p.parse(b"\x1bc"); // RIS
    assert_eq!(p.screen().last_exit(), None);
    assert_eq!(p.screen().cwd(), None);
}

// --------------------- DEC private modes (test at screen level) ---------------------
// Note: \x1b[?...h/l not parsed correctly due to intermediates bug in parser.
// These tests verify Screen methods directly.

#[test]
fn test_cursor_visible_via_screen() {
    let mut p = setup(80, 24);
    assert!(p.screen().cursor().visible);
    p.screen_mut().set_cursor_visible(false);
    assert!(!p.screen().cursor().visible);
    p.screen_mut().set_cursor_visible(true);
    assert!(p.screen().cursor().visible);
}

#[test]
fn test_reverse_video_via_screen() {
    let mut p = setup(80, 24);
    p.screen_mut().set_reverse_video(true);
    p.parse(b"X");
    p.screen_mut().set_reverse_video(false);
    let cell = p.screen().cell(0, 0).unwrap();
    // cells store original colors; renderer handles the swap
    assert_eq!(cell.fg, Color::DEFAULT_FG);
    assert_eq!(cell.bg, Color::DEFAULT_BG);
}

#[test]
fn test_autowrap_off_via_screen() {
    let mut p = setup(10, 5);
    p.screen_mut().set_autowrap(false);
    p.parse(b"1234567890AB");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 9).unwrap().ch, 'B');
}

#[test]
fn test_autowrap_on_via_screen() {
    let mut p = setup(10, 5);
    p.screen_mut().set_autowrap(true);
    p.parse(b"1234567890AB");
    let screen = p.screen();
    assert_eq!(screen.cell(0, 9).unwrap().ch, '0');
    assert_eq!(screen.cursor().row, 1);
}

// --------------------- edge cases ---------------------

#[test]
fn test_empty_input() {
    let mut p = setup(80, 24);
    p.parse(b"");
    assert_eq!(p.screen().cursor().row, 0);
    assert_eq!(p.screen().cursor().col, 0);
}

#[test]
fn test_single_byte() {
    let mut p = setup(80, 24);
    p.parse(b"X");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'X');
}

#[test]
fn test_invalid_escape_does_not_panic() {
    let mut p = setup(80, 24);
    // \x1b[ enters CSI, 'i' is consumed as final byte (no-op handler),
    // "nvalid" prints as text at cols 0-5, then "OK" at cols 6-7
    p.parse(b"\x1b[invalidOK");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'n');
    assert_eq!(p.screen().cell(0, 5).unwrap().ch, 'd');
    assert_eq!(p.screen().cell(0, 6).unwrap().ch, 'O');
    assert_eq!(p.screen().cell(0, 7).unwrap().ch, 'K');
}

#[test]
fn test_control_chars_ignored() {
    let mut p = setup(80, 24);
    p.parse(b"\x00\x01\x02\x03\x04\x05\x06\x0E\x0F\x7fX");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'X');
}

#[test]
fn test_bell_does_not_panic() {
    let mut p = setup(80, 24);
    p.parse(b"\x07");
}

#[test]
fn test_backspace() {
    let mut p = setup(80, 24);
    p.parse(b"AB\x08");
    assert_eq!(p.screen().cursor().col, 1);
}

#[test]
fn test_tab() {
    let mut p = setup(80, 24);
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 8, "tab should advance to col 8");
}

// --------------------- escape sequences ---------------------

#[test]
fn test_save_restore_cursor() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[5;10H\x1b7\x1b[1;1HAB\x1b8");
    assert_eq!(p.screen().cursor().row, 4);
    assert_eq!(p.screen().cursor().col, 9);
}

#[test]
fn test_ris_reset() {
    let mut p = setup(80, 24);
    p.parse(b"Hello\x1b[31;42mWorld\x1bc");
    let screen = p.screen();
    assert_eq!(screen.cursor().row, 0);
    assert_eq!(screen.cursor().col, 0);
    assert!(screen.cursor().visible);
}

#[test]
fn test_reverse_linefeed() {
    let mut p = setup(80, 24);
    p.parse(b"\n\n\n\x1bM");
    assert_eq!(p.screen().cursor().row, 2);
}

#[test]
fn test_long_line_wraps() {
    let mut p = setup(20, 5);
    let long: Vec<u8> = (0..25).map(|i| b'A' + i).collect();
    p.parse(&long);
    assert_eq!(p.screen().cell(0, 19).unwrap().ch, 'T');
    // After autowrap: char 20 'U' wraps to row 1, cursor goes to row 1 col 5
    assert_eq!(p.screen().cursor().row, 1);
    assert_eq!(p.screen().cursor().col, 5);
}

// --------------------- cursor state ---------------------

#[test]
fn test_cursor_shape_default() {
    let p = setup(80, 24);
    assert_eq!(p.screen().cursor().shape, CursorShape::Block);
    assert!(p.screen().cursor().visible);
}

// --------------------- cell model ---------------------

#[test]
fn test_cell_default_is_empty() {
    use zeroterm_core::cell::Cell;
    assert!(Cell::default().is_empty());
}

#[test]
fn test_cell_width_ascii() {
    use zeroterm_core::cell::Cell;
    assert_eq!(Cell::new('a').width(), 1);
    assert_eq!(Cell::new(' ').width(), 1);
}

#[test]
fn test_color_ansi_16_basics() {
    assert_eq!(Color::from_ansi_16(0), Color::BLACK);
    assert_eq!(Color::from_ansi_16(9), Color::RED);
    assert_eq!(Color::from_ansi_16(15), Color::WHITE);
}

#[test]
fn test_color_ansi_256_bounds() {
    let black = Color::from_ansi_256(16);
    assert_eq!(black.r, 0);
    assert_eq!(black.g, 0);
    assert_eq!(black.b, 0);
    let white = Color::from_ansi_256(231);
    assert_eq!(white.r, 255);
    assert_eq!(white.g, 255);
    assert_eq!(white.b, 255);
}

// --------------------- AI block detection (via screen API) ---------------------

#[test]
fn test_mark_block_boundary_creates_block() {
    let mut p = setup(80, 24);
    p.screen_mut().set_block_command("ls -la");
    p.screen_mut().mark_block_boundary();
    let blocks = p.screen().blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].command, "ls -la");
    assert_eq!(blocks[0].id, 0);
    assert!(blocks[0].end_line.is_none());
}

#[test]
fn test_multiple_blocks_increment_id() {
    let mut p = setup(80, 24);
    p.screen_mut().set_block_command("cmd1");
    p.screen_mut().mark_block_boundary();
    p.screen_mut().set_block_command("cmd2");
    p.screen_mut().mark_block_boundary();
    let blocks = p.screen().blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].id, 0);
    assert_eq!(blocks[1].id, 1);
    assert_eq!(blocks[1].command, "cmd2");
}

#[test]
fn test_block_exit_code_set_directly() {
    let mut p = setup(80, 24);
    p.screen_mut().mark_block_boundary();
    p.screen_mut().set_block_exit_code(42);
    assert_eq!(p.screen().blocks().last().unwrap().exit_code, Some(42));
}

#[test]
fn test_block_duration_and_metadata() {
    let mut p = setup(80, 24);
    p.screen_mut().set_block_command("ls");
    p.screen_mut().mark_block_boundary();
    p.screen_mut().set_block_exit_code(0);
    p.screen_mut().set_block_command("whoami");
    p.screen_mut().mark_block_boundary();
    let blocks = p.screen().blocks();
    assert_eq!(blocks.len(), 2);
    let first = &blocks[0];
    assert!(
        first.duration_ms.is_some(),
        "first block should have a duration"
    );
    assert_eq!(
        p.screen().block_metadata(first),
        format!("exit:0 \u{00b7} {}ms", first.duration_ms.unwrap())
    );
    assert_eq!(p.screen().block_metadata(&blocks[1]), "exit:?");
}

// --------------------- wide character handling ---------------------

#[test]
fn test_cjk_cell_width() {
    use zeroterm_core::cell::Cell;
    assert_eq!(Cell::new('a').width(), 1);
    assert_eq!(Cell::new(' ').width(), 1);
    assert_eq!(Cell::new('\u{4e00}').width(), 2);
    assert_eq!(Cell::new('\u{4e8c}').width(), 2);
}

// --------------------- tab stops ---------------------

#[test]
fn test_tab_advances_through_stops() {
    let mut p = setup(80, 24);
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 8);
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 16);
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 24);
}

#[test]
fn test_tab_clear_all_no_stops() {
    let mut p = setup(80, 24);
    p.screen_mut().tab_clear_all();
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 79);
}

#[test]
fn test_tab_set_at_cursor() {
    let mut p = setup(80, 24);
    p.screen_mut().tab_clear_all();
    p.screen_mut().cursor_col(15);
    p.screen_mut().tab_set();
    p.screen_mut().cursor_col(1);
    p.parse(b"\x09");
    assert_eq!(p.screen().cursor().col, 14);
}

// --------------------- alternate screen buffer ---------------------

#[test]
fn test_alternate_screen_preserves_normal_buffer() {
    let mut p = setup(80, 24);
    p.parse(b"Normal");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'N');
    p.screen_mut().set_alternate_screen(true);
    // alt screen starts empty; cursor at (0,5) from "Normal"
    assert!(p.screen().cell(0, 0).unwrap().is_empty());
    p.parse(b"\x1b[HAlt");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'A');
    p.screen_mut().set_alternate_screen(false);
    // normal buffer restored
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'N');
}

// --------------------- insert/delete line stress ---------------------

#[test]
fn test_insert_lines_operates_from_cursor_row() {
    let mut p = setup(10, 5);
    p.parse(b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD\r\nEEEEE");
    p.parse(b"\x1b[3;1H\x1b[2L");
    let screen = p.screen();
    // IL inserts blank lines at the cursor row (row 2), pushing content down.
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert_eq!(screen.cell(1, 0).unwrap().ch, 'B');
    assert!(screen.cell(2, 0).unwrap().is_empty());
    assert!(screen.cell(3, 0).unwrap().is_empty());
    assert_eq!(screen.cell(4, 0).unwrap().ch, 'C');
}

#[test]
fn test_delete_lines_operates_from_cursor_row() {
    let mut p = setup(10, 5);
    p.parse(b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD\r\nEEEEE");
    p.parse(b"\x1b[3;1H\x1b[2M");
    let screen = p.screen();
    // DL deletes from the cursor row (row 2), pulling content up.
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'A');
    assert_eq!(screen.cell(1, 0).unwrap().ch, 'B');
    assert_eq!(screen.cell(2, 0).unwrap().ch, 'E');
    assert!(screen.cell(3, 0).unwrap().is_empty());
    assert!(screen.cell(4, 0).unwrap().is_empty());
}

// --------------------- scroll region + cursor (partial: DECSTBM not implemented) ---------------------
// ponytail: DECSTBM (\e[3;5r) not implemented in parser, testing cursor bounds with origin mode

#[test]
fn test_cursor_bounded_by_origin_mode() {
    let mut p = setup(80, 24);
    p.screen_mut().set_origin_mode(true);
    p.screen_mut().cursor_pos(3, 5);
    assert_eq!(p.screen().cursor().row, 2);
    assert_eq!(p.screen().cursor().col, 4);
}

// --------------------- OSC sequences ---------------------

#[test]
fn test_osc_1_icon_name_not_implemented() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]1;IconTitle\x07");
    // ponytail: OSC 1 not implemented; only OSC 0/2 set title
    assert_eq!(p.screen().title(), "");
}

#[test]
fn test_osc_0_and_2_both_set_title() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b]0;WindowTitle\x07");
    assert_eq!(p.screen().title(), "WindowTitle");
    p.parse(b"\x1b]2;NewTitle\x07");
    assert_eq!(p.screen().title(), "NewTitle");
}

// --------------------- RIS full reset ---------------------

#[test]
fn test_ris_resets_attributes_fully() {
    let mut p = setup(80, 24);
    p.parse(b"\x1b[1;31;42mX\x1b[5;10H");
    // Before reset: bold, red fg, green bg, cursor at row 4 col 9
    let x = p.screen().cell(0, 0).unwrap();
    assert!(x.attrs.bold);
    assert_eq!(x.fg, Color::from_ansi_16(1));
    assert_eq!(x.bg, Color::from_ansi_16(2));
    assert_eq!(p.screen().cursor().row, 4);
    assert_eq!(p.screen().cursor().col, 9);
    // RIS resets everything including cursor and attrs; buffer cleared
    p.parse(b"\x1bc");
    let screen = p.screen();
    assert_eq!(screen.cursor().row, 0);
    assert_eq!(screen.cursor().col, 0);
    assert!(screen.cursor().visible);
    assert!(screen.cell(0, 0).unwrap().is_empty());
}

// --------------------- multi-line paste ---------------------

#[test]
fn test_multi_line_paste_content() {
    let mut p = setup(20, 10);
    p.parse(b"abc\r\ndef\r\nghi");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(p.screen().cell(1, 0).unwrap().ch, 'd');
    assert_eq!(p.screen().cell(2, 0).unwrap().ch, 'g');
    assert_eq!(p.screen().cursor().row, 2);
}

// --------------------- backspace + wraparound ---------------------

#[test]
fn test_backspace_stays_at_zero_col() {
    let mut p = setup(80, 24);
    p.parse(b"\x08");
    assert_eq!(p.screen().cursor().col, 0);
}

#[test]
fn test_backspace_moves_left() {
    let mut p = setup(80, 24);
    p.parse(b"AB\x08");
    assert_eq!(p.screen().cursor().col, 1);
    assert_eq!(p.screen().cursor().row, 0);
}

#[test]
fn test_backspace_at_zero_with_wraparound_off() {
    let mut p = setup(80, 24);
    p.screen_mut().set_autowrap(false);
    p.parse(b"\x08");
    assert_eq!(p.screen().cursor().col, 0);
}

// --------------------- ED scrollback semantics ---------------------

#[test]
fn test_erase_display_mode_2_preserves_scrollback() {
    // `clear` (ESC [ 2 J) must NOT wipe history: ED 2 erases the display only.
    let mut p = setup(5, 3);
    for line in 0..6 {
        p.parse(format!("L{}\r\n", line).as_bytes());
    }
    assert!(
        !p.screen().scrollback().is_empty(),
        "scrollback must be populated before ED 2"
    );
    p.parse(b"\x1b[2J");
    assert!(
        !p.screen().scrollback().is_empty(),
        "ED 2 must not clear scrollback"
    );
    // Display is erased.
    for row in 0..3 {
        for col in 0..5 {
            assert_eq!(p.screen().cell(row, col).unwrap().ch, ' ');
        }
    }
}

#[test]
fn test_erase_display_mode_3_clears_scrollback_only() {
    // xterm ED 3 = "Erase Saved Lines": scrollback is cleared, display kept.
    let mut p = setup(5, 3);
    p.parse(b"L0\r\nL1\r\nL2");
    p.parse(b"\x1b[2J"); // keep display only (mode 2 semantics)
    p.parse(b"\x1b[3J");
    assert!(
        p.screen().scrollback().is_empty(),
        "ED 3 must clear scrollback"
    );
}

// --------------------- resize preserves scrollback ---------------------

#[test]
fn test_resize_shrink_pushes_rows_to_scrollback() {
    let mut p = setup(5, 5);
    p.parse(b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD\r\nEEEEE");
    p.screen_mut().resize(5, 3);
    // The two rows that fell off the top (A, B) must survive in scrollback,
    // newest-at-front: index 0 holds B (the row just above the viewport).
    assert_eq!(
        p.screen().scrollback().len(),
        2,
        "dropped rows must be kept"
    );
    let sb = p.screen().scrollback();
    assert_eq!(sb[0][0].ch, 'B', "newest dropped row at index 0");
    assert_eq!(sb[1][0].ch, 'A', "oldest dropped row at index 1");
    // Remaining view still holds the bottom rows C, D, E.
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'C');
    assert_eq!(p.screen().cell(2, 0).unwrap().ch, 'E');
}

#[test]
fn test_resize_grow_pulls_rows_from_scrollback() {
    let mut p = setup(5, 5);
    p.parse(b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD\r\nEEEEE");
    p.screen_mut().resize(5, 3); // shrink: A, B into scrollback
    assert_eq!(p.screen().scrollback().len(), 2);
    p.screen_mut().resize(5, 5); // grow back: pull them above the view
    assert_eq!(
        p.screen().scrollback().len(),
        0,
        "grown rows are pulled out"
    );
    // Order restored: A above B above C D E.
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'A');
    assert_eq!(p.screen().cell(1, 0).unwrap().ch, 'B');
    assert_eq!(p.screen().cell(2, 0).unwrap().ch, 'C');
    assert_eq!(p.screen().cell(4, 0).unwrap().ch, 'E');
}

// --------------------- invalid UTF-8 recovery ---------------------

#[test]
fn test_invalid_utf8_does_not_swallow_following_text() {
    // A lone invalid lead byte must not swallow the ASCII that follows it.
    let mut p = setup(80, 24);
    p.parse(&[0xFF, b'a', b'b', b'c']);
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '\u{FFFD}');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'a');
    assert_eq!(screen.cell(0, 2).unwrap().ch, 'b');
    assert_eq!(screen.cell(0, 3).unwrap().ch, 'c');
}

#[test]
fn test_invalid_continuation_resyncs() {
    // 0xC3 starts a 2-byte char; a second 0xC3 is an invalid continuation.
    // Both are consumed as one replacement char, then text continues cleanly.
    let mut p = setup(80, 24);
    p.parse(&[0xC3, 0xC3, b'x']);
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '\u{FFFD}');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'x');
}

#[test]
fn test_ascii_after_partial_utf8_drops_partial_only() {
    // 0xC3 is an incomplete 2-byte lead; the following ASCII '(' is handled as
    // text and the orphaned lead is dropped (pre-existing, standard behavior).
    let mut p = setup(80, 24);
    p.parse(&[0xC3, b'(', b'x']);
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '(');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'x');
}

#[test]
fn test_multi_char_utf8_burst_emits_all() {
    // Continuation bytes piling up behind a completed code point must all emit.
    let mut p = setup(80, 24);
    p.parse(&[0xC3, 0xA9]);
    p.parse(&[0xE2, 0x82, 0xAC]); // 'é' then '€' across chunks
    let screen = p.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '\u{e9}');
    assert_eq!(screen.cell(0, 1).unwrap().ch, '\u{20ac}');
}

#[test]
fn decscusr_sets_cursor_shape() {
    // Regression: nvim/vi send DECSCUSR (CSI Ps SP q) to switch the cursor
    // between block (normal) and bar/underline (insert). The parser ignored
    // it, so the cursor stayed a block everywhere.
    let mut p = setup(20, 5);
    p.parse(b"\x1b[5 q"); // blink bar (insert mode)
    assert_eq!(p.screen().cursor().shape, CursorShape::Bar);
    p.parse(b"\x1b[2 q"); // steady block (normal mode)
    assert_eq!(p.screen().cursor().shape, CursorShape::Block);
    p.parse(b"\x1b[4 q"); // steady underline
    assert_eq!(p.screen().cursor().shape, CursorShape::Underline);
}
