# ZeroTerm

A GPU-accelerated terminal emulator built in Rust.

## Features

- GPU-accelerated rendering via wgpu (Metal/DX12/Vulkan)
- Fast VT100/ANSI parser
- Dynamic glyph atlas with font metrics
- Tab management (multiple terminal sessions)
- Local AI integration (Ollama)
- Cross-platform (macOS, Linux, Windows)

## Quick Start

```bash
git clone https://github.com/zeroterm/zeroterm.git
cd zeroterm
cargo run --release
```

## Configuration

Config file: `~/.config/zeroterm/config.toml`

See [docs/configuration.md](docs/configuration.md) for full reference.

## Project Structure

- `crates/zeroterm-core` — VT100 parser, screen buffer, cell model
- `crates/zeroterm-render` — wgpu GPU renderer, glyph atlas
- `crates/zeroterm-mux` — Tab/split/session management
- `crates/zeroterm-config` — TOML + Lua config
- `crates/zeroterm-ai` — Local AI integration (Ollama/LM Studio)
- `crates/zeroterm-sync` — E2E encrypted settings sync
- `crates/zeroterm` — Main binary
