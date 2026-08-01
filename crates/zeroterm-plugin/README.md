# zeroterm-plugin

Sandboxed WASM plugin runtime for ZeroTerm, built on [wasmtime](https://wasmtime.dev) +
WASI (Phase 5 of the roadmap). Plugins are ordinary WASI commands compiled to
`wasm32-wasip1`; the host never exposes raw pointers or host functions to them.

## Plugin ABI

A plugin is a single WASI command module:

- imports `wasi_snapshot_preview1` (standard WASI)
- exports `_start` (the command entry point)
- reads its input from **stdin**, writes its result to **stdout**

The host feeds the call's input bytes to the plugin's WASI stdin and returns
whatever the plugin writes to WASI stdout. No linear-memory pointer ABI, no
`wasm-bindgen` — just stdio.

## Building a plugin

With `wasm32-wasip1` installed (`rustup target add wasm32-wasip1`):

```sh
cargo build --target wasm32-wasip1
```

A minimal Rust plugin:

```rust
fn main() {
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf).unwrap();
    // transform buf however you like, then write the result to stdout:
    std::io::stdout().write_all(&buf).unwrap();
}
```

## Sandbox guarantees

- **Memory**: guest linear memory capped at `PluginConfig::max_memory` (default 16 MiB)
  via a wasmtime `ResourceLimiter`.
- **CPU**: every call runs under a fixed fuel budget; infinite loops are trapped.
- **Filesystem**: no preopens by default (no FS access at all). Setting
  `PluginConfig::wasi_dir` preopens one directory read-only.
- **Output**: stdout is capped at `PluginConfig::max_output` (default 1 MiB).
- No network, no env vars, no host arguments.

## Hosting in ZeroTerm

`PluginHost` owns the shared wasmtime `Engine`; each plugin `.wasm` is loaded
once with `PluginHost::load(path, config)` and invoked repeatedly via
`Plugin::call(input)`. Every call runs in a fresh `Store`, so plugins are
stateless between calls — no state leaks across invocations.
