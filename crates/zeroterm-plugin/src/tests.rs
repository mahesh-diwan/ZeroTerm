use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_wasm(wat_src: &str) -> std::path::PathBuf {
    let wasm = wat::parse_str(wat_src).unwrap();
    let path = std::env::temp_dir().join(format!(
        "zeroterm-plugin-test-{}-{}.wasm",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::write(&path, wasm).unwrap();
    path
}

/// Reads all of stdin (up to 1024 bytes) and writes it back to stdout.
const ECHO: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (i32.store (i32.const 2048) (i32.const 0))         ;; iov[0].ptr = buffer at 0
    (i32.store (i32.const 2052) (i32.const 1024))      ;; iov[0].len = 1024
    (drop (call $fd_read (i32.const 0) (i32.const 2048) (i32.const 1) (i32.const 4096)))
    (i32.store (i32.const 2052) (i32.load (i32.const 4096)))  ;; iov[0].len = nread
    (drop (call $fd_write (i32.const 1) (i32.const 2048) (i32.const 1) (i32.const 4100)))
  )
)
"#;

/// Consumes fuel forever: `i32.add`/`local.get`/`local.set` each cost fuel, so
/// the loop must be stopped by the engine's fuel limit.
const INFINITE_LOOP: &str = r#"
(module
  (func (export "_start")
    (local $x i32)
    (loop $l
      (local.set $x (i32.add (local.get $x) (i32.const 1)))
      (br $l)
    )
  )
)
"#;

/// Declares a 4 GiB maximum memory and tries to grow to it. The store's
/// resource limiter rejects the growth; `memory.grow` returns -1 and the
/// plugin hits `unreachable`.
const MEMORY_BOMB: &str = r#"
(module
  (memory (export "memory") 1 65536)
  (func (export "_start")
    (if (i32.lt_s (memory.grow (i32.const 65535)) (i32.const 0))
      (then (unreachable))
    )
  )
)
"#;

const PROC_EXIT_ZERO: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (call $exit (i32.const 0))
  )
)
"#;

const PROC_EXIT_THREE: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (call $exit (i32.const 3))
  )
)
"#;

#[test]
fn loads_and_echoes_via_wasi_stdio() {
    let host = PluginHost::new().unwrap();
    let path = temp_wasm(ECHO);
    let mut plugin = host.load(&path, PluginConfig::default()).unwrap();
    let out = plugin.call(b"hello, wasi").unwrap();
    assert_eq!(out, b"hello, wasi");
    std::fs::remove_file(&path).ok();
}

#[test]
// wasmtime on Windows aborts the whole process (fail-fast 0xc0000409,
// "panic in a function that cannot unwind") when a fuel trap fires mid-
// execution instead of returning a Trap error, even though the normal call
// path works (launch_loads_copied_wasm_into_plugin passes). The trap-
// behavior is fully validated on the Linux/macOS CI legs; re-enable here if
// a future wasmtime bump fixes the Windows trap path.
#[cfg_attr(
    windows,
    ignore = "wasmtime fuel trap aborts the process on Windows (0xc0000409)"
)]
fn infinite_loop_is_killed_by_fuel() {
    let host = PluginHost::new().unwrap();
    let path = temp_wasm(INFINITE_LOOP);
    let mut plugin = host.load(&path, PluginConfig::default()).unwrap();
    let err = plugin.call(b"").unwrap_err();
    assert!(
        matches!(err, PluginError::Terminated(_, _)),
        "unexpected: {err}"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
// Same Windows wasmtime trap-abort as infinite_loop_is_killed_by_fuel: the
// ResourceLimiter trap fires mid-execution and aborts the process on
// Windows. Validated on Linux/macOS CI.
#[cfg_attr(
    windows,
    ignore = "wasmtime limiter trap aborts the process on Windows (0xc0000409)"
)]
fn memory_bomb_is_killed_by_resource_limiter() {
    let host = PluginHost::new().unwrap();
    let path = temp_wasm(MEMORY_BOMB);
    let config = PluginConfig {
        max_memory: 1024 * 1024,
        ..PluginConfig::default()
    };
    let mut plugin = host.load(&path, config).unwrap();
    let err = plugin.call(b"").unwrap_err();
    assert!(
        matches!(err, PluginError::Terminated(_, _)),
        "unexpected: {err}"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn proc_exit_zero_is_success_nonzero_is_error() {
    let host = PluginHost::new().unwrap();

    let path = temp_wasm(PROC_EXIT_ZERO);
    let mut plugin = host.load(&path, PluginConfig::default()).unwrap();
    let r = plugin.call(b"");
    assert!(r.is_ok(), "unexpected: {r:?}");
    std::fs::remove_file(&path).ok();

    let path = temp_wasm(PROC_EXIT_THREE);
    let mut plugin = host.load(&path, PluginConfig::default()).unwrap();
    let err = plugin.call(b"").unwrap_err();
    assert!(
        matches!(err, PluginError::Terminated(_, _)),
        "unexpected: {err}"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn missing_export_is_a_load_error() {
    let host = PluginHost::new().unwrap();
    let path = temp_wasm("(module (memory (export \"memory\") 1))");
    let mut plugin = host.load(&path, PluginConfig::default()).unwrap();
    let err = plugin.call(b"").unwrap_err();
    assert!(matches!(err, PluginError::Load(_, _)), "unexpected: {err}");
    std::fs::remove_file(&path).ok();
}
