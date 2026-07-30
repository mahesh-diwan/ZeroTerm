import Link from 'next/link';

const sections = [
  {
    id: 'quickstart',
    title: 'Quick Start',
    items: [
      { label: 'Installation', content: 'Install via your preferred package manager:\n\n```bash\n# Homebrew (macOS)\nbrew install zeroterm\n\n# Cargo\ncargo install zeroterm\n\n# Linux binary\ncurl -LsSf https://github.com/mahesh-diwan/ZeroTerm/releases/latest/download/zeroterm-installer.sh | sh\n```\n\nOn first launch, ZeroTerm creates default config at `~/.config/zeroterm/config.toml`.' },
      { label: 'First Run', content: 'Run `zeroterm` from your terminal. ZeroTerm opens as a standalone window. Create a new tab with `Cmd+T` (macOS) or `Ctrl+T` (Linux/Windows).\n\nSessions persist across restarts. Close the window and reopen — your tabs are restored.' },
      { label: 'Keyboard Shortcuts', content: '| Shortcut | Action |\n|---|---|\n| `Cmd/Ctrl + T` | New tab |\n| `Cmd/Ctrl + W` | Close tab |\n| `Cmd/Ctrl + D` | Split right |\n| `Cmd/Ctrl + Shift + D` | Split down |\n| `Cmd/Ctrl + [` | Previous tab |\n| `Cmd/Ctrl + ]` | Next tab |\n| `Cmd/Ctrl + Shift + Enter` | Fullscreen |\n| `Cmd/Ctrl + K` | Clear scrollback |\n| `Cmd/Ctrl + Shift + F` | Search |\n| `Ctrl + L` | Toggle AI assistant |' },
      { label: 'SSH Connections', content: '```bash\nzeroterm ssh user@hostname\n```\n\nSSH sessions are persistent — disconnect with `~.` and reconnect later. Sessions survive network interruptions.' },
    ],
  },
  {
    id: 'configuration',
    title: 'Configuration',
    items: [
      { label: 'Config File Locations', content: '| Platform | Path |\n|---|---|\n| Linux | `~/.config/zeroterm/config.toml` |\n| macOS | `~/Library/Application Support/zeroterm/config.toml` |\n| Windows | `%APPDATA%/zeroterm/config.toml` |\n| Portable | `./zeroterm.toml` (same directory as binary) |' },
      { label: 'Available Settings', content: '```toml\n[terminal]\nfont_size = 14\nfont_family = "JetBrains Mono"\nopacity = 0.95\ncursor_style = "block"  # block | underline | bar\ncursor_blink = false\nscrollback_lines = 10000\n\n[theme]\nbackground = "#0a0a0f"\nforeground = "#e8e8ed"\nblack = "#222233"\nred = "#ff3333"\ngreen = "#00d4aa"\nyellow = "#ffaa00"\nblue = "#5599ff"\nmagenta = "#ff66aa"\ncyan = "#44ddff"\nwhite = "#cccccc"\n\n[multiplexer]\nhistory_size = 100\nrestore_sessions = true\n\n[performance]\ngpu_vsync = true\nmax_fps = 120\n\n[ai]\nenabled = false\nmodel = "llama3.2"\nprovider = "ollama"  # ollama | lm_studio\nendpoint = "http://localhost:11434"\n```' },
      { label: 'Lua Scripting', content: 'Create `~/.config/zeroterm/init.lua` to customize behavior:\n\n```lua\n-- Custom keybinding\nzeroterm.bind("Ctrl+Shift+N", function()\n  zeroterm.new_tab()\n  zeroterm.run("htop")\nend)\n\n-- Auto-execute on tab creation\nzeroterm.on("tab_created", function(tab)\n  if tab.name == "dev" then\n    zeroterm.run("cd ~/projects && ls")\n  end\nend)\n\n-- Custom prompt format\nzeroterm.on("prompt", function()\n  return os.getenv("USER") .. "@" .. os.getenv("HOSTNAME") .. " $ "\nend)\n```' },
    ],
  },
  {
    id: 'features',
    title: 'Features',
    items: [
      { label: 'Tabs & Splits', content: 'ZeroTerm supports native tab management and tiling split panes. Drag tabs to reorder. Split vertically or horizontally. Each pane independently runs a shell, SSH session, or custom command.\n\nSessions persist in the background — switching tabs preserves scrollback and running processes.' },
      { label: 'AI Integration', content: 'Connect to a local Ollama or LM Studio instance for AI-powered terminal assistance:\n\n- **Explain output**: Select command output and ask AI to explain it\n- **Suggest commands**: Describe what you want to do, get command suggestions\n- **Complete code**: AI suggests completions for multi-line commands\n- **Fix errors**: Paste error messages, get fix suggestions\n\nAll processing is local. No data leaves your machine.' },
      { label: 'Sync', content: 'Encrypted configuration sync across machines:\n\n```toml\n[sync]\nenable = true\nprovider = "manual"  # manual | s3 | webdav\nencryption_key = "your-base64-key"\n```\n\nSyncs: config.toml, init.lua, themes, session history, SSH known hosts.' },
      { label: 'Theming', content: 'Full 256-color and truecolor support. import themes from:\n\n- iTerm2 color schemes (`.itermcolors`)\n- Xresources\n- Windows Terminal themes (`.json`)\n- ZeroTerm native TOML format\n\nThemes live in `~/.config/zeroterm/themes/`.' },
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
            <div className="text-sm text-[var(--fg-muted)] leading-relaxed prose-invert max-w-none [&_code]:text-[var(--accent)] [&_code]:text-xs [&_code]:bg-[var(--bg-elevated)] [&_code]:px-1.5 [&_code]:py-0.5 [&_code]:rounded [&_pre]:bg-[var(--bg)] [&_pre]:border [&_pre]:border-[var(--border)] [&_pre]:rounded-xl [&_pre]:p-4 [&_pre]:overflow-x-auto [&_pre]:my-4 [&_table]:w-full [&_table]:text-sm [&_th]:text-left [&_th]:p-2 [&_th]:border-b [&_th]:border-[var(--border)] [&_th]:text-[var(--fg)] [&_td]:p-2 [&_td]:border-b [&_td]:border-[var(--border)] [&_td]:text-[var(--fg-muted)]">
              <div dangerouslySetInnerHTML={{ __html: item.content.replace(/\n/g, '<br/>').replace(/```(\w+)?\n([\s\S]*?)```/g, '<pre><code>$2</code></pre>').replace(/\|(.+)\|/g, (m) => { if (m.includes('---')) return ''; return m; }) }} />
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
