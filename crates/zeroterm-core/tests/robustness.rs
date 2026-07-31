//! Robustness + Unicode conformance tests for the VT parser.
//! Feed hostile/weird byte sequences; assert no panic and screen invariants.

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

fn check_after_resize(bytes: &[u8]) {
    let mut p = Parser::new(80, 24);
    p.parse(bytes);
    for (cols, rows) in [(120, 40), (40, 10), (80, 24)] {
        p.screen_mut().resize(cols, rows);
        assert_screen_ok(&p);
    }
}

#[test]
fn utf8_valid_multibyte() {
    parse_and_check("héllo wörld".as_bytes());
    parse_and_check("こんにちは世界".as_bytes());
    parse_and_check("🎉🎊".as_bytes());
    parse_and_check("𝔘𝔫𝔦𝔠𝔬𝔡𝔢".as_bytes());
    parse_and_check("mixed € and 中 and 🎉 on one line".as_bytes());
}

#[test]
fn utf8_truncated_sequences() {
    // Half a char: E2 82 (€ cut), F0 9F 8E (🎉 cut), E4 B8 (中 cut)
    parse_and_check(&[0xE2, 0x82]);
    parse_and_check(&[0xF0, 0x9F, 0x8E]);
    parse_and_check(&[0xE4, 0xB8]);
    parse_and_check(&[0xF0, 0x9F, 0x8E, b'a', b'b', 0xE2, 0x82]);
    // Truncated after partial CSI
    parse_and_check(b"\x1b[3\xe2\x82m");
}

#[test]
fn utf8_invalid_bytes() {
    parse_and_check(&[0xFF]);
    parse_and_check(&[0xFE, 0xFF, 0x80, 0xBF]);
    parse_and_check(&[0x80; 32]);
    parse_and_check(&[0xFF; 64]);
}

#[test]
fn utf8_overlong_encoding() {
    // Overlong '/' (C0 AF), overlong 0x80 (C1 81), E0 80 AF
    parse_and_check(&[0xC0, 0xAF]);
    parse_and_check(&[0xC1, 0x81]);
    parse_and_check(&[0xE0, 0x80, 0xAF]);
    parse_and_check(&[0xC0, 0xAF, b'x', 0xC1, 0x81]);
}

#[test]
fn utf8_in_middle_of_csi_params() {
    parse_and_check(b"\x1b[3;\xffm");
    parse_and_check(b"\x1b[3;\xe4\xb8\xadm");
    parse_and_check(b"\x1b[\xf0\x9f\x8e\x893A");
    parse_and_check(b"\x1b[\x801;2H");
}

#[test]
fn every_byte_0x00_through_0xff() {
    let all: Vec<u8> = (0x00..=0xFF).collect();
    parse_and_check(&all);
    // Twice, so second pass hits parser mid-escape states
    parse_and_check(&[&all[..], &all[..]].concat());
}

#[test]
fn escape_openers_followed_by_garbage() {
    for &opener in b"\x1b\x1b[\x1b]\x1bP\x1b^\x1b_\x1bX" {
        let mut input = vec![opener];
        input.extend(0x00..=0x7F);
        parse_and_check(&input);
    }
}

#[test]
fn unterminated_sequences() {
    parse_and_check(b"\x1b[1;2");
    parse_and_check(b"\x1b[38;5;");
    parse_and_check(b"\x1b]0;unterminated title");
    parse_and_check(b"\x1bP0;1;q");
    parse_and_check(b"\x1b^unterminated apc");
    parse_and_check(b"\x1b_unterminated pm");
    parse_and_check(b"\x1bXunterminated sos");
}

#[test]
fn escape_alone_at_end_of_buffer() {
    // ESC as last byte; state must persist and later data must not corrupt
    parse_and_check(b"\x1b");
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b");
    p.parse(b"[5;5H");
    assert_screen_ok(&p);
}

#[test]
fn esc_then_ascii_range() {
    // Every final byte ESC can see: digits, letters, punctuation
    for b in 0x30..=0x7E {
        parse_and_check(&[0x1B, b]);
        parse_and_check(&[0x1B, b, b'x', b'\n']);
    }
}

#[test]
fn csi_with_missing_final_byte() {
    // CSI then params/intermediates but never a final byte 0x40..=0x7E
    parse_and_check(b"\x1b[1;2;3;4;5");
    parse_and_check(b"\x1b[?25");
    parse_and_check(b"\x1b[!p");
    parse_and_check(b"\x1b[ : : : ");
    parse_and_check(b"\x1b[0;");
    parse_and_check(b"\x1b[");
}

#[test]
fn osc_and_dcs_without_terminator() {
    let mut osc = b"\x1b]0;".to_vec();
    osc.extend(std::iter::repeat(b'a').take(4096));
    parse_and_check(&osc);
    let mut dcs = b"\x1bPq".to_vec();
    dcs.extend(std::iter::repeat(b'!').take(4096));
    parse_and_check(&dcs);
    let mut apc = b"\x1b_G".to_vec();
    apc.extend(std::iter::repeat(b'=').take(4096));
    parse_and_check(&apc);
}

#[test]
fn huge_param_values() {
    parse_and_check(b"\x1b[999999A");
    parse_and_check(b"\x1b[999999S");
    parse_and_check(b"\x1b[9999999999999999A"); // i64::MAX edge
    parse_and_check(b"\x1b[99999999999999999999A"); // overflows i64 -> ignored
    parse_and_check(b"\x1b[999999;999999H");
    parse_and_check(b"\x1b[999999m");
    parse_and_check(b"\x1b[9999999999999999;1r");
    parse_and_check(b"\x1b[1;999999;2;999999;3m");
}

#[test]
fn params_with_leading_zeros() {
    parse_and_check(b"\x1b[0000001;000002H");
    parse_and_check(b"\x1b[000000;000000r");
    parse_and_check(b"\x1b[00000m");
    parse_and_check(b"\x1b[0000000000000A");
}

#[test]
fn screen_integrity_after_garbage() {
    let mut corpus = Vec::new();
    for opener in [b'\x1b', b'\x1b', b'\x1b'] {
        corpus.push(opener);
    }
    corpus.extend(b"[?1049h");
    corpus.extend(&[0xFF, 0x80, 0xE2, 0x82]);
    corpus.extend(b"\x1b]52;c;YQ==");
    corpus.extend(b"\x1bPq###P$$$");
    corpus.extend(b"\x1b[999999;999999;999999;999999m");
    corpus.extend(b"\x1b[?1049l");
    corpus.push(0x1B);
    check_after_resize(&corpus);
}

#[test]
fn chunked_parse_state_persistence() {
    let stream: Vec<u8> = (0x00..=0xFF)
        .chain(
            b"\x1b[31;42mhello\x1b]2;t\x1bP0;1;q\x1b[999999A"
                .iter()
                .copied(),
        )
        .collect();
    let mut p = Parser::new(80, 24);
    for chunk in stream.chunks(7) {
        p.parse(chunk);
        assert_screen_ok(&p);
    }
}

#[test]
fn kitty_and_sixel_garbage() {
    // Valid kitty with absurd dimensions: no large allocation on decode
    parse_and_check(b"\x1b_Ga=T;s=999999;v=999999;AAAA\x1b\\");
    // Kitty with truncated escape terminator
    parse_and_check(b"\x1b_Ga=T;s=1;v=1;AAAA\x1b");
    // Bare sixel stair-step garbage
    parse_and_check(b"\x1bPq#;2;0;0;0;0~-{}#$@!?\x1b\\");
    parse_and_check(b"\x1bPq");
}
