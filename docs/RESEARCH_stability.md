# Stability research — Alacritty + Windows Terminal

Research re-run against live source: `alacritty/alacritty` master and `alacritty/vte` master (both MIT/Apache-2.0), `microsoft/terminal` main + MS Console docs. All constants/mechanisms below verified from the cited files (raw.githubusercontent.com, 2026-07). Note: earlier memory claimed a 1 KiB OSC cap is active in Alacritty — **corrected below**: `MAX_OSC_RAW = 1024` is enforced only in vte's `no_std` build; Alacritty compiles vte with the `std` feature, where the OSC raw buffer is an unbounded `Vec`.

## Alacritty (MIT/Apache — legally copyable)

### 1. PTY read loop & backpressure — `alacritty_terminal/src/event_loop.rs`

- `pub(crate) const READ_BUFFER_SIZE: usize = 0x10_0000;` — 1 MiB read buffer. Doc comment: "Max bytes to read from the PTY before forced terminal synchronization."
- `const MAX_LOCKED_READ: usize = u16::MAX as usize;` — 65535 bytes. `pty_read()` accumulates reads while holding the terminal lock but breaks the batch once `processed >= MAX_LOCKED_READ`, so the renderer can get the lock.
- Backpressure ladder inside `pty_read()`:
  1. `self.terminal.lease()` — reserves the FairMutex `next` slot before reading, so the reader queues ahead of new renderer lockers.
  2. `try_lock_unfair()` — parse opportunistically without forcing a lock.
  3. `None if unprocessed >= READ_BUFFER_SIZE => self.terminal.lock_unfair()` — at the 1 MiB cap, **block** on the lock (mandatory synchronization) rather than let the PTY ring buffer overrun.
  4. `None => continue` — otherwise yield: keep reading but don't block on the lock; lock the terminal again on a later iteration.
- Read-error tolerance:
  - `ErrorKind::Interrupted | ErrorKind::WouldBlock` → continue reading if bytes were already collected, else break back to the poller.
  - Linux EIO on the master side: `#[cfg(target_os = "linux")] if err.raw_os_error() == Some(libc::EIO) { continue; }` with the comment "a `read` on the master side of a PTY can fail with `EIO` if the client side hangs up … just loop back round for the inevitable `Exited` event."
- `drain_on_exit`: on `ChildEvent::Exited`, `if self.drain_on_exit { let _ = self.pty_read(...) }` runs **before** `self.terminal.lock().exit()`, so the last bytes the child wrote (e.g. a final prompt / error message) are not lost.
- Sync-aware redraw: `if state.parser.sync_bytes_count() < processed && processed > 0 { send Wakeup }` — bytes absorbed by a DECSET-2026 sync buffer do **not** trigger a redraw.
- Write side (`pty_write`): per-message `Writing { source, written }` cursor; `Interrupted | WouldBlock` → break out and keep the partial message in `state` to resume later (`needs_write()` re-registers write interest on the poller).
- Event loop runs on a dedicated "PTY reader" thread (`thread::spawn_named("PTY reader", ...)`).

### FairMutex — `alacritty_terminal/src/sync.rs`

```rust
pub struct FairMutex<T> { data: Mutex<T>, next: Mutex<()> }
```

- `lock()` takes `next` then `data`; `lease()` takes `next` alone (reserves the lock, blocks if a lease is held). Unfair paths (`try_lock_unfair`, `lock_unfair`) bypass `next` for latency-critical use. Guarantees a waiter is served before the current holder can re-lock — prevents reader/renderer starvation.

### 2. Parser robustness — vte crate

**Fixed-size params — `vte/src/params.rs`:**

- `pub(crate) const MAX_PARAMS: usize = 32;` — `params: [u16; MAX_PARAMS]`, `subparams: [u8; MAX_PARAMS]`. No heap allocation in the hot path.
- `is_full()` → callers set `ignoring = true`; remaining bytes of the sequence are swallowed (see below).

**Overflow → ignore flag — `vte/src/lib.rs`:**

- `const MAX_INTERMEDIATES: usize = 2; const MAX_OSC_PARAMS: usize = 16; const MAX_OSC_RAW: usize = 1024;`
- `action_collect`: `if self.intermediate_idx == MAX_INTERMEDIATES { self.ignoring = true }` (>2 intermediates).
- `action_param` / `action_subparam` / `action_paramnext`: when `params.is_full()` (32 reached) → `ignoring = true`. Param arithmetic is saturating over `u16` (`saturating_mul(10)`, `saturating_add`) — a 19-digit parameter clamps to `u16::MAX` instead of overflowing (test `parse_long_csi_param`).
- `Perform::csi_dispatch(…, ignore, …)` passes the flag to the handler, which may reject the whole sequence.
- **OSC cap — corrected finding:** `osc_raw: ArrayVec<u8, MAX_OSC_RAW>` only when compiled `no_std`; with `#[cfg(feature = "std")]` it is `osc_raw: Vec<u8>` — unbounded. Alacritty's `alacritty_terminal/Cargo.toml` declares `vte = { version = "0.15.0", default-features = false, features = ["std", "ansi"] }`, so **Alacritty itself does not enforce the 1 KiB OSC buffer**; the std-mode test `exceed_max_buffer_size` asserts the param slice grows to `MAX_OSC_RAW + 100` bytes. The practical caps in Alacritty are: 16 OSC params (`MAX_OSC_PARAMS`, extra `;` separators dropped), saturating u16 params, and whatever the `Handler` validates. OSC is **bell-terminated** (`0x07`) as well as ST (`ESC \`) — `advance_osc_string`: `0x07 => osc_end(...); state = Ground`, and `osc_dispatch(performer, byte == 0x07)` reports the terminator so handlers can differ on it.

**Partial-UTF-8 across `advance()` calls — `vte/src/lib.rs`:**

- `partial_utf8: [u8; 4]` + `partial_utf8_len` live in the `Parser` struct, so a codepoint split across PTY read boundaries survives until the next `advance()`. `advance_ground` uses `memchr::memchr(0x1B, bytes)` + `str::from_utf8` to dispatch whole plain-text runs; on `Err` it dispatches `valid_up_to()` bytes, emits U+FFFD for invalid bytes (or `execute()` for lone C1 bytes `<= 0x9F`), and buffers the trailing partial codepoint in `partial_utf8` for the next call.

### 3. Resize handling — `alacritty_terminal/src/grid/resize.rs`

- `grow_lines`: pulls rows from history first — `let from_history = min(history_size, lines_added);` — before `scroll_up` for the remainder; cursor + saved cursor offset down by `from_history`; display offset clamped. Keeps the cursor at the bottom while scrollback exists.
- `shrink_lines`: `scroll_up` first so content stays in the viewport, then clamps primary and saved cursors to `Line(target - 1)`, then `raw.rotate((self.lines - target) as isize)` + `shrink_visible_lines`.
- Column shrink/grow reflows wrapped lines (WRAPLINE flags), preserving content across width changes; wide-char spacers (`LEADING_WIDE_CHAR_SPACER`) prevent orphaned wide glyphs; `display_offset` clamped to `history_size()` afterwards.
- `alacritty/src/config/scrolling.rs`: scrollback default **10,000** lines, max configurable **100,000** (`MAX_SCROLLBACK_LINES`).

### 4. Error recovery / EOF / child exit

- Decode errors → U+FFFD (above). EOF → `Ok(0)` breaks the read loop, waits for `ChildEvent::Exited`; then optional drain (`drain_on_exit`) → `Term::exit()` → `ChildExit` event to the UI, event loop tears down, deregisters the pty from the poller. Linux EIO tolerated as non-fatal (section 1).

### 5. Scrollback ring buffer — `alacritty_terminal/src/grid/storage.rs`

- `const MAX_CACHE_SIZE: usize = 1_000;` — "Maximum number of buffered lines outside of the grid for performance optimization."
- Ring buffer via modular arithmetic, not `Vec::rotate`: `zero: usize` (bottommost line offset) + `len` (live lines) + `inner` (allocation). `Index`/`IndexMut` reimplemented to map `Line` → physical index with an if/else over `zeroed >= inner.len()` (deliberately avoids `%` in the hot path).
- Lazy truncate on shrink: `shrink_lines`: `self.len -= shrinkage; if self.inner.len() > self.len + MAX_CACHE_SIZE { self.truncate(); }` — frees memory only once 1000 dead lines accumulate, so shrink-then-grow cycles reuse the existing allocation.
- Grow: `initialize()` reallocates to `inner.len() + max(additional_rows, MAX_CACHE_SIZE)` — always overallocates ≥1000 rows to amortize.
- `rotate`/`rotate_down` are O(1) `zero` updates (rows wrap around the buffer without moving).

### 6. DECSET 2026 synchronized output — `vte/src/ansi.rs`

- `const SYNC_UPDATE_TIMEOUT: Duration = Duration::from_millis(150);` — max time a sync update may hold rendering.
- `const SYNC_BUFFER_SIZE: usize = 0x20_0000;` — 2 MiB byte ceiling per update.
- `const SYNC_ESCAPE_LEN: usize = 8; const BSU_CSI: [u8;8] = *b"\x1b[?2026h"; const ESU_CSI: [u8;8] = *b"\x1b[?2026l";`
- `Processor::advance()`: while a sync is pending, `advance_sync()` **buffers** bytes (`sync_state.buffer`) and only scans for BSU/ESU (`advance_sync_csi`, `memchr::memchr_iter(0x1B, …)` over the tail of the buffer); `terminated()` makes `advance_until_terminated` stop at an ESU so the handler's renderer state flushes atomically.
- Ceiling: `if buffer.len() + bytes.len() >= SYNC_BUFFER_SIZE - 1 { stop_sync(...) }` — a runaway sync is force-flushed at 2 MiB.
- Timeout flush: `event_loop.rs` computes `handler.sync_timeout()` → `poll.wait(&mut events, timeout)`; when the poll times out with no events/channel messages: `state.parser.stop_sync(&mut *self.terminal.lock()); send Wakeup` — unterminated syncs are flushed after 150 ms.
- Alacritty registers `NamedPrivateMode::SyncUpdate = 2026`; the escape sequences are handled entirely inside vte (handler never sees them).

## Windows Terminal (MIT — legally copyable)

### 7. ConPTY lifetime & threading — `microsoft/terminal` `src/winconpty/winconpty.h` + MS Console docs

- **`PseudoConsole` ABI struct** (`src/winconpty/winconpty.h`):
  ```c
  typedef struct _PseudoConsole {
      HANDLE hSignal;        // anonymous pipe for out-of-band PTY_SIGNAL_* messages
      HANDLE hPtyReference;  // \Device\ConDrv\Reference handle, inherited by children
      HANDLE hConPtyProcess; // conhost process handle
  } PseudoConsole;
  ```
- **hPtyReference refcount = conhost lifetime.** Header comment (verbatim sense): `hPtyReference` is a child of the conhost "server handle" (`\Device\ConDrv\Server`); console processes inherit the reference handle to talk to the console server. "When the reference count of the `\Reference` handle drops to 0, it'll release its reference to the server handle," and the server handle breaks the IPC pipe once the refcount drops to 1 (i.e. only conhost is left). "As long as hPtyReference exists it'll keep the server handle alive and thus keep conhost alive. Closing this handle will make conhost exit." Handle inheritance via `CreateProcess` makes this safe even if the terminal process dies mid-spawn.
- **Synchronous-I/O restriction** (`learn.microsoft.com/en-us/windows/console/createpseudoconsole`): the `hInput`/`hOutput` channels "are currently restricted to synchronous I/O" (`ReadFile`/`WriteFile` without `OVERLAPPED`). No async I/O on ConPTY.
- **UTF-8** (same doc): "The input and output streams encoded as UTF-8 contain plain text interleaved with Virtual Terminal Sequences" — ConPTY emits UTF-8 on output; terminal must handle partial UTF-8 at pipe boundaries (same as vte's `partial_utf8`).
- **1 thread per channel** (`learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session`, Warning): "we highly recommend that each of the communication channels is serviced on a separate thread that maintains its own client buffer state and messaging queue … Servicing all of the pseudoconsole activities on the same thread may result in a deadlock where one of the communications buffers is filled and waiting for your action while you attempt to dispatch a blocking request on another channel."
- **Handle discipline** (same doc): after `CreateProcess`, the terminal must close its copies of the parent-side handles — "This will decrease the reference count on the underlying device object and allow I/O operations to properly detect a broken channel" — otherwise reads block forever after the child exits.
- **Close semantics** (same doc): `ClosePseudoConsole` terminates the attached client app and its whole process tree; closing "may emit a final frame update to `hOutput` which should be drained"; with `PSEUDOCONSOLE_INHERIT_CURSOR` the cursor query must be answered asynchronously (reply on `hInput`) or the close deadlocks. The output channel must keep being drained until it breaks on its own.

## Take-aways for ZeroTerm

1. **Bounded, cap-aware PTY read loop** — `crates/zeroterm/src/main.rs` PTY reader thread (buf `[0u8; 4096]`, plain blocking `read`). Adopt Alacritty's pattern: larger buffer (~1 MiB) + a per-batch parse ceiling (Alacritty: 64 KiB / `MAX_LOCKED_READ`), tolerate `EIO` on Linux during teardown, and drain the PTY once before treating child-exit as final so the tail of output is not lost.
2. **Hard caps in the parser** — `crates/zeroterm-core/src/parser.rs` currently has **unbounded** `CsiParams.params: Vec<Option<i64>>`, `intermediates: Vec<char>`, `osc_buffer: Vec<u8>`, `dcs_buffer: Vec<u8>`; a hostile escape (`ESC [` + thousands of `;`) grows memory without bound. Add vte-style fixed caps: 32 params, 2 intermediates, ~1 KiB OSC/DCS raw, saturating u16 param arithmetic, and an `ignoring` flag so over-limit sequences are swallowed instead of interpreted. OSC bell-termination (`0x07`) already exists — keep it.
3. **Ring-buffer scrollback with lazy truncate** — replace ZeroTerm's linear `scrollback: Vec`/`buffer: Vec` split in `crates/zeroterm-core/src/screen.rs` with a `zero`/`len` modular ring: O(1) rotate, `MAX_CACHE_SIZE`-style slack (Alacritty: 1000 rows) before freeing memory, so shrink-then-grow reuses allocation. Default history ~10k lines.
4. **Resize = pull-from-history / push-to-scrollback, clamp cursors** — in `screen.rs` resize: grow by moving rows out of history first; shrink by scrolling content up into the viewport and clamping primary/saved cursor to the new bounds; reflow wrapped lines on column change.
5. **DECSET 2026 synchronized output** — buffer parsed output while `ESC[?2026h` is active and defer redraw until `ESC[?2026l`, with a hard flush at 150 ms / 2 MiB (vte constants). ZeroTerm has no 2026 handling today; this removes mid-frame flicker from programs that batch-render (vim, cava, etc.).
6. **ConPTY (future Windows port)** — follow the winconpty lifetime model: keep a reference handle alive for the child's whole lifetime, close parent-side pipe copies after `CreateProcess` (else reads never EOF), service input and output on **separate threads** (synchronous I/O only), and keep draining output through teardown since `ClosePseudoConsole` kills the child tree and can emit a final frame.

# Stability research — Ghostty + WezTerm

Source-verified against upstream `main` (2026-07). Both projects are MIT-licensed — code and mechanisms are legally copyable into ZeroTerm (MIT). Companion doc: `RESEARCH_stability.md` (kitty GPLv3 = design-ideas only; foot MIT = copyable).

Licenses:

- Ghostty — MIT — github.com/ghostty-org/ghostty (Zig). Terminal engine: `src/terminal/`, pty IO: `src/termio/`, renderer: `src/renderer/`.
- WezTerm — MIT — github.com/wez/wezterm (Rust). Mux/pty: `mux/src/`, parser: `vtparse/src/` + `wezterm-escape-parser/src/`, terminal state: `term/src/`.

---

## Ghostty (MIT — legally copyable)

### Architecture: 1 pty reader thread + 1 renderer thread, message-passing only

Per-surface: a `termio.Thread` (pty IO + parse) and a `renderer.Thread` (draw). Spawned in `src/apprt/embedded/Surface.zig:722-733`. All cross-thread state moves through bounded SPSC mailboxes:

- termio→renderer mailbox: `BlockingQueue(renderer.Message, 64)` — `src/renderer/Thread.zig:37`
- termio writer mailbox: `BlockingQueue(termio.Message, 64)` — `src/termio/mailbox.zig:18` (comment: capacity "hardcoded to a value that empirically has made sense for Ghostty usage")
- surface→app mailbox: `BlockingQueue(App.Message, 64)` — `src/App.zig:563`

Bounded queues (64) mean a slow consumer can never make a producer allocate unboundedly; a full queue makes the producer wait instead.

### M1 — Two-stage pty read pipeline, bounded buffers → kernel backpressure

`src/termio/Exec.zig` (ReadThread): pty output is gathered by one stage into a ring of fixed buffers, and a second stage parses it. Constants:

- `buffer_count = 4`, `buffer_capacity = 64 * 1024` — the gather ring totals 4×64 KiB, deliberately small.
- `bridge_threshold = 1024` — handoff granularity between stages.

The loop blocks when the ring is full. Because the child writes into the kernel pty queue, a full ring backs up into the kernel queue, which blocks the child's write. Result: no unbounded userspace buffering; the producer (shell/AI CLI) is throttled by the terminal, not the terminal's heap. EAGAIN is spin-retried before the loop falls back to `poll` (fds: pty, quit pipe, pipeline idle pipe).

### M2 — Mutex-protected render state with demand handoff (no starvation)

`src/renderer/State.zig` guards the live terminal (`terminal`, `inspector`, `preedit`, `mouse`) with `*std.Io.Mutex`. The parser thread (IO) locks it per parse-batch; the renderer locks it per frame snapshot.

Because Ghostty's mutex is unfair (barging), a saturated parse thread could starve the renderer's frame lock indefinitely. Fix: a `demand` atomic counter + `handoff_gen`:

- `lockDemand()` / `unlockDemand()` — when the renderer needs the lock it registers demand; the holder checks demand and yields.
- `yieldToDemand()` — the IO thread, before releasing, checks whether a renderer is waiting; if so it bumps the generation and lets the renderer in first.

Landed as PR #13265 (commit `11b9a6e`, "renderer: hand off state mutex to avoid starving frames"). This is the canonical fix for "renderer freezes while a fast CLI streams output".

### M3 — Two-phase render snapshot (`beginUpdate`/`endUpdate`)

`src/renderer/generic.zig` + `src/terminal/render.zig:326-732` (RenderState): the terminal lock is held ONLY inside `beginUpdate` (copy the visible rows / pin the viewport / consume dirty flags). All per-cell work (style denormalization, atlas writes) happens in `endUpdate` WITHOUT the terminal lock. `src/renderer/generic.zig` additionally takes `draw_mutex` and, when the terminal is in synchronized-output mode, pauses rendering entirely (`if (state.terminal.modes.get(.synchronized_output)) { ... }`).

RenderState validates against page `serial`s: `row_serials[y] != node_serial` ⇒ row changed; unchanged rows are skipped wholesale (cheap no-op frames under cursor-only updates).

### M4 — Synchronized output (DEC 2026) with a 1000 ms watchdog reset

- Mode registered as `synchronized_output` (DEC 2026) — `src/terminal/modes.zig:282`.
- `src/termio/Thread.zig`: when sync-output becomes active, a timer (`sync_reset_ms = 1000`) is armed. If the terminator (DEC 2026 OFF) never arrives within 1 s, the termio thread calls `resetSynchronizedOutput()`.
- `src/termio/Termio.zig:536` `resetSynchronizedOutput`: locks the renderer-state mutex, clears the mode, and `renderer_wakeup.notify()` — unblocks the renderer even if it was paused on the mode flag.
- `src/terminal/Terminal.zig:3760-3794` (resize): rejects `cols == 0 || rows == 0` (`error.InvalidValue`), **clears synchronized_output on resize**, rolls back via `errdefer`, and saturates pixel-geometry multiplication overflow to `maxInt(u32)`.

The watchdog is the escape hatch: a malformed/never-terminated DEC 2026 sequence can freeze rendering for at most 1 s.

### M5 — Inline-image (kitty) memory limits

- `src/terminal/kitty/graphics_storage.zig:133`: `total_limit: usize = 320 * 1000 * 1000, // 320MB` — hard cap on total image storage. At the cap, oldest / unused-first images are evicted; lowering the limit evicts immediately; setting it to 0 disables the protocol.
- `src/terminal/kitty/graphics_command.zig`: the APC parser enforces a `max_bytes` cap on the data payload — "to prevent malicious input from causing us to allocate too much memory" (~4096 default per comment).

### M6 — Config errors never panic: defaults + clamped fields

`src/config/Config.zig`: `load()` = build `default()` first, then overlay file/CLI/recursive sources, then `finalize()`. Field doc-comments declare clamping instead of rejection (e.g. "clamped to [0.01, 10000]", "clamped to the maximum value"). A bad config file yields a degraded-but-running config, not a crash. Error lists surface via `src/config/ErrorList.zig`.

### M7 — Worker threads log errors instead of panicking

`src/renderer/Thread.zig`: `threadMain` catches errors → `log.warn("error in renderer err={}", ...)`; `drawFrame` failures are `catch |err| log.warn(...)` — a draw hiccup never takes down the process. Crash reporting (Sentry client + crash-report dir, `src/crash/`) is a separate opt-in package, not a runtime dependency.

### Known instability bug classes (tracker, all fixed unless noted)

1. **Renderer starvation by saturated parse thread** — PR #13265 / `11b9a6e` (M2 above). Symptom: frozen frames while streaming output.
2. **Mailbox-full deadlock** — issue #9224/#9191: the IO thread filled the renderer mailbox while holding the terminal lock → deadlock. Fix: `rendererMailboxWriter` helper — when the mailbox is full, release the lock, wake the renderer, retry.
3. **DEC 2026 + DECSTBM rendering corruption** — issue #12685 (open, 2026-05): "Progressive rendering corruption (fuzzy/ghosting text) with DEC 2026 + cursor-positioned status bar updates", reproduced with AI CLIs (claude/gemini). DEC 2026 with active scroll regions/status-line writes desyncs the renderer snapshot.
4. **Renderer wakeup handle copied by value on Windows** — discussion #11877 (embedded build, Windows-specific async handle semantics).

---

## WezTerm (MIT — legally copyable)

### Architecture: pty reader thread + parser thread joined by a 1 MB socketpair

`mux/src/lib.rs`: two threads per pane.

- `read_from_pane_pty` (line 283): a reader thread does **blocking** reads from the pty into `vec![0; BUFSIZE]` and writes to one end of a socketpair.
- `parse_buffered_data` (line 142): a parser thread reads the socketpair and feeds `Perform`.

The transport is a `socketpair` sized `BUFSIZE = 1024 * 1024` (line 118) with SO_SNDBUF/SO_RCVBUF set to match. A `dead: AtomicBool` terminates both threads on error. Invalid UTF-8 input is logged, never fatal.

### M1 — Block-on-full = kernel backpressure (socketpair as a brake)

The reader thread's `write` to the socketpair blocks when the peer's 1 MB socket buffer is full; that backs up the pty kernel queue; the child blocks. Bounded memory regardless of output rate, exactly like Ghostty M1 but with the kernel socket buffer as the buffer instead of a userspace ring.

### M2 — Output coalescing before the renderer

`parse_buffered_data` (mux/src/lib.rs): after draining the socket, parsed actions are delivered in batches, coalesced by:

- `mux_output_parser_coalesce_delay_ms` = `3` ms — `config/src/config.rs:1669-1675` (`default_mux_output_parser_coalesce_delay_ms() -> u64 { 3 }`)
- `mux_output_parser_buffer_size` = `128 * 1024` (`default_mux_output_parser_buffer_size() -> usize { 128 * 1024 }`)

Batch sizes are bounded by that 128 KiB buffer; the renderer is never fed a single-event-per-message flood.

### M3 — Synchronized output hold + SoftReset escape hatch + EOF tail-drain

DEC 2026 handling lives in the mux parser thread (`mux/src/lib.rs`, "wezterm's mux" — `term/src/terminalstate/mod.rs:1573-1589` defers it there):

- While sync-output is `hold`, parsed actions are accumulated in a buffer instead of being sent onward.
- The hold is broken by an explicit escape hatch: `Action::CSI(CSI::Device(dev)) if matches!(**dev, Device::SoftReset)` (mux/src/lib.rs:183) — RIS/SoftReset force-flushes.
- EOF tail-drain (mux/src/lib.rs:240-246): on pty EOF, "Don't forget to send anything that we might have buffered" — buffered actions are flushed so a trailing partial update isn't lost.

### M4 — Parser state-machine caps (vtparse)

`vtparse/src/lib.rs:311-313`:

- `MAX_INTERMEDIATES = 2`
- `MAX_OSC = 64` (max OSC bytes _collected into the OSC string_)
- `MAX_PARAMS = 256`

**Gap to flag**: `OscState` uses `buffer: Vec<u8>` under the `std`/`alloc` feature — an **unbounded OSC payload** under std (bounded only by RAM). Only the `no_std` build caps it (`heapless::Vec<u8, { MAX_OSC * 16 }>`). WezTerm relies on `MAX_OSC = 64` being enforced by the _no_std_ path; the std path is a documented-class OOM risk if a malicious/app-buggy peer emits one giant OSC. (Mirrored upstream in a tracker issue; the mechanism to copy is the cap, not this gap.)

### M5 — Sixel / PDU bounds (fixes CVE-2022-24130)

`wezterm-escape-parser/src/parser/sixel.rs`:

- `MAX_SIXEL_SIZE = 100_000_000` (100 MB)
- `MAX_PARAMS = 5`
- `push()`: if the buffer would exceed `MAX_SIXEL_SIZE` (or overflow), the buffer is cleared and the data ignored (logged), `pixel_width/pixel_height = None`.

**CVE-2022-24130** (issue #1610): a sixel repeat-count triggered a ~103 GB allocation crash ("memory allocation of 103079215056 bytes failed / Aborted"). Fixed by the 100 MB cap (commit `7577eb3`). The cap is the fix — not validation of the repeat count itself.

`term/src/terminalstate/sixel.rs` additionally calls `check_image_dimensions(width, height)` and returns early on absurd sizes; `term/src/terminalstate/iterm.rs` rejects zero-pixel images and "Unable to decode image" on decode failure.

### M6 — Image cache is LRU-bounded

`term/src/terminalstate/mod.rs:358,569`: `image_cache: lru::LruCache<[u8; 32], Arc<ImageData>>` with capacity `NonZeroUsize::new(16).unwrap()` — 16 cached images, oldest evicted. Decoded bitmaps never grow without bound.

### M7 — Config fails to defaults

`config/src/lib.rs`: `ConfigInner { config, error: Option<String>, warnings }`; `configuration_result()` bails to a known-good fallback when a config file is unparseable; `config::default_config()` is the fallback. `config/src/config.rs:1699-1711` clamps scrollback: `default_scrollback_lines() = 3500`, `MAX_SCROLLBACK_LINES = 999_999_999`, `validate_scrollback_lines` rejects the rest. Unknown/unhandled terminal sequences are `log::warn`ed, never panicked (`term/src/terminalstate/performer.rs:339,349,358,485,579`).

### Known instability bug classes (tracker, all fixed unless noted)

1. **Sixel OOM crash** — CVE-2022-24130 (issue #1610): repeat-count-driven allocation, ~103 GB. Fixed by `MAX_SIXEL_SIZE` (M5).
2. **Unbounded PDU memory allocation** — issue #7527 → commit `d0c7326` ("fix(codec): bound PDU data allocation to avoid OOM crash"): PDU length fields capped (256 MB) so a forged length cannot preallocate.
3. **ST never returns to ground state** — PR #2341 ("Ensure parse_first_as_vec groups ST with OSC sequence", fixes markbt/streampager#57): a TMUX/OSC terminator `ESC \` must close the OSC/string and return to ground, else the parser eats subsequent output into the wrong state.
4. **Long CSI sequences mis-parsed** — changelog entries #5161, #6194.
5. **Panic on very long lines** — commit `7aadebfe4` ("fixed panic with very long lines", ClusteredLine).
6. **Stack overflow with tmux -CC** — commit `574e53190`.

---

## Take-aways for ZeroTerm

1. **Two-stage pty read with fixed 64 KiB batches** (Ghostty `Exec.zig`: 4×64 KiB ring; WezTerm: 1 MB socketpair). The `crates/zeroterm-mux` read loop should read fixed 64 KiB batches, parse in place, and _block_ when the pipeline is full — let the kernel pty queue throttle the child. Never accumulate output in an unbounded `Vec`.
2. **Bounded mailboxes (64) + bounded wait, never lock-held blocking send** between the parse thread and the wgpu renderer thread. On full mailbox: release the shared state lock, wake the renderer, retry (Ghostty `rendererMailboxWriter`, issue #9224). A full-queue stall is correct behavior; an unbounded queue or a lock-held send is a deadlock.
3. **Renderer two-phase snapshot + demand handoff**: hold the terminal lock only to copy the visible rows (Ghostty `beginUpdate`), do per-cell/styling work outside it (`endUpdate`); a `demand`/`yieldToDemand` counter keeps the renderer from being starved by a saturated parser (PR #13265). Unfair-mutex barging is the #1 renderer-freeze cause under AI-CLI streams.
4. **DEC 2026 watchdog + escape hatches**: parser sets the flag; a 1000 ms timer force-clears it and wakes the renderer (Ghostty `sync_reset_ms` / `resetSynchronizedOutput`); clear it on resize (Ghostty `Terminal.zig` resize); treat RIS/SoftReset as a flush escape (WezTerm mux:183); drain buffered output on EOF (WezTerm mux:240-246). Watch DECSTBM/status-line writes inside a sync block — corruption class ghostty #12685.
5. **Payload caps, log + discard, never panic/OOM**: cap OSC payloads in `crates/zeroterm-core` (WezTerm vtparse is _unbounded_ under std — do not copy that gap); sixel ≤ 100 MB with `check_image_dimensions` (CVE-2022-24130); kitty total ≤ 320 MB with eviction + APC `max_bytes` (Ghostty graphics_storage/graphics_command); image cache = 16-entry LRU. Oversized → `log::warn` + drop.
6. **Fail to defaults, clamp config, log-don't-panic in threads**: config builds from defaults with clamped fields (Ghostty `Config.zig`; WezTerm `ConfigInner.error` + `default_config()`); scrollback capped (`MAX_SCROLLBACK_LINES`); worker-thread errors are `log::warn`, not panics (Ghostty renderer threadMain/drawFrame).

# Stability research — kitty (GPL) + foot (MIT)

Source: kitty master `999cde66db` (github.com/kovidgoyal/kitty), foot master `8db88cceb7` (codeberg.org/dnkl/foot). kitty is GPLv3 → design-ideas only, NO code copying. foot is MIT → code usable.

## kitty (GPL — design-ideas only) + foot (MIT — copyable)

License split:

- **kitty is GPLv3** → ZeroTerm (MIT) may copy NONE of kitty's code or
  structure, only study the design/techniques. Anything below under kitty is a
  _design idea_, re-implement independently.
- **foot is MIT** → foot's code is usable in ZeroTerm with attribution.
  Anything below under foot is _copyable_.

Note on the research brief: `kitty/io.py` and `kitty/graphics.py` no longer
exist on master. I/O moved into the C extension `kitty/child-monitor.c`;
graphics protocol handling is `kitty/graphics.c` (C, not Python). There is no
`render_program`/`borders` ring-buffer-of-screen-updates system either — kitty's
render pipeline is the main-thread GLFW loop + a per-Screen `is_dirty`/dirty-line
system. Findings below describe what the code actually does.

---

### kitty — design principles only

Kitty is a 2-thread terminal: a **main thread** (GLFW window events, Python
logic, render) and a **dedicated I/O thread** (`io_loop` in
`kitty/child-monitor.c`) that owns all child PTY fds and feeds bytes to the VT
parser. All hot paths are C inside the `fast_data_types` extension.

**1. Non-blocking PTY reads into a 1 MB ring buffer, with _natural_
backpressure.** `io_loop` runs `poll()` over `[wakeup_fd, signalfd, ...child_ptys]`
(`child-monitor.c:1663-1669`). For each child it sets `POLLIN` **only if**
`vt_parser_has_space_for_input(screen->vt_parser)` is true; the parser's read
buffer is a fixed 1 MiB ring (`BUF_SZ = 1024*1024`, `vt-parser.c:18`). When the
ring fills, POLLIN is disabled → the kernel PTY buffer absorbs output → the
child's `write()` blocks. That is the entire backpressure story: **stop reading
faster than you parse.** Reads use `read()` on a non-blocking fd, retrying only
`EINTR`/`EAGAIN`, aborting on `EIO` (PTY closed) (`child-monitor.c:1500-1520`).
The I/O thread never renders; the main thread parses.

**2. Input batching (coalescing) with a hard flush bound.** The main thread's
`run_worker` (`vt-parser.c:1519-1549`) parses only when one of three conditions
holds: `flush` requested, `input_delay` elapsed since first unread byte, or the
ring is nearly full (`self->read.sz + 16*1024 > BUF_SZ`). `input_delay` default
3 ms (`kitty/options/definition.py`). Effect: fast output is batched into
larger parse runs, but the near-full condition guarantees the producer can never
stall the consumer into a livelock. The parser is a VTE-style byte state machine
(`vt-parser.c`), _not_ per-escape-code — a partial escape sequence mid-buffer is
handled by retaining unconsumed bytes (`self->read.pos -= consumed`).

**3. Bounded write queue to the child (the "rate limiter").** Keyboard/mouse
input → `schedule_write_to_child` (`child-monitor.c:324-378`) appends to a
per-Screen growable `write_buf`, capped at **100 MB**
(`write_buf_limit = 100*1024*1024`, `child-monitor.c:317`). Over the cap: data is
dropped with `log_error("Too much data being sent to child ... ignoring it")`.
The I/O thread registers `POLLOUT` only when `write_buf_used > 0` and drains it
with a `write()` loop that breaks on `EWOULDBLOCK`/`EAGAIN` (`child-monitor.c`),
and shrinks the buffer back to `BUFSIZ` once drained. The child-fd write of the
initial payload (`thread_write`) is a separate detached thread that clears
`O_NONBLOCK` and loops until all bytes go out.

**4. Render throttling + dirty tracking.** `repaint_delay` default 10 ms
(≈100 FPS ceiling, `options/definition.py`); `render()` in
`child-monitor.c` early-returns if `time_since_last_render < repaint_delay`
unless input was read. The Screen sets `is_dirty` on any cell/line change
(`screen.c`) and a per-line dirty flag in the `linebuf`; only dirty lines are
re-uploaded to GPU. This is why kitty survives `yes`-style floods: parse is
decoupled from paint.

**5. Synchronized output (DECSET 2026) with a timeout.** `screen_pause_rendering`
(`screen.c:3442-3490`): while paused, all cell mutations go to a **snapshot**
(separate `grman` sprite manager + `linebuf` + cursor/selection copy) so the
renderer keeps showing the last committed frame. A monotonic deadline
`expires_at` is armed for **2000 ms by default** (`for_in_ms <= 0 → 2000`,
`screen.c:3460`); `screen_check_pause_rendering` is polled on every parse and
auto-resumes if the app never sends the end sequence — **a buggy client cannot
freeze the UI.** On resume the snapshot is freed and the screen marked dirty.

**6. Image/graphics memory management (anti-leak).** `kitty/graphics.c`:

- In-progress upload buffer capped at **400 MB** (`MAX_DATA_SZ = 4*100000000`).
- Per-image dimension cap **10000×10000** (`MAX_IMAGE_DIMENSION`, `graphics.c:26`);
  oversized images are rejected with `EINVAL` "Image too large".
- Global storage quota **320 MB** default (`DEFAULT_STORAGE_LIMIT`, `graphics.c:27`).
- `apply_storage_quota` (`graphics.c:281-299`): when `used_storage > storage_limit`,
  first frees unreferenced images (`trim_predicate`), then sorts survivors by
  `transient_or_older_first` and evicts until under quota — **transient images
  first, then least-recently-used by `atime`.**
- A separate on-disk cache (for the `disk-cache.c`) is bounded at **5× the
  storage limit** (`graphics.c:1602`). Deleting scrolled-out images is
  integrated with the history buffer (images are removed when their rows leave
  scrollback, not on a timer).

**7. Parser robustness / "never trust the client".** Escape/OSC/DCS strings are
accumulated with a cap `MAX_ESCAPE_CODE_LENGTH = BUF_SZ/4 = 256 KB`
(`vt-parser.c:21`); longer sequences are dropped with a `REPORT_ERROR` and the
parser returns to a sane state (`vt-parser.c:427-442`). CSI params capped at
`MAX_CSI_PARAMS = 256` (`vt-parser.c:22`). Oversized OSC 52 (clipboard) payloads
are handled specially: chunked into 256 KB pieces via `continue_osc_52` so huge
clipboard writes work without unbounded memory. Malformed sequences log and are
ignored (e.g. malformed OSC 8 hyperlinks, `vt-parser.c:472`). **Actionable: cap
every string-accumulating parser buffer; on overrun, discard and return to a
well-defined state — never grow without bound, never keep partial state that
confuses later input.**

**8. Signal handling.** `SIGCHLD`, `SIGTERM`, `SIGHUP`, `SIGINT` are blocked and
delivered via a **signalfd** on the I/O thread (`loop-utils.c`,
`child-monitor.c:121-147`); the main loop never handles signals synchronously.
`reap_children` runs a `waitpid(-1, WNOHANG)` loop so all children are reaped
without blocking. `close_on_child_death` option lets a child exit close its
window. Resize sends SIGWINCH (via `TIOCSWINSZ` + `notify_child_of_resize`); an
interactive (drag) resize **pauses resize notifications to the child**
(`window.py pause_resize_notifications_to_child`) to avoid a resize storm.

**Informational:** the "never trust the client" principle is pervasive — every
allocation site assumes adversarial input, every growable structure has a hard
ceiling, and dropping malformed/oversized input is the default response.

---

### foot — MIT code usable

Foot is a **single-threaded** terminal: one `epoll` loop (`fdm.c`) drives PTY
reads, timers, Wayland, and render. Simpler than kitty's thread split; stability
comes from fixed-size structures and cooperative batching. Copyable into
ZeroTerm (MIT, attribution).

**1. Event loop + signals (`fdm.c`).** `epoll_pwait` with a signal mask
(`fdm.c:447-448`); signals (incl. `SIGCHLD`) are caught into a
`received_signals[]` bitmask and dispatched to registered handlers between
epoll rounds. Hooks run in three priority bands (high/normal/low) before each
`epoll_pwait` (`fdm.c:423-443`) — the render scheduler is a normal-priority hook.
`reaper.c:90`: SIGCHLD handler runs `waitpid(-1, WNOHANG)` in a loop.
**Actionable: a single non-blocking event loop with signal-masked `epoll_pwait`
and a WNOHANG reaper is all that's needed; no threads required.**

**2. VT parser (`vt.c`).** Paul Williams' DEC state machine, one `switch(state)`
per byte with **fixed-size state** (no allocations in the hot path). Parameter
storage is fixed: `struct vt_param v[16]` in `terminal.h`; overflow is silently
diverted to a `dummy` slot with a one-time warning (`vt.c:325-336`) — the parser
**never allocates** for parameter lists and never grows them. Sub-parameters
(`:`) have the same fixed-array + dummy pattern (`vt.c:350-372`). `vt_param_get`
clamps indices/values. UTF-8 is decoded inline via dedicated states
(`STATE_UTF8_21/31/32/41/42/43`); an invalid sequence inserts `U+FFFD`
REPLACEMENT CHARACTER and resyncs (vt.c commit log: "insert a REPLACEMENT
CHARACTER when an invalid UTF-8 sequence is detected"). Unknown
intermediates/finals land in `STATE_CSI_IGNORE`/`STATE_DCS_IGNORE` and are
discarded (`vt.c` state tables). **Actionable: fixed-size param array + overflow
sink; U+FFFD on bad UTF-8; explicit IGNORE states.**

**3. OSC buffer (`osc.c`).** `osc_ensure_size` (`osc.c:1707-1735`) grows the
OSC string buffer by powers of two from a 4096 floor; on allocation failure it
returns false and the OSC string is **dropped** (no partial state). No hard byte
cap — memory failure is the cap. Compare kitty's 256 KB hard cap; for ZeroTerm,
prefer kitty's explicit cap over foot's unbounded growth.

**4. PTY read path (`terminal.c fdm_ptmx`).** Reads into a stack buffer of
**24 KB**, at most **10 iterations per event** (`terminal.c:273-276`), treating
`EAGAIN` (no more data) and `EIO` (PTY closed) as normal exits, and letting a
later `EPOLLIN` re-trigger. While an **interactive resize** is in progress the
PTY is _not_ consumed at all (`terminal.c:263-271`, plus explicit
`term_ptmx_pause`/`term_ptmx_resume`, `terminal.c:392-405`) because the normal
grid is a temporary copy during reflow. **Actionable: cap bytes per wakeup so a
single flood cannot monopolize the loop; pause reads during resize.**

**5. Write path to child (`terminal.c`).** Try a synchronous `write()` first; on
`EAGAIN`, register `EPOLLOUT` and queue the remainder in an order-preserving
linked list (`ptmx_buffers`), draining on `EPOLLOUT` (`data_to_slave`,
`fdm_ptmx_out`, `terminal.c:64-180`). Paste data goes through a **separate
queue** (`ptmx_paste_buffers`) so paste never interleaves with keystrokes
(issue #101). No hard cap on the queue — bounded only by `xmemdup`; for ZeroTerm
add kitty's drop-on-overflow cap here.

**6. Render throttling — dual-timer frame scheduling.** After parsing, foot does
**not** render immediately; it arms a `timerfd` (`delayed_render_lower_ns`,
default **0.5 ms**) that is _re-armed on every new input_, plus a second timer
(`delayed_render_upper_ns`, default **8.3 ms ≈ half a frame**) that is _only
reset when a frame is actually rendered_ (`terminal.c:300-365`). This is the
anti-flicker, anti-starvation trick: fast writers keep getting coalesced, but
the render is guaranteed to happen at least every ~8 ms. Tunables in `config.c`
(`.delayed_render_lower_ns = 500000`, `.delayed_render_upper_ns = 16666666/2`).
**Actionable: two timers — one reset per input, one reset per frame — so
batching can't starve painting.** Title/icon/app-id OSC updates are throttled to
~8.3 ms each via their own timerfds (`terminal.c fdm_title_update_timeout`).

**7. Synchronized output (DECSET 2026).** `term_enable_app_sync_updates`
(`terminal.c:3832-3861`) suppresses grid refreshes, disarms the delayed-render
timers, and arms a **1 second** `timerfd`; on expiry `term_disable_app_sync_updates`
forces a refresh (`fdm_app_sync_updates_timeout`, `terminal.c:612-632`). CSD and
search overlays are _never_ suppressed. Same lesson as kitty: **hard timeout,
never trust the app to send `ESC[?2026l`.**

**8. Damage tracking / dirty-rect rendering (`render.c`).** Two-tier dirtiness:
per-cell `attrs.clean` and per-row `row->dirty`. `grid_render` iterates rows,
skips clean rows, and chunks consecutive dirty rows to minimize compositing
(`render.c:1576-1592`). Damage is accumulated in a `pixman_region32_t` and
translated to `wl_surface_damage_buffer` regions. Two **scroll damage
algorithms** (foot wiki: "two algorithms for scroll damage"): when a large
scroll happens foot copies the existing framebuffer with damage regions instead
of re-compositing everything; for small scrolls it re-renders. Double-buffering
reapplies last frame's damage (`reapply_old_damage`, `render.c`) when the
compositor is slow to release buffers, avoiding flicker. Cell compositing is
farmed to a **worker pool** (`render.workers`, default `sysconf(_SC_NPROCESSORS_ONLN)`
threads, `render.c:2205-2239`, `config.c:3631`) via start/done semaphores.
**Actionable: cell `clean` bits + row dirty flags + region-based damage; worker
pool only if composition is CPU-bound (foot does CPU compositing; a GPU
renderer like ZeroTerm may not need threads).**

**9. Scrollback memory (`grid.c`).** The grid is a **ring buffer of rows sized
to a power of two** (`num_rows = rows + scrollback_lines`, indexing via
`& (num_rows - 1)`), so every row is a fixed-size allocation and scrollback
memory is _inherently_ bounded by the configured line count — default
**1000 lines** (`config.c:3575`). Rows are freed as the ring wraps; sixels
attached to rows are destroyed with them (`sixel_scroll_up` erases images
scrolled out, `sixel.c`). On resize, `grid_resize_and_reflow` re-wraps wrapped
lines into the new width. **Actionable: fixed ring of rows with line-count cap
beats an unbounded `VecDeque` of cells; ZeroTerm should do the same and drop
sixel/image objects when their row is evicted.**

**10. Sixel memory limits (`sixel.h`, `sixel.c`).** Hard constants:
`SIXEL_MAX_COLORS = 1024`, `SIXEL_MAX_WIDTH = 10000`, `SIXEL_MAX_HEIGHT = 10000`
(`sixel.h:5-7`). The decoder hard-crops at those bounds
(`sixel.c:1434,1498,1552-1559`); palettes and images are freed by
`sixel_fini`/`sixel_destroy_all`; the live-image list is sorted and
invariant-checked under `_DEBUG` (`verify_sixels`). No per-image bytes quota like
kitty — foot relies on dimension caps + scrollback eviction. For ZeroTerm,
kitty's byte-quota design is the stronger one (foot's 10000² ≈ 400 MB ARGB image
can still be large).

---

### Cross-cutting stability rules (both terminals, in ZeroTerm terms)

1. **Never trust the client.** Every parse path assumes adversarial/hostile
   bytes. Malformed input → log + drop, never panic/abort/OOM.
2. **Cap every growable buffer.** Params (fixed array + overflow sink), escape
   codes, OSC/DCS strings, image dimensions, image byte quota, write queue.
   Overflow = discard, and the discard must not corrupt parser state.
3. **Non-blocking PTY reads; read only as fast as you parse.** Stop reading
   (disable POLLIN/EPOLLIN) when the parse ring is full — the kernel PTY buffer
   then blocks the child. This is the whole backpressure story.
4. **Bound the write path too.** Queue writes to the child with a byte cap;
   drop + log beyond it (kitty: 100 MB; foot: unbounded, prefer kitty's rule).
   Handle `EAGAIN` by registering `POLLOUT`, never spinning.
5. **Batch, but with a hard deadline.** Coalesce input before parsing/painting
   (`input_delay`/delayed-render), but force flush on near-full buffer (kitty)
   or on a frame-reset timer (foot), so the UI can never starve.
6. **Dirty-rect rendering.** Per-cell clean bits + per-row dirty flags +
   region damage. Never repaint the whole grid; never upload clean cells.
7. **Synchronized output must time out** (kitty 2000 ms, foot 1000 ms). A hung
   or malicious app that never sends `ESC[?2026l` must not freeze rendering.
8. **Signals via signalfd/self-pipe + `waitpid(-1, WNOHANG)` reaper loop.**
   Never do a blocking wait in the render loop; reap until ECHILD.
9. **Resize resilience.** Coalesce/rate-limit SIGWINCH; pause PTY consumption
   (foot) or pause resize notifications to the child (kitty) during interactive
   drag-resize; reflow must be allocation-bounded.
10. **Fixed-size hot path.** Keep the byte-parser allocation-free
    (foot's approach); all growth happens in cold paths (OSC/DCS/image).

_Flags: items 1–9 are actionable for ZeroTerm; 10 is informational/aspirational._
