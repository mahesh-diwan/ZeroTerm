//! Robustness + Unicode conformance tests for the VT parser.
//! Feed hostile/weird byte sequences; assert no panic and screen invariants.

use zeroterm_core::parser::Parser;
use zeroterm_core::screen::Screen;

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

// Deterministic xorshift PRNG: no rand dep, same stream every run.
struct Xs(u64);
impl Xs {
    fn next(&mut self) -> u8 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 33) as u8
    }
}

#[test]
fn fuzz_random_bytes_no_panic() {
    let mut rng = Xs(0x9E3779B97F4A7C15);
    for round in 0..500 {
        let len = 1 + (rng.next() as usize % 128);
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            *b = rng.next();
        }
        let mut p = Parser::new(80, 24);
        for chunk in buf.chunks(1 + rng.next() as usize % 13) {
            p.parse(chunk);
            assert_screen_ok(&p);
        }
        if round % 10 == 0 {
            p.screen_mut()
                .resize(30 + rng.next() as usize % 100, 5 + rng.next() as usize % 40);
            assert_screen_ok(&p);
        }
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
fn osc_title_with_utf8_sets_title() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b]0;caf\xc3\xa9\x07");
    assert_eq!(p.screen().title(), "café");
}

#[test]
fn utf8_renders_non_ascii_chars() {
    let mut p = Parser::new(80, 24);
    p.parse("héllo".as_bytes());
    let buf = p.screen().buffer();
    assert_eq!(buf[0][0].ch, 'h');
    assert_eq!(buf[0][1].ch, 'é');
    assert_eq!(buf[0][2].ch, 'l');
    // 3-byte (€) and 4-byte (🎉) sequences
    let mut p = Parser::new(80, 24);
    p.parse("a€".as_bytes());
    assert_eq!(p.screen().buffer()[0][1].ch, '€');
    let mut p = Parser::new(80, 24);
    p.parse("a🎉".as_bytes());
    assert_eq!(p.screen().buffer()[0][1].ch, '🎉');
    // Split across parse() calls (streaming PTY reads)
    let mut p = Parser::new(80, 24);
    p.parse(&[b'a', 0xC3]);
    p.parse(&[0xA9]);
    assert_eq!(p.screen().buffer()[0][1].ch, 'é');
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
    osc.extend(std::iter::repeat_n(b'a', 4096));
    parse_and_check(&osc);
    let mut dcs = b"\x1bPq".to_vec();
    dcs.extend(std::iter::repeat_n(b'!', 4096));
    parse_and_check(&dcs);
    let mut apc = b"\x1b_G".to_vec();
    apc.extend(std::iter::repeat_n(b'=', 4096));
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
    for opener in *b"\x1b\x1b\x1b" {
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

// --------------------- iTerm2 inline image (OSC 1337) ---------------------

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Structurally-valid 2x3 8-bit RGBA PNG. CRCs are zeros — the parser only
/// reads the IHDR dims, so checksums don't matter here.
fn tiny_png() -> Vec<u8> {
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0, 0, 0, 13]); // IHDR chunk length
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&[0, 0, 0, 2]); // width
    png.extend_from_slice(&[0, 0, 0, 3]); // height
    png.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA
    png.extend_from_slice(&[0, 0, 0, 0]); // CRC
    png.extend_from_slice(&[0, 0, 0, 1]); // IDAT chunk length
    png.extend_from_slice(b"IDAT");
    png.push(0); // payload (irrelevant to dims parsing)
    png.extend_from_slice(&[0, 0, 0, 0]); // CRC
    png.extend_from_slice(&[0, 0, 0, 0]); // IEND chunk length
    png.extend_from_slice(b"IEND");
    png.extend_from_slice(&[0, 0, 0, 0]); // CRC
    png
}

#[test]
fn sixel_palette_and_pixels_render_color() {
    // Set palette reg 0 to red (100%), then draw a 1x1 red pixel.
    let seq = "\x1bPqP0;100;0;0#0~\x1b\\";
    let mut p = Parser::new(80, 24);
    p.parse(seq.as_bytes());
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1, "one sixel image");
    assert_eq!(imgs[0].width, 1);
    assert_eq!(imgs[0].height, 6, "'~' sets all 6 bits");
    // RGBA data: [r, g, b, a] — all 6 rows red
    let red = [255, 0, 0, 255];
    assert_eq!(imgs[0].data, red.repeat(6), "red from palette");
}

#[test]
fn iterm1337_inline_png_places_image() {
    let png = tiny_png();
    let seq = format!(
        "\x1b]1337;File=name=aW1nLnBuZw==;size={};inline=1:{}\x07",
        png.len(),
        b64(&png)
    );
    let mut p = Parser::new(80, 24);
    p.parse(seq.as_bytes());
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1, "one pending image");
    assert_eq!(imgs[0].data, png, "payload round-trips");
    assert_eq!(imgs[0].width, 2, "dims auto-parsed from PNG IHDR");
    assert_eq!(imgs[0].height, 3);
    assert!(
        p.screen().image_cells().contains_key(&(0, 0)),
        "image at cursor"
    );
    assert!(p.screen().image_registry().contains_key(&imgs[0].id));
}

#[test]
fn iterm1337_header_dims_override_png() {
    let png = tiny_png();
    let seq = format!(
        "\x1b]1337;File=name=AA;size=1;width=7;height=9;inline=1:{}\x07",
        b64(&png)
    );
    let mut p = Parser::new(80, 24);
    p.parse(seq.as_bytes());
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].width, 7, "header width wins");
    assert_eq!(imgs[0].height, 9, "header height wins");
}

#[test]
fn iterm1337_garbage_base64_ignored() {
    let mut p = Parser::new(80, 24);
    // '!' is not base64 -> decodes to empty -> skipped
    p.parse(b"\x1b]1337;File=name=AA;size=99;inline=1:!!!!\x07");
    assert!(p.take_pending_images().is_empty());
    assert!(p.screen().image_registry().is_empty());
}

#[test]
fn iterm1337_malformed_ignored() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b]1337;File=name=AA;size=99\x07"); // no colon separator
    p.parse(b"\x1b]1337;File=name=AA;size=99:\x07"); // empty payload
    p.parse(b"\x1b]1337;notafile\x07"); // missing File= prefix
    p.parse(b"\x1b]1337;File=name=AA;size=1;inline=0:QUFB\x07"); // download-only
    assert!(p.take_pending_images().is_empty());
    assert!(p.screen().image_registry().is_empty());
}

#[test]
fn csi_params_capped_at_32() {
    let mut input = b"\x1b[".to_vec();
    for i in 1..=150 {
        input.extend(format!("{};", i).as_bytes());
    }
    input.push(b'm');
    let mut p = Parser::new(80, 24);
    p.parse(&input);
    assert_screen_ok(&p);
    p.screen_mut().resize(40, 10);
    assert_screen_ok(&p);
}

#[test]
fn osc_buffer_capped() {
    let mut input = b"\x1b]0;".to_vec();
    input.extend(std::iter::repeat_n(b'a', 2 * 1024 * 1024));
    input.push(0x07);
    let mut p = Parser::new(80, 24);
    p.parse(&input);
    assert_screen_ok(&p);
    assert_eq!(p.screen().title(), "a".repeat((1 << 20) - 2));
}

#[test]
fn sixel_dimensions_capped() {
    let mut input = b"\x1bPq#0".to_vec();
    input.extend(std::iter::repeat_n(b'~', 20_000));
    input.extend(b"\x1b\\");
    let mut p = Parser::new(80, 24);
    p.parse(&input);
    assert_screen_ok(&p);
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1, "one truncated sixel image");
    assert_eq!(imgs[0].width, 8192, "width clamped to MAX_SIXEL_W");
    assert_eq!(imgs[0].height, 6);
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1bPq#0!99999999~$!99999999~\x1b\\");
    assert_screen_ok(&p);
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].width, 8192, "repeat count clamped");
    assert_eq!(imgs[0].height, 12, "two bands");
}

#[test]
fn image_registry_capped_at_64() {
    let mut s = Screen::new(80, 24);
    for _ in 0..80 {
        s.place_image(vec![0u8; 4], 1, 1);
    }
    assert!(s.image_registry().len() <= 64, "registry capped");
    assert!(s.image_cells().len() <= 64, "cell map capped");
    let id = s.place_image(vec![0u8; 4], 1, 1);
    assert!(s.image_registry().contains_key(&id), "newest kept");
}

#[test]
fn dec_2026_sets_sync_flag() {
    let mut p = Parser::new(80, 24);
    assert!(!p.sync_output(), "defaults off");
    p.parse(b"\x1b[?2026h");
    assert!(p.sync_output(), "DECSET 2026 enables");
    p.parse(b"\x1b[?2026l");
    assert!(!p.sync_output(), "DECRST 2026 disables");
}

// --------------------- escape sequence correctness ---------------------

#[test]
fn esc_intermediate_bytes_are_consumed_not_printed() {
    // ESC ( 0 = designate DEC special graphics; the '0' final byte must NOT print.
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b(0X");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().ch,
        'X',
        "charset final byte after ESC ( must be consumed"
    );
    assert_eq!(p.screen().cursor().col, 1);

    // ESC # 8 = DECALN; the '8' must not print.
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b#8Y");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().ch,
        'Y',
        "ESC # 8 final byte must be consumed"
    );

    // ESC ) 0, ESC * 0, ESC + 0 charset designations
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b)0Z");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'Z');
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b*0W");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'W');

    // ESC $ B and ESC % G charset designations
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b$BV");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'V');
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b%GU");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'U');

    // Multiple intermediates then final (ESC ( B)
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b(BT");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'T');
}

#[test]
fn st_terminator_does_not_print_backslash() {
    // OSC 0;title terminated by ST (ESC \) — no stray backslash may print.
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b]0;t\x1b\\hi");
    assert_eq!(p.screen().title(), "t");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().ch,
        'h',
        "no stray backslash after OSC ST"
    );
    assert_eq!(p.screen().cell(0, 1).unwrap().ch, 'i');

    // DCS terminated by ST
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1bPq\x1b\\hi");
    assert_eq!(
        p.screen().cell(0, 0).unwrap().ch,
        'h',
        "no stray backslash after DCS ST"
    );

    // APC / PM / SOS terminated by ST
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b_G\x1b\\hi");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'h');
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b^pm\x1b\\hi");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'h');
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1bXsos\x1b\\hi");
    assert_eq!(p.screen().cell(0, 0).unwrap().ch, 'h');
}

#[test]
fn scroll_region_scroll_keeps_scrollback_empty() {
    // DECSTBM 2;4 then scroll inside the region: scrollback must stay empty
    // (only full-screen scrolls write to scrollback).
    let mut p = Parser::new(10, 6);
    p.parse(b"abcdefghij");
    p.parse(b"\x1b[2;4r"); // region rows 1..=3 (0-indexed)
    p.parse(b"\x1b[4;1H"); // cursor to region bottom
    p.parse(b"\x1bD"); // IND -> scrolls the region
    assert_eq!(
        p.screen().scrollback().len(),
        0,
        "partial-region scroll must not write scrollback"
    );
    // Cursor stays at region bottom.
    assert_eq!(p.screen().cursor().row, 3);

    // Same for scroll_up/down escape sequences (SU/SD).
    let mut p = Parser::new(10, 6);
    p.parse(b"\x1b[2;4r\x1b[4;1H\x1b[5S"); // SU 5 inside region
    assert_eq!(
        p.screen().scrollback().len(),
        0,
        "SU in region must not write scrollback"
    );

    // Reverse scroll at region top must not pull lines back out of scrollback.
    let mut p = Parser::new(10, 4);
    p.parse(b"line0\r\nline1\r\nline2\r\nline3"); // 4 rows, then LF pushes to scrollback
    p.parse(b"\r\n"); // row 3 -> scroll, scrollback=1
    p.parse(b"\x1b[2;3r"); // region rows 1..=2
    p.parse(b"\x1b[2;1H"); // cursor to region top
    p.parse(b"\x1bM"); // RI at region top -> scroll_down region
    assert_eq!(
        p.screen().scrollback().len(),
        1,
        "RI in region must not consume scrollback"
    );
}

#[test]
fn huge_insert_delete_counts_are_bounded() {
    let mut p = Parser::new(80, 24);
    p.parse(b"\x1b[999999999999L"); // IL huge
    assert_screen_ok(&p);
    p.parse(b"\x1b[999999999999M"); // DL huge
    assert_screen_ok(&p);
    p.parse(b"\x1b[999999999999@"); // ICH huge
    assert_screen_ok(&p);
    p.parse(b"\x1b[999999999999P"); // DCH huge
    assert_screen_ok(&p);
}

// --------------------- Animated image support (Phase 3.3) ---------------------

fn make_gif_2frame() -> Vec<u8> {
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut enc = GifEncoder::new(&mut buf);
        let frame = |r: u8, g: u8, b: u8| {
            Frame::from_parts(
                RgbaImage::from_pixel(2, 2, Rgba([r, g, b, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            )
        };
        enc.encode_frames(vec![frame(255, 0, 0), frame(0, 255, 0)])
            .unwrap();
    }
    buf.into_inner()
}

#[test]
fn gif_animation_decodes_frames() {
    let gif = make_gif_2frame();
    assert!(gif.starts_with(b"GIF8"), "encoder wrote a GIF");
    let decoded = zeroterm_core::image_decode::decode_frames(&gif).unwrap();
    assert!(decoded.is_animated, "multi-frame GIF flagged animated");
    assert_eq!(decoded.frames.len(), 2, "two frames decoded");
    for f in &decoded.frames {
        assert_eq!((f.width, f.height), (2, 2), "canvas-sized frames");
        assert_eq!(f.rgba.len(), 2 * 2 * 4, "RGBA payload per frame");
    }
    assert!(
        decoded.frames.iter().any(|f| f.delay_ms > 0),
        "frame delay present"
    );
}

#[test]
fn static_png_still_renders() {
    use image::{ExtendedColorType, ImageFormat, Rgba, RgbaImage};
    let img = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut buf,
        img.as_raw(),
        1,
        1,
        ExtendedColorType::Rgba8,
        ImageFormat::Png,
    )
    .unwrap();
    let png = buf.into_inner();

    let seq = format!(
        "\x1b]1337;File=name=AA;size={};inline=1:{}\x07",
        png.len(),
        b64(&png)
    );
    let mut p = Parser::new(80, 24);
    p.parse(seq.as_bytes());
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1, "one pending image");
    assert_eq!((imgs[0].width, imgs[0].height), (1, 1), "dims from decode");
    assert_eq!(imgs[0].frames.len(), 1, "single static frame");
    assert_eq!(imgs[0].frames[0].rgba, vec![255, 0, 0, 255], "decoded RGBA");
    let reg = p.screen().image_registry();
    let stored = reg.get(&imgs[0].id).unwrap();
    assert_eq!(
        stored.rgba_data,
        vec![255, 0, 0, 255],
        "registry holds RGBA"
    );
}

#[test]
fn iterm1337_animated_gif_places_frames() {
    let gif = make_gif_2frame();
    let seq = format!(
        "\x1b]1337;File=name=AA;size={};inline=1:{}\x07",
        gif.len(),
        b64(&gif)
    );
    let mut p = Parser::new(80, 24);
    p.parse(seq.as_bytes());
    let imgs = p.take_pending_images();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].frames.len(), 2, "both GIF frames propagated");
    assert_eq!(imgs[0].width, 2, "dims auto-parsed from GIF canvas");
    assert_eq!(imgs[0].height, 2);
    let reg = p.screen().image_registry();
    let stored = reg.get(&imgs[0].id).unwrap();
    assert_eq!(stored.frames.len(), 2, "registry stores all frames");
}
