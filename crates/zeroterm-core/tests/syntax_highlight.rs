//! Scrollback-wide syntax highlighting: rows carry `Cell::syntax_color` tags
//! written when the text lands, so colors survive scrolling into scrollback.

use zeroterm_core::highlight::{HL_COMMENT, HL_KEYWORD, HL_NUMBER, HL_STRING};
use zeroterm_core::parser::Parser;
use zeroterm_core::screen::Screen;

#[test]
fn scrollback_line_keeps_syntax_tags() {
    let mut p = Parser::new(80, 24);
    p.parse(b"if echo 42 # note\n");
    // Enough newlines to push the first line out of the 24-row buffer.
    for _ in 0..40 {
        p.parse(b"\n");
    }

    let s = p.screen();
    assert!(!s.scrollback().is_empty(), "line scrolled into scrollback");
    let line = s
        .scrollback()
        .iter()
        .find(|row| row[0].ch == 'i' && row[1].ch == 'f')
        .expect("tagged line present in scrollback");

    // "if echo 42 # note" → keyword(if), keyword(echo), number(42), comment.
    assert_eq!(line[0].syntax_color, HL_KEYWORD);
    assert_eq!(line[1].syntax_color, HL_KEYWORD);
    assert_eq!(line[2].syntax_color, 0, "space stays default");
    assert_eq!(line[3].syntax_color, HL_KEYWORD);
    assert_eq!(line[8].syntax_color, HL_NUMBER);
    assert_eq!(line[9].syntax_color, HL_NUMBER);
    assert_eq!(line[11].syntax_color, HL_COMMENT);
    assert_eq!(line[16].syntax_color, HL_COMMENT);
}

#[test]
fn visible_row_tagged_as_typed() {
    let mut s = Screen::new(80, 24);
    for ch in "echo \"hi\" 42".chars() {
        s.put_char(ch);
    }
    let row = &s.buffer()[0];
    assert_eq!(row[0].syntax_color, HL_KEYWORD); // echo
    assert_eq!(row[5].syntax_color, HL_STRING); // opening quote
    assert_eq!(row[8].syntax_color, HL_STRING); // closing quote
    assert_eq!(row[10].syntax_color, HL_NUMBER); // 4
    assert_eq!(row[11].syntax_color, HL_NUMBER); // 2
}

#[test]
fn overwrite_with_plain_text_resets_tags() {
    let mut s = Screen::new(80, 24);
    for ch in "if".chars() {
        s.put_char(ch);
    }
    assert_eq!(s.buffer()[0][0].syntax_color, HL_KEYWORD);
    assert_eq!(s.buffer()[0][1].syntax_color, HL_KEYWORD);

    s.carriage_return();
    for ch in "ab".chars() {
        s.put_char(ch);
    }
    assert_eq!(s.buffer()[0][0].ch, 'a', "overwrite actually landed");
    assert_eq!(
        s.buffer()[0][0].syntax_color,
        0,
        "plain text clears the tag"
    );
    assert_eq!(s.buffer()[0][1].syntax_color, 0);
}
