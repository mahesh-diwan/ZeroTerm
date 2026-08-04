# ZeroTerm Plugin Development Guide

ZeroTerm plugins are **sandboxed WASM commands** run through the
`zeroterm-plugin` crate (wasmtime + WASI). A plugin is an ordinary
`wasm32-wasip1` command module: it imports `wasi_snapshot_preview1`, exports
`_start`, reads its input from **stdin**, and writes its result to **stdout**.
There is no linear-memory pointer ABI and no host function table — WASI stdio is
the entire surface a plugin can touch.

## Sandbox guarantees

| Guarantee                | Detail                                                                                                                  |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| **Memory**               | Guest linear memory capped at `max_memory` (default **16 MiB**) via a wasmtime `ResourceLimiter`.                       |
| **CPU**                  | Every call runs under a fixed fuel budget (`DEFAULT_FUEL`, 50M instructions); infinite loops are trapped by the engine. |
| **Filesystem**           | No preopens by default — zero FS access. Setting `wasi_dir` preopens one directory read-only at `/`.                    |
| **Output**               | Stdout capped at `max_output` (default **1 MiB**); extra bytes are dropped.                                             |
| **Network / env / argv** | None. Plugins get no network, no environment variables, and no host arguments.                                          |

## Install the target

```sh
rustup target add wasm32-wasip1
```

## A minimal Rust plugin

`src/main.rs` of your plugin crate:

```rust
use std::io::{Read, Write};

fn main() {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).unwrap();
    // transform `buf` however you like...
    std::io::stdout().write_all(&buf).unwrap();
}
```

Everything your plugin produces must go to stdout; anything from stdin is the
host's payload. Because each invocation runs in a fresh WASI `Store`, plugins
are **stateless between calls** — no guest state survives one call to the next.

## Build

```sh
cd your-plugin
cargo build --target wasm32-wasip1 --release
# produces target/wasm32-wasip1/release/your-plugin.wasm
```

## Install & run in ZeroTerm

1. Create the plugins directory and copy the `.wasm` there:

   ```sh
   mkdir -p ~/.config/zeroterm/plugins
   cp target/wasm32-wasip1/release/your-plugin.wasm ~/.config/zeroterm/plugins/
   ```

   (The directory is `<config_dir>/plugins`, i.e. `~/.config/zeroterm/plugins`.)
   Every `.wasm` file there is loaded at startup.

2. Press **`Ctrl+Shift+B`** to run a loaded plugin. It reads the current call
   input and its stdout is written back into the terminal.

> Plugin loading is behind the `plugins` cargo feature of the `zeroterm`
> binary. If no plugins are loaded, `Ctrl+Shift+B` prints a notice.

## How the host calls plugins

The plugin runtime lives in `crates/zeroterm-plugin` (`Plugin` / `PluginHost`):

- `PluginHost::new()` builds the shared wasmtime `Engine` (fuel consumption on).
- `PluginHost::load(path, config)` compiles one `.wasm` once into a `Plugin`.
- `Plugin::call(input: &[u8]) -> Result<Vec<u8>>` runs it: `input` goes to the
  plugin's WASI stdin, and the plugin's stdout is captured and returned. Each
  call creates a fresh `Store` with its own WASI context and fuel allocation.

Failure to load reports a `PluginError::Load`; a trap, abort, or nonzero
`proc_exit` reports `PluginError::Terminated`.

### `PluginConfig` knobs

| Field        | Type              | Default            | Meaning                                                   |
| ------------ | ----------------- | ------------------ | --------------------------------------------------------- |
| `name`       | `String`          | `"plugin"`         | Human-readable name, used in errors/logs.                 |
| `max_memory` | `usize`           | `16 * 1024 * 1024` | Max guest linear memory (bytes).                          |
| `max_output` | `usize`           | `1024 * 1024`      | Max stdout bytes per call.                                |
| `wasi_dir`   | `Option<PathBuf>` | `None`             | Directory preopened **read-only** at `/`; `None` = no FS. |

The ZeroTerm app itself loads plugins with just `name` set (all other knobs at
their defaults). If you load plugins programmatically you can raise/lower the
bounds per plugin.
