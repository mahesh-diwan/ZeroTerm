//! Tests for ZeroTerm core

use crate::{
    cell::{Cell, Color},
    parser::Parser,
    screen::Screen,
};

#[test]
fn test_cell_default() {
    let cell = Cell::default();
    assert_eq!(cell.ch, ' ');
    assert_eq!(cell.fg, Color::DEFAULT_FG);
    assert_eq!(cell.bg, Color::DEFAULT_BG);
}

#[test]
fn test_cell_width() {
    assert_eq!(Cell::new('a').width(), 1);
    assert_eq!(Cell::new('\u{4e2d}').width(), 2);
    assert_eq!(Cell::new(' ').width(), 1);
}

#[test]
fn test_color_ansi_16() {
    assert_eq!(Color::from_ansi_16(0), Color::BLACK);
    assert_eq!(Color::from_ansi_16(9), Color::RED);
    assert_eq!(Color::from_ansi_16(10), Color::GREEN);
    assert_eq!(Color::from_ansi_16(15), Color::WHITE);
}

#[test]
fn test_color_ansi_256() {
    assert_eq!(Color::from_ansi_256(0), Color::BLACK);
    assert_eq!(Color::from_ansi_256(15), Color::WHITE);
    let c = Color::from_ansi_256(16);
    assert_eq!(c.r, 0);
    assert_eq!(c.g, 0);
    assert_eq!(c.b, 0);
    let c = Color::from_ansi_256(231);
    assert_eq!(c.r, 255);
    assert_eq!(c.g, 255);
    assert_eq!(c.b, 255);
}

#[test]
fn test_screen_basic() {
    let screen = Screen::new(80, 24);
    assert_eq!(screen.size().cols, 80);
    assert_eq!(screen.size().rows, 24);
    assert_eq!(screen.cursor().row, 0);
    assert_eq!(screen.cursor().col, 0);
}

#[test]
fn test_screen_put_char() {
    let mut screen = Screen::new(10, 5);
    screen.put_char('H');
    screen.put_char('i');
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'H');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'i');
    assert_eq!(screen.cursor().col, 2);
}

#[test]
fn test_screen_cursor_movement() {
    let mut screen = Screen::new(10, 5);
    screen.cursor_pos(3, 4);
    assert_eq!(screen.cursor().row, 2);
    assert_eq!(screen.cursor().col, 3);

    screen.cursor_up(1);
    assert_eq!(screen.cursor().row, 1);

    screen.cursor_down(2);
    assert_eq!(screen.cursor().row, 3);

    screen.cursor_right(2);
    assert_eq!(screen.cursor().col, 5);

    screen.cursor_left_n(1);
    assert_eq!(screen.cursor().col, 4);
}

#[test]
fn test_screen_linefeed() {
    let mut screen = Screen::new(10, 3);
    screen.put_char('a');
    screen.linefeed();
    assert_eq!(screen.cursor().row, 1);
    assert_eq!(screen.cursor().col, 1);

    screen.put_char('b');
    screen.linefeed();
    assert_eq!(screen.cursor().row, 2);

    screen.put_char('c');
    screen.linefeed();
    assert_eq!(screen.cursor().row, 2);
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'b');
    assert_eq!(screen.cell(1, 2).unwrap().ch, 'c');
}

#[test]
fn test_screen_scrollback() {
    let mut screen = Screen::new(10, 3);
    for _i in 0..5 {
        for c in 'a'..='z' {
            screen.put_char(c);
        }
        screen.linefeed();
    }
    assert!(!screen.scrollback().is_empty());
}

#[test]
fn test_screen_erase() {
    let mut screen = Screen::new(10, 3);
    for c in 'a'..='j' {
        screen.put_char(c);
    }
    screen.cursor_pos(1, 6);
    screen.erase_line(0);
    assert_eq!(screen.cell(0, 5).unwrap().ch, ' ');
    assert_eq!(screen.cell(0, 9).unwrap().ch, ' ');
    assert_eq!(screen.cell(0, 4).unwrap().ch, 'e');
}

#[test]
fn test_screen_attributes() {
    let mut screen = Screen::new(10, 3);
    screen.set_bold(true);
    screen.set_fg_rgb(255, 0, 0);
    screen.set_bg_rgb(0, 0, 255);
    screen.put_char('X');

    let cell = screen.cell(0, 0).unwrap();
    assert!(cell.attrs.bold);
    assert_eq!(cell.fg, Color { r: 255, g: 0, b: 0 });
    assert_eq!(cell.bg, Color { r: 0, g: 0, b: 255 });
}

#[test]
fn test_parser_basic() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"Hello");
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'H');
    assert_eq!(screen.cell(0, 4).unwrap().ch, 'o');
}

#[test]
fn test_parser_esc_sequence() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b[2J");
    let screen = parser.screen();
    for row in 0..24 {
        for col in 0..80 {
            let cell = screen.cell(row, col).unwrap();
            assert_eq!(cell.ch, ' ');
        }
    }
}

#[test]
fn test_parser_cursor_movement() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b[10;20H");
    let screen = parser.screen();
    assert_eq!(screen.cursor().row, 9);
    assert_eq!(screen.cursor().col, 19);
}

#[test]
fn test_parser_colors() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b[91m");
    parser.parse(b"X");
    let cell = parser.screen().cell(0, 0).unwrap();
    assert_eq!(cell.fg, Color::RED);
}

#[test]
fn test_parser_sgr_reset() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b[1;31;42m");
    parser.parse(b"\x1b[0m");
    parser.parse(b"X");
    let cell = parser.screen().cell(0, 0).unwrap();
    assert!(!cell.attrs.bold);
    assert_eq!(cell.fg, Color::DEFAULT_FG);
    assert_eq!(cell.bg, Color::DEFAULT_BG);
}

#[test]
fn test_osc8_hyperlink_stamps_cells_until_closed() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b]8;;https://example.com\x1b\\");
    parser.parse(b"click");
    parser.parse(b"\x1b]8;;\x1b\\"); // close
    parser.parse(b"plain");
    let linked = parser.screen().cell(0, 0).unwrap();
    assert_ne!(linked.link_id, 0);
    assert_eq!(
        parser.screen().link_uri(linked.link_id),
        Some("https://example.com")
    );
    // After the closing OSC 8 the same id must not leak to new cells.
    assert_eq!(parser.screen().cell(0, 5).unwrap().link_id, 0);
}

#[test]
fn test_osc8_hyperlink_id_roundtrip_via_uri_lookup() {
    let mut screen = Screen::new(40, 10);
    screen.set_hyperlink("https://a.test/x");
    screen.put_char('a');
    screen.set_hyperlink("https://a.test/x"); // dedupe: same id
    screen.put_char('b');
    let id_a = screen.cell(0, 0).unwrap().link_id;
    let id_b = screen.cell(0, 1).unwrap().link_id;
    assert_eq!(id_a, id_b);
    assert_eq!(screen.link_uri(id_a), Some("https://a.test/x"));
}

#[test]
fn test_osc9_notification_is_drained() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b]9;build finished\x07");
    assert_eq!(
        parser.take_notification().as_deref(),
        Some("build finished")
    );
    assert_eq!(parser.take_notification(), None);
    // Windows Terminal urgency form: 9;0;text
    parser.parse(b"\x1b]9;0;errors\x07");
    assert_eq!(parser.take_notification().as_deref(), Some("errors"));
}

#[test]
fn test_kitty_query_reply_when_supported() {
    let mut parser = Parser::new(80, 24);
    parser.set_kitty_supported(true);
    parser.parse(b"\x1b[?u");
    assert_eq!(
        parser.take_response().as_deref(),
        Some(b"\x1b[?1u".as_slice())
    );
}

#[test]
fn test_kitty_push_sets_disambiguate() {
    let mut parser = Parser::new(80, 24);
    parser.set_kitty_supported(true);
    assert!(!parser.kitty_disambiguate());
    parser.parse(b"\x1b[>1u"); // push disambiguate
    assert!(parser.kitty_disambiguate());
    parser.parse(b"\x1b[<u"); // pop -> back to 0
    assert!(!parser.kitty_disambiguate());
}

#[test]
fn test_kitty_ignored_when_unsupported() {
    let mut parser = Parser::new(80, 24);
    parser.parse(b"\x1b[?u");
    assert_eq!(parser.take_response(), None);
    parser.parse(b"\x1b[>1u");
    assert!(!parser.kitty_disambiguate());
}
