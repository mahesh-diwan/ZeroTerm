//! Criterion benchmarks for the VT parser.
//!
//! Run: `rtk cargo bench -p zeroterm-core --bench parser_bench`
//! Quick sanity run: `rtk cargo bench -p zeroterm-core --bench parser_bench -- --quick`

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use zeroterm_core::parser::Parser;

const COLS: usize = 80;
const ROWS: usize = 24;

fn plain_text() -> Vec<u8> {
    "The quick brown fox jumps over the lazy dog. 0123456789\r\n"
        .repeat(200)
        .into_bytes()
}

fn mixed_utf8() -> Vec<u8> {
    let mut v = String::new();
    for i in 0..400 {
        v.push_str("héllo wörld ");
        v.push('\u{4e2d}');
        v.push(' ');
        v.push('\u{1F389}');
        v.push_str(&format!("\r\nline {i}\r\n"));
    }
    v.into_bytes()
}

fn csi_heavy() -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..500 {
        v.extend_from_slice(format!("\x1b[38;5;{}m", i % 256).as_bytes());
        v.extend_from_slice(b"\x1b[48;5;10m");
        v.extend_from_slice(b"\x1b[1mX\x1b[0m");
        v.extend_from_slice(b"\x1b[1;2H\x1b[10C\x1b[2K");
    }
    v
}

fn osc_and_sixel() -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..100 {
        v.extend_from_slice(format!("\x1b]0;café title {i}\x07").as_bytes());
        v.extend_from_slice(b"\x1bPq#0;2;0;0;0;0~-{}#$@!?~\x1b\\");
        v.extend_from_slice(b"\x1b]1337;File=name=AA;size=1;inline=1:AAAA\x07");
    }
    v
}

fn newline_storm() -> Vec<u8> {
    vec![b'\n'; 10_000]
}

fn vim_session() -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..200 {
        v.extend_from_slice(b"\x1b[1;7m-- INSERT --\x1b[0m\r\n");
        for line in 0..24 {
            v.extend_from_slice(b"\x1b[38;5;70m");
            v.extend_from_slice(b"fn main() { println!(\"hi\"); } // ");
            v.extend_from_slice(format!("line {line}\r\n").as_bytes());
            v.extend_from_slice(b"\x1b[0m");
        }
        v.extend_from_slice(b"\x1b[5;10H\x1b[K\x1b[P");
        v.extend_from_slice(b"\x1b[2L\x1b[2M");
        v.extend_from_slice(b"\x1b[10;5H\x1b[10C\x1b[2;15r");
        v.extend_from_slice(b"\x1b7\x1b[24;1H\x1b[0m~ \x1b8");
        v.extend_from_slice(b"\x1b[H\x1b[2J");
    }
    v
}

fn four_k_fill() -> Vec<u8> {
    let mut v = Vec::with_capacity(64_800 * 2);
    let line = b"the quick brown fox jumps over the lazy dog 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    for _ in 0..(64_800 / line.len()) {
        v.extend_from_slice(line);
        v.push(b'\n');
    }
    v
}

fn progress_rewrite() -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..400 {
        v.extend_from_slice(b"\x1b[G\x1b[K");
        v.extend_from_slice(format!("Progress: {}% [", i * 100 / 400).as_bytes());
        v.extend_from_slice(&b"#".repeat(i % 40));
        v.extend_from_slice(b"]\r");
    }
    v
}

fn bench_plain(c: &mut Criterion) {
    let data = plain_text();
    let mut g = c.benchmark_group("parse_plain_text");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("10k_ascii", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_mixed(c: &mut Criterion) {
    let data = mixed_utf8();
    let mut g = c.benchmark_group("parse_mixed_utf8");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("latin_cjk_emoji", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_csi(c: &mut Criterion) {
    let data = csi_heavy();
    let mut g = c.benchmark_group("parse_csi_heavy");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("sgr_cursor_moves", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_osc_sixel(c: &mut Criterion) {
    let data = osc_and_sixel();
    let mut g = c.benchmark_group("parse_osc_sixel");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("osc1337_sixel", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_scroll(c: &mut Criterion) {
    let data = newline_storm();
    let mut g = c.benchmark_group("screen_scroll");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("10000_newlines", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_vim(c: &mut Criterion) {
    let data = vim_session();
    let mut g = c.benchmark_group("parse_vim_session");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("mixed_csi_scroll_edits", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_4k_fill(c: &mut Criterion) {
    let data = four_k_fill();
    let mut g = c.benchmark_group("parse_4k_fill");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("64800_cells_plus_scroll", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

fn bench_progress(c: &mut Criterion) {
    let data = progress_rewrite();
    let mut g = c.benchmark_group("parse_progress_rewrite");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("cursor_rewrite_burst", |b| {
        let mut p = Parser::new(COLS, ROWS);
        b.iter(|| p.parse(black_box(&data)));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_plain,
    bench_mixed,
    bench_csi,
    bench_osc_sixel,
    bench_scroll,
    bench_vim,
    bench_4k_fill,
    bench_progress
);
criterion_main!(benches);
