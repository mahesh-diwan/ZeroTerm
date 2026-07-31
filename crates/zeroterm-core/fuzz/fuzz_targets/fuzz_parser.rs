#![no_main]

use libfuzzer_sys::fuzz_target;
use zeroterm_core::parser::Parser;

fuzz_target!(|data: &[u8]| {
    let mut p = Parser::new(80, 24);
    if data.is_empty() {
        p.parse(data);
        return;
    }
    // Split input into 1-5 chunks: exercises state persistence across parse() calls.
    // Boundaries derived from data itself — deterministic, no rand dep.
    let n_chunks = (data[0] as usize % 5) + 1;
    let step = data.len() / n_chunks;
    for i in 0..n_chunks {
        let start = i * step;
        let end = if i == n_chunks - 1 {
            data.len()
        } else {
            start + step
        };
        p.parse(&data[start..end]);
    }
});
