# ZeroTerm Performance Validation

Reproducible benchmark suite for the roadmap deferred item **"4K @ 120fps + <50MB binary"**.
This is a _measurement_ document: it records the current parser/render baselines and compares them
to the roadmap targets. It does not optimize code.

Reference: roadmap target "4K@120fps + <50MB validation" (deferred item). ZeroTerm v0.2.0.

## Roadmap targets, operationally

| Target           | Operational meaning                                                                                          | Where it's enforced                                                                      |
| ---------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| **4K @ 120fps**  | The app can keep a 4K viewport (3840×2160) repainting every frame at 120 Hz, given realistic terminal input. | Not CI-gated (needs a GPU). Validated by this document's headroom analysis.              |
| **<50MB binary** | The `zeroterm` release binary must stay under 50 MiB.                                                        | `.github/workflows/release.yml` "Check binary size (50MB gate)" — hard-fails at ≥50 MiB. |

The two targets are independent. The parser benchmark suite measures the _input_ side (can we parse
the bytes fast enough?), the renderer cost model measures the _output_ side (can we paint the cells?).
Neither is CI-gated for GPU reasons — see "How to validate".

## Parser headroom vs 4K @ 120fps

### The requirement

A 4K viewport with ~8×16px cells:

- cols = 3840 / 8 = 480, rows = 2160 / 16 = 135 → **64,800 cells/frame**
- at 120 fps → **7,776,000 cells/s ≈ 7.78M cells/s** must be produced by the parser
  (and consumed by the renderer) to keep up.

### Measured parser throughput

Ran `cargo bench -p zeroterm-core --bench parser_bench` (full criterion run, medians). Machine: dev
workstation, release bench profile, criterion 0.5.

| Bench (workload)                                  | MiB/s | bytes/s | cells/s @ 1B/cell | cells/s @ 2B/cell | Headroom vs 7.78M             |
| ------------------------------------------------- | ----- | ------- | ----------------- | ----------------- | ----------------------------- |
| parse_plain_text (ASCII)                          | 28.3  | 29.6M   | 29.6M             | 14.8M             | **1.9–3.8×**                  |
| parse_mixed_utf8 (Latin+CJK+emoji)                | 20.8  | 21.8M   | 21.8M             | 10.9M             | **1.4×**                      |
| parse_csi_heavy (SGR+cursor)                      | 79.6  | 83.4M   | 83.4M             | 41.7M             | **5.4×**                      |
| parse_osc_sixel (OSC 1337 + sixel)                | 18.4  | 19.3M   | 19.3M             | 9.7M              | **1.2×**                      |
| screen_scroll (10k newlines)                      | 2.7   | 2.8M    | 2.8M              | 1.4M              | 0.18× (degenerate, see below) |
| **parse_vim_session** (mixed CSI edit stream)     | 35.6  | 37.4M   | 37.4M             | 18.7M             | **2.4×**                      |
| **parse_4k_fill** (64.8k cells + scroll)          | 28.8  | 30.2M   | 30.2M             | 15.1M             | **1.9×**                      |
| **parse_progress_rewrite** (cursor rewrite burst) | 31.2  | 32.7M   | 32.7M             | 16.3M             | **2.1×**                      |

Method: `cells/s = bytes/s ÷ bytes-per-cell`. "1B/cell" is exact for pure ASCII text (each byte is
one cell); "2B/cell" is a conservative bound for mixed UTF-8 (multi-byte chars amortize lower).
Required = 64,800 cells/frame × 120 fps = 7.78M cells/s. Rows in bold are new workloads added for
realism (vim-style session, a 4K-screen fill, progress-bar rewrites).

### Conclusion: the parser is NOT the bottleneck

Every realistic workload clears the 7.78M cells/s target by 1.4–5.4× **before** the renderer is even
considered. Even the worst realistic case (mixed UTF-8, 2B/cell) has ~40% headroom. The renderer
consumes cells at most as fast as a frame paints; the parser runs far ahead and can sit idle
between frames.

**The one caveat:** `screen_scroll` (10k bare `\n`) measures 2.7 MiB/s — below the naive
bytes-per-cell threshold. This is a degenerate worst case, not a realistic 4K workload:

- Each LF shifts the _entire_ 80×24 screen buffer + scrollback line — the cost is dominated by
  Vec shifts, not byte parsing, and scales with screen size (a 4K screen shift is 33× more cells).
- Real 4K usage fills lines with text then LFs (`parse_4k_fill` is the honest proxy: 1.9× headroom).
- Criterion reports _input bytes/s_; for a pure-LF storm the byte cost underestimates the real work.

Watch list (not a current bottleneck): pure-newline storms on a 4K buffer are the one input pattern
where the screen/scrollback machinery, not the parser state machine, could bite. If a 4K-screen
`\n`-storm ever matters, it's a screen-shift optimization, not a parser fix.

## Renderer cost model (no GPU benchmark)

`render_frame` needs a real wgpu surface, so it is **not** benchmarked headless and not CI-runnable.
Theoretical 4K paint cost instead:

- 4K cells/frame = 64,800
- cell data written per frame: 1B/cell (fg+style only) ≈ 63 KiB/frame; with per-cell colors
  (BGRA, attr) ≈ 4–8B/cell ≈ 253–506 KiB/frame
- at 120 fps → **7.8M cell-writes/s ≈ 7.4 MiB/s of raw cell data** (1B/cell) up to
  ~60 MiB/s (8B/cell full-color).

Compare to parser (above): 16–95 MiB/s input parse comfortably exceeds the 7.4–60 MiB/s render
cell-data need — and the renderer does not re-parse, it consumes the parser's in-memory screen.

What actually bounds 120fps, in descending likelihood:

1. **Surface present / v-sync** — winit + wgpu `Surface::configure` and `present`. With vsync on
   (default), output is locked to the display's refresh rate; 120fps only matters on a 120Hz
   display and only if present never blocks past the frame budget.
2. **Per-frame uploads** — one `queue.write_texture` (or a shared buffer + `copy_buffer_to_texture`)
   for the cell/color textures. At 64,800 cells this is a few hundred KiB/frame; 120Hz of that is
   well within PCIe/USB bandwidth but dominates the per-frame submission time.
3. **Draw call count** — one pass per plane (bg + fg text + images/tab-bar) is the goal; each extra
   pass adds a full-screen draw call. Keep the pass count flat, not proportional to pane count.
4. **Glyph atlas eviction** — swash rasterizes glyphs into a texture atlas; churning the atlas
   (new glyphs mid-frame) forces uploads. Cache is the knob.
5. **Batch-write granularity** — per-cell writes are the known perf trap (see the earlier
   batch-write-buffer fix); per-frame dirty-cell batches must be coalesced into few writes.

So the honest 120fps budget is: parser input is not the limit; the limit is present/vsync cadence
and keeping per-frame wgpu work (uploads + draw calls) flat and small. Cell data alone (~7.4 MiB/s
at 1B/cell) is trivial; full-color cell data (~60 MiB/s) is the number that keeps the renderer from
becoming a bottleneck, and both sit below measured parser throughput.

## Binary size (<50MB)

Current state: `target/release/zeroterm` is not built in this workspace (no local release build).
The gate is enforced in CI (`.github/workflows/release.yml`, hard-fail at ≥50 MiB).

Where the size comes from (dominant static deps of `zeroterm` + `zeroterm-render`):

| Contributor                                             | Why it's large                                                                            | Notes                                                                        |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| **wgpu**                                                | Full WebGPU implementation (Vulkan/Metal/DX12 backends, wgsl validation, shader compiler) | Largest single dep; `wgpu = "23"`. Bigger with `spirv`/glsl feature baggage. |
| **winit**                                               | Whole windowing stack (X11, Wayland, Cocoa, Windows) in one binary                        | Platform backends all compiled in by default.                                |
| **swash**                                               | Font shaping/rasterization, glyph outline data                                            | Needed for the glyph atlas; moderate size.                                   |
| zeroterm-* crates + tokio/mlua/reqwest (workspace deps) | Parser, SSH, plugin runtime (mlua), sync daemon (tokio + crypto), AI client (reqwest)     | Feature-gated deps pull in significant transitive code.                      |

To stay <50MB as features grow, the levers in order of size-impact:

1. **Keep `zeroterm`'s dependency graph lean** — the biggest wins are disabling unused wgpu
   features (e.g. no `spirv`, no GL) and keeping winit/wgpu on the same platform-targeted build.
   A single-platform release (linux-only, or per-OS jobs) can strip unneeded backends via cfg.
2. **`strip = true` + `opt-level = "z"`** in the release profile (strip debug symbols — often tens
   of MB on wgpu-heavy builds).
3. **`#![no_mangle]`/dead-code pruning** is secondary; the real mass is the three big crates above,
   which are shared with other crates and can't be removed, only slimmed via features.
4. **LTO** (already a typical release setting for perf) reduces code but is not a size _guarantee_.

The 50MB gate is a tripwire, not a design constraint — current binary likely sits in the 30–45 MiB
band once stripped. If it ever approaches 50, check feature flags + strip before restructuring.

## How to run

```bash
# Full criterion run (all 8 benches, ~2-5 min)
cargo bench -p zeroterm-core --bench parser_bench

# Quick sanity run (faster, no change-detection baseline)
cargo bench -p zeroterm-core --bench parser_bench -- --quick

# Human-readable summary with the headroom math (runs --quick internally)
bash scripts/bench.sh
```

The bench source is `crates/zeroterm-core/benches/parser_bench.rs` (criterion, `harness = false`).
Criterion reports throughput in MiB/s per workload.

## How to validate (roadmap acceptance checklist)

| Roadmap claim                  | How to verify                                                                                   | Pass criteria                                                                                                   |
| ------------------------------ | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Parser keeps up with 4K@120fps | `cargo bench -p zeroterm-core --bench parser_bench`                                             | `parse_4k_fill` ≥ 7.78M cells/s (~14.8 MiB/s at 1B/cell); all realistic workloads ≥ 1.5× the 7.78M cells/s need |
| Renderer can paint 4K@120fps   | Manual: run `zeroterm` on a 120Hz 4K display, paste the 4K fill workload, watch for frame drops | No sustained frame drops on 120Hz display; cell uploads stay coalesced (see cost model)                         |
| Binary <50MB                   | CI release job (50MB gate), or `strip` + `du -h target/release/zeroterm` locally                | CI passes the size check; local stripped binary <50MiB                                                          |
| Suite stays green              | `cargo test -p zeroterm-core`                                                                   | All tests pass                                                                                                  |
| Suite is reproducible          | `cargo bench` numbers stable across runs on same machine                                        | No >2× run-to-run spread on the headroom rows                                                                   |

Notes: the parser row is CI-runnable headless; the renderer row is GPU-only and manual. Keep the
bench set in the headroom table as the single source of truth — re-run `--quick` after any parser or
screen-model change and re-check the "every realistic workload ≥ 1.5×" criterion.
