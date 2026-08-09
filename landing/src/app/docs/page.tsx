const REPO_DOCS = 'https://github.com/mahesh-diwan/ZeroTerm/tree/main/docs';

const sections = [
  {
    id: 'quickstart',
    title: 'Quick Start',
    items: [
      {
        label: 'Installation',
        content: 'ZeroTerm is not yet published to crates.io, Homebrew, or Flathub, so the supported paths are the install script and building from source:\n\n```bash\n# Linux/macOS — resolves the latest release tag, downloads a prebuilt\n# binary when one exists, otherwise builds from source at that tag\ncurl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | sh\n\n# Build from source\n# Requires Rust stable: https://rustup.rs\ngit clone https://github.com/mahesh-diwan/ZeroTerm.git\ncd ZeroTerm\ncargo run --release\n```\n\nPrebuilt binaries (AppImage, macOS zip, Windows zip, .deb, .rpm) are attached to each GitHub Release once the release pipeline publishes them. On first launch, ZeroTerm writes a default config to `~/.config/zeroterm/config.toml`.',
      },
      {
        label: 'First Run',
        content: 'Run `zeroterm` from your terminal — it opens as a standalone window. Sessions persist across restarts: close the window and reopen, and your tabs are restored.\n\nLaunch it in a shell, start the demo, or connect to a remote host with `Ctrl+Shift+S` (uses the host in `config.ssh`, or opens a host picker).',
      },
      {
        label: 'Keyboard Shortcuts',
        content: '| Shortcut | Action |\n|---|---|\n| `Ctrl+Shift+T` | New tab |\n| `Ctrl+Shift+W` | Close active tab |\n| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |\n| `Alt+1` … `Alt+9` | Switch to tab 1-9 |\n| `Ctrl+Shift+E` | Split pane vertically |\n| `Ctrl+Shift+D` | Split pane horizontally |\n| `Ctrl+Shift+G` | Float active pane (overlay) |\n| `Alt+Arrow` | Focus adjacent pane |\n| `Ctrl+Shift+F` | Toggle search overlay |\n| `Ctrl+Shift+J` / `K` | Jump to next / previous output block |\n| `Ctrl+Shift+C` / `V` | Copy selection / paste |\n| `Ctrl+Shift+O` | Cycle window opacity |\n| `F12` | Toggle quake (drop-down) mode |\n| `Shift+PageUp/Down` | Scroll back / forward |\n| `Shift+Home` / `Shift+End` | Jump to oldest / newest scrollback |\n\nA plain `PageUp`/`PageDown`/`Home`/`End` is forwarded to the shell (for `less`, `vim`, etc.); only the `Shift` variants scroll the scrollback.',
      },
      {
        label: 'SSH',
        content: 'SSH is a built-in feature (Unix): configure a host under `[ssh]` in `config.toml` and press `Ctrl+Shift+S` to connect. Sessions are persistent — disconnect without killing the remote work, and reconnect later. See the user guide for details.',
      },
    ],
  },
  {
    id: 'configuration',
    title: 'Configuration',
    items: [
      {
        label: 'Config File Locations',
        content: 'ZeroTerm reads `~/.config/zeroterm/config.toml` for core settings and an optional `~/.zeroterm.lua` Lua script for advanced customization.\n\n| Path | Purpose |\n|---|---|\n| `~/.config/zeroterm/config.toml` | Core settings (window, fonts, colors, keybindings, SSH, sync) |\n| `~/.zeroterm.lua` | Optional Lua scripting |\n\nSee the [config reference](https://github.com/mahesh-diwan/ZeroTerm/blob/main/docs/CONFIG_REFERENCE.md) for every setting and its default.',
      },
      {
        label: 'Documentation',
        content: 'The full, current documentation lives in the repository:\n\n- [User guide](https://github.com/mahesh-diwan/ZeroTerm/blob/main/docs/USER_GUIDE.md) — shortcuts, selection, search, blocks, images\n- [Config reference](https://github.com/mahesh-diwan/ZeroTerm/blob/main/docs/CONFIG_REFERENCE.md) — every TOML key and Lua hook\n- [Plugin development guide](https://github.com/mahesh-diwan/ZeroTerm/blob/main/docs/PLUGIN_DEV_GUIDE.md) — writing WASM plugins\n- [Architecture](https://github.com/mahesh-diwan/ZeroTerm/blob/main/docs/ARCHITECTURE.md) — how the crates fit together',
      },
    ],
  },
  {
    id: 'features',
    title: 'Features',
    items: [
      {
        label: 'What is implemented',
        content: '- **Tabs & split panes** — tiling splits, floating-pane overlay, session restore\n- **Scrollback** — search overlay, output-block navigation, syntax highlighting\n- **Graphics protocols** — Kitty, Sixel, and iTerm2 inline images\n- **SSH** — native client with persistent sessions (Unix)\n- **Encrypted sync** — ChaCha20-Poly1305 settings sync (`sync` feature)\n- **Plugins** — WASM sandbox via wasmtime (`plugins` feature)\n- **Line editor** — readline-style multi-line editing with history\n\nAll claims on this site correspond to code in the repository; features are feature-gated in `crates/zeroterm/Cargo.toml`.',
      },
    ],
  },
];

function DocSection({ section }: { section: typeof sections[number] }) {
  return (
    <div id={section.id} className="mb-16">
      <h2 className="text-3xl font-bold mb-8">{section.title}</h2>
      <div className="space-y-8">
        {section.items.map((item) => (
          <div key={item.label} className="bg-[var(--card)] border border-[var(--border)] rounded-xl p-6">
            <h3 className="text-xl font-semibold mb-4">{item.label}</h3>
            <div className="text-sm text-[var(--fg-muted)] leading-relaxed prose-invert max-w-none [&_code]:text-[var(--accent)] [&_code]:text-xs [&_code]:bg-[var(--bg-elevated)] [&_code]:px-1.5 [&_code]:py-0.5 [&_code]:rounded [&_pre]:bg-[var(--bg)] [&_pre]:border [&_pre]:border-[var(--border)] [&_pre]:rounded-xl [&_pre]:p-4 [&_pre]:overflow-x-auto [&_pre]:my-4 [&_pre]:text-[var(--fg)] [&_table]:w-full [&_table]:text-sm [&_th]:text-left [&_th]:p-2 [&_th]:border-b [&_th]:border-[var(--border)] [&_th]:text-[var(--fg)] [&_td]:p-2 [&_td]:border-b [&_td]:border-[var(--border)] [&_td]:text-[var(--fg-muted)]">
              <div dangerouslySetInnerHTML={{ __html: item.content
                .replace(/\n/g, '<br/>')
                .replace(/```(\w+)?\n([\s\S]*?)```/g, '<pre><code>$2</code></pre>')
                .replace(/\|(.+)\|/g, (m) => { if (m.includes('---')) return ''; return m; })
                .replace(/\[([^\]]+)\]\((https?:\/\/[^)]+)\)/g, '<a href="$2" class="text-[var(--accent)] hover:underline">$1</a>') }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export default function DocsPage() {
  const toc = [
    { href: '#quickstart', label: 'Quick Start' },
    { href: '#configuration', label: 'Configuration' },
    { href: '#features', label: 'Features' },
  ];

  return (
    <div className="min-h-screen">
      <div className="max-w-6xl mx-auto px-6 py-16 flex gap-12">
        <nav className="hidden lg:block w-56 shrink-0">
          <div className="sticky top-24 space-y-2">
            <h3 className="text-sm font-semibold text-[var(--fg-muted)] uppercase tracking-wider mb-4">Documentation</h3>
            {toc.map((t) => (
              <a key={t.href} href={t.href} className="block text-sm text-[var(--fg-muted)] hover:text-[var(--accent)] transition-colors py-1">
                {t.label}
              </a>
            ))}
          </div>
        </nav>
        <div className="flex-1 min-w-0">
          <h1 className="text-4xl md:text-5xl font-bold mb-4">Documentation</h1>
          <p className="text-lg text-[var(--fg-muted)] mb-16">
            Everything you need to get started with ZeroTerm.
          </p>
          {sections.map((s) => <DocSection key={s.id} section={s} />)}
        </div>
      </div>
    </div>
  );
}
