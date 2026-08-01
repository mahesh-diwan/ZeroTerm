//! Unicode conformance suite for the VT parser.
//!
//! Covers: Latin-1, combining marks, wide (CJK/emoji), ZWJ sequences,
//! variation selectors, UTF-8 split across parse() calls, non-ASCII in
//! OSC/DCS, bidi text, malformed/surrogate input, and line-wrap edge cases.
//!
//! Conformance model (current implementation): the screen is a
//! cell-per-scalar buffer. `put_grapheme` advances the cursor by the glyph's
//! display width (unicode-width, min 1) — wide CJK/emoji advance 2, combining
//! marks/ZWJ fall back to 1. There is NO grapheme clustering. `Cell::width()`
//! (cell.rs:206) reports the unicode-width of the stored scalar, so wide
//! glyphs are *stored* correctly and renderer-side width info is available,
//! but the cell-per-scalar layout diverges from a grapheme-aware terminal.
//! The wide/combining/ZWJ tests assert this storage model explicitly so a
//! future parser change (per-grapheme advance) is caught as a test update
//! rather than a silent regression.

use zeroterm_core::parser::Parser;

fn assert_screen_ok(p: &Parser) {
    let s = p.screen();
    let size = s.size();
    let buf = s.buffer();
    assert_eq!(buf.len(), size.rows, "row count == rows");
    for (r, row) in buf.iter().enumerate() {
        assert_eq!(row.len(), size.cols, "row {r} length == cols");
    }
    let c = s.cursor();
    assert!(c.row < size.rows, "cursor row in bounds");
    // col may equal cols: pending-autowrap position after writing last column
    assert!(c.col <= size.cols, "cursor col in bounds");
}

fn parse_and_check(bytes: &[u8]) {
    let mut p = Parser::new(80, 24);
    p.parse(bytes);
    assert_screen_ok(&p);
}

// ------------------------------- Latin-1 --------------------------------

#[test]
fn latin1_accents_render_at_columns() {
    let mut p = Parser::new(80, 24);
    p.parse("héllo wörld".as_bytes());
    let buf = p.screen().buffer();
    assert_eq!(buf[0][0].ch, 'h');
    assert_eq!(buf[0][1].ch, 'é');
    assert_eq!(buf[0][2].ch, 'l');
    assert_eq!(buf[0][6].ch, 'w');
    assert_eq!(buf[0][7].ch, 'ö');
    assert_eq!(buf[0][8].ch, 'r');
    assert_eq!(p.screen().cursor().col, "héllo wörld".chars().count());
    assert_screen_ok(&p);
}

// ----------------------------- Combining marks ---------------------------

#[test]
fn combining_mark_stored_after_base() {
    // "e" + U+0301 COMBINING ACUTE ACCENT
    let mut p = Parser::new(80, 24);
    p.parse("e\u{301}".as_bytes());
    let buf = p.screen().buffer();
    assert_eq!(buf[0][0].ch, 'e', "base char in first cell");
    assert_eq!(buf[0][1].ch, '\u{301}', "combining mark in following cell");
    assert_eq!(buf[0][0].width(), 1);
    assert_eq!(
        buf[0][1].width(),
        0,
        "combining mark is width-0 per unicode-width"
    );
    // Current model: 1 col/scalar. A grapheme-aware terminal would keep the
    // mark in the base cell and advance only 1 col total.
    assert_eq!(p.screen().cursor().col, 2);
}

#[test]
fn combining_mark_at_end_of_line_wraps() {
    // 80-col screen; base char lands in last column (col 79) -> pending
    // autowrap. The following combining mark triggers the wrap.
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b[1;80H");
    p.parse("e\u{301}".as_bytes());
    assert_eq!(p.screen().cell(0, 79).unwrap().ch, 'e');
    assert_eq!(
        p.screen().cell(1, 0).unwrap().ch,
        '\u{301}',
        "mark wraps to next line"
    );
    assert_screen_ok(&p);
}

// ------------------------------ Wide chars -------------------------------

#[test]
fn wide_cjk_and_emoji_stored_in_cells() {
    let mut p = Parser::new(80, 24);
    p.parse("a你b🎉c".as_bytes());
    let buf = p.screen().buffer();
    assert_eq!(buf[0][0].ch, 'a');
    assert_eq!(buf[0][1].ch, '你');
    assert_eq!(buf[0][1].width(), 2, "CJK reported as 2 cols");
    assert_eq!(buf[0][3].ch, 'b');
    assert_eq!(buf[0][4].ch, '🎉');
    assert_eq!(buf[0][4].width(), 2, "emoji reported as 2 cols");
    assert_eq!(buf[0][6].ch, 'c');
    // Width-aware advance: 你/🎉 move the cursor 2 columns each.
    assert_eq!(p.screen().cursor().col, 7);
    assert_screen_ok(&p);
}

#[test]
fn wide_chars_do_not_overwrite_each_other() {
    // 你 (width 2) advances the cursor by 2, so 好 lands at col 2 and the
    // first glyph's second half is not clobbered.
    let mut p = Parser::new(80, 24);
    p.parse("你好".as_bytes());
    assert_eq!(
        p.screen().cell(0, 0).unwrap().ch,
        '你',
        "no overwrite at col 0"
    );
    assert_eq!(
        p.screen().cell(0, 2).unwrap().ch,
        '好',
        "second wide char lands at col 2"
    );
    assert_eq!(p.screen().cursor().col, 4);
    assert_screen_ok(&p);
}

// ----------------------------- ZWJ sequences -----------------------------

#[test]
fn zwj_family_emoji_no_panic() {
    // 👨 U+1F468 ZWJ U+200D 👩 U+1F469 ZWJ U+200D 👧 U+1F467
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut p = Parser::new(80, 24);
    p.parse(fam.as_bytes());
    let buf = p.screen().buffer();
    // Wide emoji advance 2; ZWJ (width 0) falls back to 1.
    assert_eq!(buf[0][0].ch, '\u{1F468}');
    assert_eq!(buf[0][2].ch, '\u{200D}');
    assert_eq!(buf[0][3].ch, '\u{1F469}');
    assert_eq!(buf[0][5].ch, '\u{200D}');
    assert_eq!(buf[0][6].ch, '\u{1F467}');
    // Each scalar is its own cell in the current model. A grapheme-aware
    // terminal renders the whole cluster in one glyph width of 2.
    assert_eq!(p.screen().cursor().col, 8);
    assert_screen_ok(&p);
}

#[test]
fn zero_width_joiner_alone() {
    let mut p = Parser::new(80, 24);
    p.parse("\u{200D}".as_bytes());
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, '\u{200D}');
    assert_eq!(p.screen().cursor().col, 1);
    assert_screen_ok(&p);
}

// -------------------------- Variation selectors --------------------------

#[test]
fn emoji_variation_selector_no_panic() {
    // ❤ + U+FE0F VARIATION SELECTOR-16
    let mut p = Parser::new(80, 24);
    p.parse("a\u{2764}\u{FE0F}b".as_bytes());
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, '\u{2764}');
    assert_eq!(p.screen().cell(0, 2).unwrap().ch, '\u{FE0F}');
    assert_eq!(p.screen().cell(0, 3).unwrap().ch, 'b');
    assert_screen_ok(&p);
}

// --------------------- UTF-8 split across parse calls --------------------

#[test]
fn utf8_split_byte_by_byte() {
    let mut p = Parser::new(80, 24);
    for &b in "é".as_bytes() {
        p.parse(&[b]);
    }
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'é');

    let mut p = Parser::new(80, 24);
    for &b in "a🎉".as_bytes() {
        p.parse(&[b]);
    }
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(
        p.screen().cell(0, 1).unwrap().ch,
        '🎉',
        "4-byte char reassembled"
    );
}

#[test]
fn utf8_split_arbitrary_chunk_boundaries() {
    let mut p = Parser::new(80, 24);
    let data = "héllo 中world 🎉!".as_bytes();
    for chunk in data.chunks(1 + (data.len() % 5)) {
        p.parse(chunk);
        assert_screen_ok(&p);
    }
    // Every scalar landed, regardless of where chunks cut the encodings.
    let mut col = 0;
    for ch in "héllo 中world 🎉!".chars() {
        assert_eq!(p.screen().cell(0, col).unwrap().ch, ch);
        // Advance by the cell's display width, mirroring put_grapheme.
        col += p.screen().cell(0, col).unwrap().width().max(1);
    }
    assert_eq!(p.screen().cursor().col, col);
}

// ------------------------ Non-ASCII in OSC / DCS -------------------------

#[test]
fn non_ascii_osc_title() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b]0;na\xc3\xafve \xe4\xb8\xad\xe6\x96\x87\x07");
    assert_eq!(p.screen().title(), "naïve 中文");
}

#[test]
fn non_ascii_dcs_passthrough() {
    // UTF-8 bytes ride through DCS passthrough untouched; state must recover.
    // Note: DCS passthrough terminates on ESC alone, so the ST backslash of
    // `\x1b\\` leaks into the screen as a literal '\\' — pinned as current
    // behavior.
    let mut p = Parser::new(80, 24);
    p.parse("before".as_bytes());
    p.parse(b"\x1bP0;1;q\xc3\xa9\xe4\xb8\xad");
    assert_eq!(
        p.screen().cursor().col,
        6,
        "DCS bytes do not hit the screen"
    );
    p.parse(b"\x1b\\after");
    let buf = p.screen().buffer();
    assert_eq!(buf[0][6].ch, '\\', "ST backslash leaks as one char");
    assert_eq!(buf[0][7].ch, 'a');
    assert_eq!(buf[0][11].ch, 'r');
    assert_eq!(p.screen().cursor().col, 12);
    assert_screen_ok(&p);
}

// --------------------------------- Bidi ----------------------------------

#[test]
fn bidi_arabic_no_reorder() {
    // Terminal layout is left-to-right; no bidi reordering applied.
    let word = "مرحبا"; // 5 letters: م ر ح ب ا
    assert_eq!(word.chars().count(), 5);
    let mut p = Parser::new(80, 24);
    p.parse(word.as_bytes());
    let buf = p.screen().buffer();
    let cells: String = buf[0][..5].iter().map(|c| c.ch).collect();
    assert_eq!(cells, word, "stored LTR in cell order");
    assert_eq!(p.screen().cursor().col, 5);
    assert_screen_ok(&p);
}

// ----------------------- Malformed / surrogate bytes ---------------------

#[test]
fn truncated_lead_byte_recovers() {
    // 0xC3 alone (incomplete é) then a valid ASCII byte: state recovers.
    let mut p = Parser::new(80, 24);
    p.parse(&[0xC3]);
    p.parse(b"x");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'x');
    assert_eq!(p.screen().cursor().col, 1);
}

#[test]
fn lone_surrogate_encoding_no_crash() {
    // 0xED 0xA0 0x80 is a UTF-8-encoded lone surrogate (U+D800) — invalid.
    parse_and_check(&[0xED, 0xA0, 0x80]);
    // Embedded between valid text: subsequent text must still render.
    let mut p = Parser::new(80, 24);
    p.parse(b"ab");
    p.parse(&[0xED, 0xA0, 0x80, 0xED, 0xB0, 0x80]);
    p.parse(b"cd");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, 'b');
    assert_eq!(p.screen().cell(0, 2).unwrap().ch, 'c');
    assert_eq!(p.screen().cell(0, 3).unwrap().ch, 'd');
    assert_screen_ok(&p);
}

#[test]
fn overlong_and_overmax_invalid_utf8() {
    // Overlong '/' (C0 AF) and >4-byte lead abuse — no panic, state recovers.
    parse_and_check(&[0xC0, 0xAF, 0xE2, 0x82, 0xF0, 0x9F, 0x8E, 0x89]);
    let mut p = Parser::new(80, 24);
    p.parse(&[0xC1, 0x81, b'q']);
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'q');
}

// --------------------------- Control handling ----------------------------

#[test]
fn tab_stops_at_eight() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\t");
    assert_eq!(p.screen().cursor().col, 8, "default tab stop at col 8");
    p.parse(b"\x1b[?1l"); // clear? no-op CSI; col unchanged
    assert_eq!(p.screen().cursor().col, 8);
}

#[test]
fn crlf_returns_to_line_start() {
    let mut p = Parser::new(80, 24);
    p.parse("ab\r\ncd".as_bytes());
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, 'b');
    assert_eq!(p.screen().cell(1, 0).unwrap().ch, 'c');
    assert_eq!(p.screen().cell(1, 1).unwrap().ch, 'd');
    assert_eq!(p.screen().cursor().col, 2);
    assert_eq!(p.screen().cursor().row, 1);
}

#[test]
fn backspace_at_line_start_stays() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x08");
    assert_eq!(p.screen().cursor().col, 0, "BS at col 0 clamps");
    p.parse(b"ab\x08");
    assert_eq!(p.screen().cursor().col, 1, "BS moves back one col");
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, 'b', "BS does not erase");
}

#[test]
fn del_is_noop() {
    let mut p = Parser::new(80, 24);
    p.parse(b"ab\x7fc");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'a');
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, 'b');
    assert_eq!(
        p.screen().cell(0, 2).unwrap().ch,
        'c',
        "DEL ignored, c lands after b"
    );
    assert_eq!(p.screen().cursor().col, 3);
}

#[test]
fn newline_advances_row_only() {
    // LF keeps the column (line feed only); here col is 1 after 'x'.
    let mut p = Parser::new(80, 24);
    p.parse("x\n\ny".as_bytes());
    assert_eq!(p.screen().cursor().row, 2);
    assert_eq!(p.screen().cursor().col, 2);
    assert_eq!(
        p.screen().cell(2, 1).unwrap().ch,
        'y',
        "y keeps col 1 after LF"
    );
}

// ----------------------------- Line wrapping -----------------------------

#[test]
fn long_line_wraps_without_panic() {
    let mut p = Parser::new(80, 24);
    let line = "a".repeat(10_000);
    p.parse(line.as_bytes());
    let c = p.screen().cursor();
    assert!(c.row < p.screen().size().rows, "cursor row in bounds");
    // Last char written wraps/scrolls but never corrupts the buffer.
    assert_screen_ok(&p);
}

#[test]
fn line_of_wide_chars_wraps() {
    // CJK on an 80-col screen — no width-aware advance, so all fit as cells.
    let mut p = Parser::new(80, 24);
    let line = "你".repeat(1000);
    p.parse(line.as_bytes());
    assert_screen_ok(&p);
}

#[test]
fn wide_char_at_last_column_then_text() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b[1;80H");
    p.parse("你x".as_bytes());
    // 你 lands in col 79 (no width check), cursor -> col 80, 'x' wraps.
    assert_eq!(p.screen().cell(0, 79).unwrap().ch, '你');
    assert_eq!(p.screen().cell(1, 0).unwrap().ch, 'x');
    assert_screen_ok(&p);
}
