//! Unicode conformance suite (v1.0 gate: "100% Unicode test suite").
//! Exercises the parser + screen through their public APIs; locks in the
//! CURRENT behavior, including known simplifications noted inline.

use crate::parser::Parser;

/// Wide char (CJK) writes at the cursor and advances it by its display width
/// (2 columns); the next char lands after the wide cell.
#[test]
fn wide_char_advances_cursor_two_columns() {
    let mut parser = Parser::new(80, 24);
    parser.parse("你".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '你');
    assert_eq!(screen.cursor().col, 2);

    parser.parse("A".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 2).unwrap().ch, 'A');
    assert_eq!(screen.cursor().col, 3);
}

/// Two adjacent wide chars do not overlap: each lands at its own start column.
#[test]
fn wide_chars_stack_adjacent() {
    let mut parser = Parser::new(80, 24);
    parser.parse("你你".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '你');
    assert_eq!(screen.cell(0, 2).unwrap().ch, '你');
    assert_eq!(screen.cursor().col, 4);
}

/// Combining acute (U+0301) is NOT merged into the base cell: the screen model
/// is cell-per-scalar, so it is stored in its own cell, adjacent, without
/// overwriting the base char. (Real terminals render it as one grapheme; that
/// merge would happen at the glyph/shaping layer, not the cell buffer.)
#[test]
fn combining_char_is_stored_adjacent_not_merged() {
    let mut parser = Parser::new(80, 24);
    parser.parse("e\u{0301}".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'e');
    assert_eq!(screen.cell(0, 1).unwrap().ch, '\u{0301}');
    assert_eq!(screen.cursor().col, 2);
}

/// BEL (0x07) rings the bell; it writes no cell and does not move the cursor.
#[test]
fn bel_writes_no_cell() {
    let mut parser = Parser::new(80, 24);
    parser.parse("ab\x07".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cursor().col, 2);
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'a');
    assert_eq!(screen.cell(0, 2).unwrap().ch, ' ');
}

/// CR LF moves the cursor to column 0 of the next line.
#[test]
fn crlf_moves_to_next_line_start() {
    let mut parser = Parser::new(80, 24);
    parser.parse("abc\r\n".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cursor().row, 1);
    assert_eq!(screen.cursor().col, 0);
}

/// HT advances to the next tab stop (every 8 columns by default).
#[test]
fn tab_advances_to_next_tab_stop() {
    let mut parser = Parser::new(80, 24);
    parser.parse("a\t".as_bytes());
    assert_eq!(parser.screen().cursor().col, 8);

    let mut parser = Parser::new(80, 24);
    parser.parse("abcdefghi\t".as_bytes());
    assert_eq!(parser.screen().cursor().col, 16);
}

/// A multi-byte UTF-8 sequence split across two read() chunks is buffered and
/// emits a single character once complete. ('é' = C3 A9)
#[test]
fn utf8_split_across_reads_is_buffered() {
    let mut parser = Parser::new(80, 24);
    parser.parse(&[0xC3]);
    assert_eq!(parser.screen().cursor().col, 0);

    parser.parse(&[0xA9]);
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'é');
    assert_eq!(screen.cursor().col, 1);
}

/// A 4-byte sequence (emoji, F0 9F 98 80) split 2+2 across reads still renders
/// one character.
#[test]
fn utf8_four_byte_split_across_reads() {
    let mut parser = Parser::new(80, 24);
    parser.parse(&[0xF0, 0x9F]);
    parser.parse(&[0x98, 0x80]);
    assert_eq!(parser.screen().cell(0, 0).unwrap().ch, '\u{1F600}');
}

/// A bare ESC + unknown final byte is consumed as an escape sequence, not
/// printed, and the following prompt text is untouched.
#[test]
fn unknown_escape_is_ignored_and_prompt_intact() {
    let mut parser = Parser::new(80, 24);
    parser.parse("ab\x1bxcd".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'a');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'b');
    assert_eq!(screen.cell(0, 2).unwrap().ch, 'c');
    assert_eq!(screen.cell(0, 3).unwrap().ch, 'd');
    assert_eq!(screen.cursor().col, 4);
}

/// A bare `ESC [` with no final byte treats the next byte as its final byte
/// (real terminal behavior: `ESC [ y` is an unknown CSI sequence, consumed and
/// ignored, never printed). The rest of the line is unaffected.
#[test]
fn truncated_csi_consumes_next_byte_as_final() {
    let mut parser = Parser::new(80, 24);
    parser.parse("hi\x1b[".as_bytes());
    parser.parse("yo".as_bytes());
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, 'h');
    assert_eq!(screen.cell(0, 1).unwrap().ch, 'i');
    assert_eq!(screen.cell(0, 2).unwrap().ch, 'o');
    assert_eq!(screen.cursor().col, 3);
}
