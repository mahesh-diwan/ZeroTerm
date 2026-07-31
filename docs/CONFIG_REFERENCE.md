# ZeroTerm Config Reference

ZeroTerm loads two sources of configuration:

1. **`~/.config/zeroterm/config.toml`** — primary TOML file (path via
   `dirs::config_dir()`).
2. **`.zeroterm.lua`** — optional Lua overlay, evaluated at load and applied
   on top of the TOML values.

The file is watched at runtime; edits hot-reload into the running app.

> **Gotcha:** the TOML file is deserialized as a whole. Every non-optional key
> below is required — a missing key rejects the whole file and ZeroTerm falls
> back to built-in defaults. Start from the template below.

## TOML — Default Template

```toml
[font]
family = "Liberation Mono"   # optional, informational
size = 14.0                  # pt
line_height = 1.2
path = "/path/to/font.ttf"   # optional; overrides family lookup

[colors]
foreground = "#e0e0e0"
background = "#1e1e1e"

[shell]
program = "zsh"              # "cmd.exe" on Windows
args = ["-l"]                # [] on Windows

[window]
width = 1200
height = 800
opacity = 1.0                # 0.0–1.0

[ai]
endpoint = ""                # e.g. http://localhost:11434

[sync]
server_url = ""              # e.g. https://sync.example.com

[ssh]
host = ""
port = 22
user = "username"            # defaults to OS username
key_path = ""                # optional public key file
auto_connect = false
```

## Section Reference

### `[font]`

| Key           | Type    | Default | Notes                                   |
| ------------- | ------- | ------- | --------------------------------------- |
| `family`      | string? | `None`  | Optional; not currently used for lookup |
| `size`        | float   | `14.0`  | Font size in points                     |
| `line_height` | float   | `1.2`   | Multiplier for cell height              |
| `path`        | string? | `None`  | Path to a `.ttf`; overrides fallback    |

If `path` is unset, ZeroTerm tries system fonts (Liberation Mono, Geist Mono,
DejaVu Sans Mono) before the embedded DejaVu fallback. `family` is reserved.

### `[colors]`

| Key          | Type    | Default   | Notes                   |
| ------------ | ------- | --------- | ----------------------- |
| `foreground` | hex str | `#e0e0e0` | Default text color      |
| `background` | hex str | `#1e1e1e` | Clear color / window bg |

### `[shell]`

| Key       | Type     | Default         | Notes                   |
| --------- | -------- | --------------- | ----------------------- |
| `program` | string   | `zsh`/`cmd.exe` | Executable to spawn     |
| `args`    | string[] | `["-l"]`/`[]`   | Arguments (login shell) |

### `[window]`

| Key       | Type  | Default | Notes                          |
| --------- | ----- | ------- | ------------------------------ |
| `width`   | int   | `1200`  | Initial width (px)             |
| `height`  | int   | `800`   | Initial height (px)            |
| `opacity` | float | `1.0`   | 0.0–1.0; `Ctrl+Shift+O` cycles |

### `[ai]`

| Key        | Type   | Default | Notes                                                                     |
| ---------- | ------ | ------- | ------------------------------------------------------------------------- |
| `endpoint` | string | `""`    | Ollama/LM Studio base URL. Empty disables AI. Hardcoded model: `llama3.2` |

### `[sync]`

| Key          | Type   | Default | Notes                                                                           |
| ------------ | ------ | ------- | ------------------------------------------------------------------------------- |
| `server_url` | string | `""`    | Sync server base URL. Empty disables the daemon. Plain `http://` logs a warning |

Sync pushes/pulls the serialized config to `GET/POST /api/sync[/latest]`,
encrypted client-side with a random ChaCha20-Poly1305 key (new key per launch;
server never sees plaintext).

### `[ssh]`

| Key            | Type   | Default     | Notes                             |
| -------------- | ------ | ----------- | --------------------------------- |
| `host`         | string | `""`        | Target host; `Ctrl+Shift+S` opens |
| `port`         | int    | `22`        | TCP port                          |
| `user`         | string | OS username | Login user                        |
| `key_path`     | string | `""`        | Public key file; empty → agent    |
| `auto_connect` | bool   | `false`     | Reserved; not yet wired           |

Auth precedence: password (not currently passed), public key, then SSH agent.

## Lua Overlay — `.zeroterm.lua`

`set(key, value)` writes an override; keys accept a short or dotted form. Values
are strings; numeric and boolean keys are parsed at apply time. Run in a
sandboxed Lua 5.4 VM (`io`, `require`, `package`, `debug`, etc. are removed;
`os` exposes only `clock` and `time`).

```lua
set("font_size", 16)
set("colors.background", "#111111")
set("opacity", 0.85)
set("ssh_host", "prod.example.com")
set("ssh_auto_connect", "true")
```

Predefined globals:

| Global        | Default         | Notes                        |
| ------------- | --------------- | ---------------------------- |
| `font_size`   | `14.0`          | Mirrors `[font] size`        |
| `line_height` | `1.2`           | Mirrors `[font] line_height` |
| `opacity`     | `1.0`           | Mirrors `[window] opacity`   |
| `theme`       | `"tokyo-night"` | Reserved; informational      |

### Supported `set()` keys

| Short key          | Dotted key          | Applies to          | Parsed as |
| ------------------ | ------------------- | ------------------- | --------- |
| `font_family`      | `font.family`       | `font.family`       | string    |
| `font_size`        | `font.size`         | `font.size`         | float     |
| `line_height`      | `font.line_height`  | `font.line_height`  | float     |
| `font_path`        | `font.path`         | `font.path`         | string    |
| `foreground`       | `colors.foreground` | `colors.foreground` | string    |
| `background`       | `colors.background` | `colors.background` | string    |
| `shell`            | `shell.program`     | `shell.program`     | string    |
| `window_width`     | `window.width`      | `window.width`      | u32       |
| `window_height`    | `window.height`     | `window.height`     | u32       |
| `opacity`          | `window.opacity`    | `window.opacity`    | f64       |
| `ai_endpoint`      | `ai.endpoint`       | `ai.endpoint`       | string    |
| `ssh_host`         | `ssh.host`          | `ssh.host`          | string    |
| `ssh_user`         | `ssh.user`          | `ssh.user`          | string    |
| `ssh_port`         | `ssh.port`          | `ssh.port`          | u16       |
| `ssh_key_path`     | `ssh.key_path`      | `ssh.key_path`      | string    |
| `ssh_auto_connect` | `ssh.auto_connect`  | `ssh.auto_connect`  | bool      |

Unrecognized keys are silently ignored. Note `shell.args` and `sync.server_url`
are not exposed to Lua.
