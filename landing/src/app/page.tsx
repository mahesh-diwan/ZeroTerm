'use client';

import { useState, useEffect } from 'react';
import Link from 'next/link';

const features = [
  {
    icon: '⚡',
    title: 'Zero Latency',
    desc: 'GPU-accelerated rendering via wgpu. 120 FPS at 4K. Every keystroke renders in < 1ms.',
  },
  {
    icon: '🦀',
    title: 'Pure Rust',
    titleExtra: 'No Electron',
    desc: 'Written entirely in Rust. No JS runtime, no Electron overhead. < 50MB RAM at idle.',
  },
  {
    icon: '📦',
    title: 'Native Multiplexing',
    desc: 'Tabs, splits, sessions — built-in. No tmux, no screen, no config required.',
  },
  {
    icon: '🔒',
    title: 'SSH First',
    desc: 'Native SSH client (thrussh). Persistent sessions. Disconnect without killing remote work.',
  },
  {
    icon: '🎨',
    title: 'Graphics Protocols',
    desc: 'Kitty, Sixel, iTerm2 inline images. Render images, plots, GIFs directly in terminal.',
  },
  {
    icon: '🤖',
    title: 'Local AI (Optional)',
    desc: 'Ollama/LM Studio integration. Explain output, suggest commands, complete code — all local.',
  },
  {
    icon: '🖥️',
    title: 'Cross-Platform',
    desc: 'Metal on macOS, DX12 on Windows, Vulkan on Linux. Native feel everywhere.',
  },
  {
    icon: '⚙️',
    title: 'Zero Config',
    desc: 'Works out of the box. TOML + Lua for power users. Sensible defaults for everyone.',
  },
];

const stats = [
  { value: '120', label: 'FPS at 4K' },
  { value: '<50MB', label: 'RAM at idle' },
  { value: '<200ms', label: 'Cold start' },
  { value: '100%', label: 'Unicode pass' },
];

const platforms = [
  { name: 'macOS', badge: 'Metal', color: 'from-purple-500 to-pink-500' },
  { name: 'Windows', badge: 'DX12', color: 'from-blue-500 to-cyan-500' },
  { name: 'Linux', badge: 'Vulkan', color: 'from-orange-500 to-red-500' },
];

function TerminalDemo() {
  const [lines, setLines] = useState<string[]>([
    'user@zeroterm:~$ ',
  ]);
  const [currentLine, setCurrentLine] = useState('');
  const [cursor, setCursor] = useState(true);

  useEffect(() => {
    const interval = setInterval(() => setCursor(c => !c), 530);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const demoCommands = [
      { cmd: 'echo "Zero latency. Zero bloat. Zero config."', delay: 800 },
      { cmd: 'zeroterm --version', delay: 1600 },
      { cmd: 'ZeroTerm v0.1.0 (rustc 1.79)', delay: 2000 },
      { cmd: 'htop', delay: 2800 },
      { cmd: '  CPU: ████░░░░░░ 12%  MEM: ████░░░░░░ 48MB', delay: 3200 },
      { cmd: '  GPU: ████████░░ 89%  FPS: 120', delay: 3600 },
      { cmd: 'ssh prod-server', delay: 4400 },
      { cmd: 'Connected to prod-server (persistent session)', delay: 4800 },
      { cmd: 'user@prod:~$ ', delay: 5200 },
    ];

    let totalDelay = 0;
    demoCommands.forEach(({ cmd, delay }) => {
      totalDelay += delay;
      setTimeout(() => {
        setLines(prev => {
          const last = prev[prev.length - 1];
          if (last.endsWith('$ ') || last.endsWith('# ')) {
            return [...prev.slice(0, -1), last + cmd, 'user@zeroterm:~$ '];
          }
          return [...prev, cmd, 'user@zeroterm:~$ '];
        });
      }, totalDelay);
    });
  }, []);

  return (
    <div className="terminal-font bg-[var(--card)] border border-[var(--border)] rounded-xl overflow-hidden max-w-3xl mx-auto animate-fade-in-up delay-3">
      <div className="flex items-center gap-2 px-4 py-3 bg-[var(--bg-elevated)] border-b border-[var(--border)]">
        <div className="w-3 h-3 rounded-full bg-red-500" />
        <div className="w-3 h-3 rounded-full bg-yellow-500" />
        <div className="w-3 h-3 rounded-full bg-green-500" />
        <span className="ml-4 text-sm text-[var(--fg-muted)]">zeroterm</span>
      </div>
      <div className="p-4 h-64 overflow-y-auto">
        <pre className="whitespace-pre-wrap text-sm leading-relaxed">
          {lines.map((line, i) => (
            <div key={i} className="flex items-start">
              <span className="text-[var(--accent)] mr-2">{i === lines.length - 1 ? '▶' : '✓'}</span>
              <span>{line}</span>
              {i === lines.length - 1 && <span className={`animate-cursor text-[var(--accent)] ml-1`}>█</span>}
            </div>
          ))}
        </pre>
      </div>
    </div>
  );
}

function Hero() {
  return (
    <section className="relative min-h-screen flex items-center justify-center px-6 py-20 bg-grid">
      <div className="absolute inset-0 bg-gradient-to-br from-[var(--accent)]/5 via-transparent to-transparent" />
      <div className="relative z-10 max-w-5xl mx-auto text-center">
        <div className="animate-fade-in-up">
          <span className="inline-block px-4 py-1.5 rounded-full border border-[var(--accent)]/30 bg-[var(--accent)]/10 text-[var(--accent)] text-sm font-medium mb-6">
            Now in Public Beta
          </span>
        </div>
        <h1 className="text-5xl md:text-7xl lg:text-8xl font-bold leading-[1.05] tracking-tight animate-fade-in-up delay-1">
          <span className="bg-gradient-to-r from-[var(--fg)] via-[var(--accent)] to-[var(--fg)] bg-clip-text text-transparent">
            ZeroTerm
          </span>
        </h1>
        <p className="mt-6 text-xl md:text-2xl text-[var(--fg-muted)] max-w-3xl mx-auto animate-fade-in-up delay-2 text-balance">
          Zero latency. Zero bloat. Zero config. Zero cloud. Zero tools.
        </p>
        <p className="mt-4 text-lg text-[var(--fg-muted)] animate-fade-in-up delay-2">
          GPU-accelerated terminal emulator built in Rust. 120 FPS at 4K. {'<50MB'}  RAM. Native multiplexing.
        </p>
        <div className="mt-10 flex flex-col sm:flex-row items-center justify-center gap-4 animate-fade-in-up delay-3">
          <Link
            href="https://github.com/mahesh-diwan/ZeroTerm/releases"
            className="group px-8 py-4 rounded-xl bg-[var(--accent)] text-[var(--bg)] font-semibold text-lg hover:scale-[1.02] transition-transform glow-accent"
          >
            Download Latest Release
            <span className="ml-2 inline-block group-hover:translate-x-1 transition-transform">→</span>
          </Link>
          <Link
            href="https://github.com/mahesh-diwan/ZeroTerm"
            className="px-8 py-4 rounded-xl border border-[var(--border)] text-[var(--fg)] font-semibold text-lg hover:bg-[var(--card-hover)] transition-colors"
          >
            View on GitHub
          </Link>
        </div>
        <TerminalDemo />
      </div>
    </section>
  );
}

function Stats() {
  return (
    <section className="py-20 px-6 bg-[var(--bg-elevated)]/50 border-y border-[var(--border)]">
      <div className="max-w-5xl mx-auto grid grid-cols-2 md:grid-cols-4 gap-8">
        {stats.map((stat, i) => (
          <div key={stat.label} className="text-center animate-fade-in-up" style={{ animationDelay: `${i * 0.1}s` }}>
            <div className="text-4xl md:text-5xl lg:text-6xl font-bold bg-gradient-to-r from-[var(--accent)] to-[var(--fg)] bg-clip-text text-transparent">
              {stat.value}
            </div>
            <div className="mt-2 text-[var(--fg-muted)] text-lg">{stat.label}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Features() {
  return (
    <section className="py-20 px-6">
      <div className="max-w-5xl mx-auto">
        <div className="text-center mb-16 animate-fade-in-up">
          <h2 className="text-3xl md:text-4xl font-bold">Built Different</h2>
          <p className="mt-4 text-[var(--fg-muted)] text-lg max-w-2xl mx-auto">
            Every feature exists because developers asked for it. No bloat, no telemetry, no accounts.
          </p>
        </div>
        <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-6">
          {features.map((feature, i) => (
            <div
              key={feature.title}
              className="group p-6 rounded-2xl bg-[var(--card)] border border-[var(--border)] hover:border-[var(--accent)]/50 hover:bg-[var(--card-hover)] transition-all duration-300 animate-fade-in-up"
              style={{ animationDelay: `${i * 0.08}s` }}
            >
              <div className="text-4xl mb-4">{feature.icon}</div>
              <h3 className="text-xl font-semibold mb-2">
                {feature.title}
                {feature.titleExtra && <span className="text-[var(--accent)] ml-2 text-base">{feature.titleExtra}</span>}
              </h3>
              <p className="text-[var(--fg-muted)] leading-relaxed">{feature.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Platforms() {
  return (
    <section className="py-20 px-6 bg-[var(--bg-elevated)]/50 border-y border-[var(--border)]">
      <div className="max-w-5xl mx-auto">
        <div className="text-center mb-16 animate-fade-in-up">
          <h2 className="text-3xl md:text-4xl font-bold">Native Everywhere</h2>
          <p className="mt-4 text-[var(--fg-muted)] text-lg max-w-2xl mx-auto">
            Not a web app wrapped in Tauri. Native GPU APIs on every platform.
          </p>
        </div>
        <div className="grid md:grid-cols-3 gap-6">
          {platforms.map((platform, i) => (
            <div
              key={platform.name}
              className="p-8 rounded-2xl bg-[var(--card)] border border-[var(--border)] text-center animate-fade-in-up"
              style={{ animationDelay: `${i * 0.1}s` }}
            >
              <div className={`inline-flex items-center gap-2 px-4 py-2 rounded-full bg-gradient-to-r ${platform.color} text-white font-medium mb-4`}>
                {platform.badge}
              </div>
              <h3 className="text-2xl font-bold mb-2">{platform.name}</h3>
              <p className="text-[var(--fg-muted)]">
                {platform.name === 'macOS' && 'Metal via wgpu • Transparent titlebar • .app bundle • Notarized'}
                {platform.name === 'Windows' && 'DirectX 12 via wgpu • ConPTY • Acrylic/Mica • .msi installer'}
                {platform.name === 'Linux' && 'Vulkan via wgpu • Wayland + X11 • GTK4 dialogs • .deb/.rpm/Flatpak'}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Install() {
  const commands = {
    cargo: 'cargo install zeroterm',
    brew: 'brew install zeroterm',
    scoop: 'scoop install zeroterm',
    aur: 'yay -S zeroterm',
    deb: 'sudo dpkg -i zeroterm_0.1.0_amd64.deb',
    rpm: 'sudo rpm -i zeroterm-0.1.0-1.x86_64.rpm',
    flatpak: 'flatpak install flathub dev.zeroterm.ZeroTerm',
    binary: 'curl -LsSf https://github.com/mahesh-diwan/ZeroTerm/releases/latest/download/zeroterm-x86_64-unknown-linux-gnu.tar.gz | tar xz && ./zeroterm',
  };

  const tabs = [
    { id: 'cargo', label: 'Cargo', cmd: commands.cargo },
    { id: 'brew', label: 'Homebrew', cmd: commands.brew },
    { id: 'scoop', label: 'Scoop', cmd: commands.scoop },
    { id: 'aur', label: 'AUR', cmd: commands.aur },
    { id: 'deb', label: '.deb', cmd: commands.deb },
    { id: 'rpm', label: '.rpm', cmd: commands.rpm },
    { id: 'flatpak', label: 'Flatpak', cmd: commands.flatpak },
    { id: 'binary', label: 'Binary', cmd: commands.binary },
  ];

  const [activeTab, setActiveTab] = useState('cargo');

  return (
    <section className="py-20 px-6">
      <div className="max-w-5xl mx-auto">
        <div className="text-center mb-12 animate-fade-in-up">
          <h2 className="text-3xl md:text-4xl font-bold">Install in Seconds</h2>
          <p className="mt-4 text-[var(--fg-muted)] text-lg">Pick your platform. One command. No dependencies.</p>
        </div>
        <div className="bg-[var(--card)] border border-[var(--border)] rounded-2xl overflow-hidden animate-fade-in-up delay-1">
          <div className="flex overflow-x-auto px-4 py-3 bg-[var(--bg-elevated)] border-b border-[var(--border)]">
            {tabs.map(tab => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`px-4 py-2 rounded-lg text-sm font-medium whitespace-nowrap transition-colors ${
                  activeTab === tab.id
                    ? 'bg-[var(--accent)] text-[var(--bg)]'
                    : 'text-[var(--fg-muted)] hover:text-[var(--fg)] hover:bg-[var(--card)]'
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>
          <div className="p-6">
            <div className="terminal-font bg-[var(--bg)] border border-[var(--border)] rounded-xl p-4 relative">
              <div className="flex items-center gap-2 mb-3">
                <div className="w-3 h-3 rounded-full bg-red-500" />
                <div className="w-3 h-3 rounded-full bg-yellow-500" />
                <div className="w-3 h-3 rounded-full bg-green-500" />
              </div>
              <pre className="text-sm text-[var(--fg)]">
                <code>{tabs.find(t => t.id === activeTab)?.cmd}</code>
              </pre>
              <button
                className="absolute top-4 right-4 text-[var(--fg-muted)] hover:text-[var(--accent)] text-sm"
                onClick={() => navigator.clipboard.writeText(tabs.find(t => t.id === activeTab)?.cmd || '')}
              >
                Copy
              </button>
            </div>
          </div>
        </div>
        <p className="mt-8 text-center text-[var(--fg-muted)] animate-fade-in-up delay-2">
          Or download from <Link href="https://github.com/mahesh-diwan/ZeroTerm/releases" className="text-[var(--accent)] hover:underline">GitHub Releases</Link>
        </p>
      </div>
    </section>
  );
}

function Roadmap() {
  const phases = [
    {
      num: '1',
      title: 'The Engine',
      status: '✅ Complete',
      items: ['PTY Integration', 'VT100 Parser', 'Screen Buffer', 'wgpu Rendering', 'Input Handling'],
      timeframe: 'Months 1–3',
    },
    {
      num: '2',
      title: 'Multiplexing',
      status: '🚧 In Progress',
      items: ['Tab System', 'Splits (Tiling)', 'SSH Integration', 'Session Restore'],
      timeframe: 'Months 3–4',
    },
    {
      num: '3',
      title: 'Modern UX',
      status: '📋 Planned',
      items: ['Block Output', 'Modern Input', 'Graphics Protocols', 'Local AI', 'GUI Settings'],
      timeframe: 'Months 4–6',
    },
    {
      num: '4',
      title: 'Cross-Platform Polish',
      status: '📋 Planned',
      items: ['macOS Native', 'Windows Native', 'Linux Native', 'Encrypted Sync'],
      timeframe: 'Months 6–8',
    },
    {
      num: '5',
      title: 'Ecosystem & v1.0',
      status: '📋 Planned',
      items: ['WASM Plugins', 'Documentation', 'Plugin Marketplace', 'v1.0 Release'],
      timeframe: 'Months 8–12',
    },
  ];

  return (
    <section className="py-20 px-6 bg-[var(--bg-elevated)]/50 border-y border-[var(--border)]">
      <div className="max-w-5xl mx-auto">
        <div className="text-center mb-16 animate-fade-in-up">
          <h2 className="text-3xl md:text-4xl font-bold">Roadmap</h2>
          <p className="mt-4 text-[var(--fg-muted)] text-lg max-w-2xl mx-auto">
            Transparent development. No surprises. Community-driven priorities.
          </p>
        </div>
        <div className="relative">
          <div className="absolute left-8 top-0 bottom-0 w-0.5 bg-gradient-to-b from-[var(--accent)] to-[var(--border)]" />
          {phases.map((phase, i) => (
            <div key={phase.num} className="relative pl-20 pb-16 animate-fade-in-up" style={{ animationDelay: `${i * 0.1}s` }}>
              <div className="absolute left-0 top-0 flex items-center justify-center">
                <div className={`w-16 h-16 rounded-full border-4 flex items-center justify-center text-2xl font-bold z-10 bg-[var(--bg)] ${
                  phase.status.includes('Complete') ? 'border-[var(--accent)] text-[var(--accent)]' :
                  phase.status.includes('Progress') ? 'border-[var(--accent)]/50 text-[var(--accent)]' :
                  'border-[var(--border)] text-[var(--fg-muted)]'
                }`}>
                  {phase.num}
                </div>
              </div>
              <div className="bg-[var(--card)] border border-[var(--border)] rounded-2xl p-6 ml-4">
                <div className="flex items-baseline gap-4 mb-4">
                  <h3 className="text-xl font-bold">{phase.title}</h3>
                  <span className={`px-3 py-1 rounded-full text-sm font-medium ${
                    phase.status.includes('Complete') ? 'bg-[var(--accent)]/20 text-[var(--accent)] border border-[var(--accent)]/30' :
                    phase.status.includes('Progress') ? 'bg-blue-500/20 text-blue-400 border border-blue-500/30' :
                    'bg-[var(--bg-elevated)] text-[var(--fg-muted)] border border-[var(--border)]'
                  }`}>
                    {phase.status}
                  </span>
                </div>
                <p className="text-[var(--fg-muted)] mb-4">{phase.timeframe}</p>
                <ul className="grid grid-cols-2 gap-2 text-sm">
                  {phase.items.map(item => (
                    <li key={item} className="flex items-center gap-2 text-[var(--fg-muted)]">
                      <span className="w-1.5 h-1.5 rounded-full bg-[var(--border)]" />
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

export default function Home() {
  return (
    <div className="min-h-screen">
      <Hero />
      <Stats />
      <Features />
      <Platforms />
      <Install />
      <Roadmap />
    </div>
  );
}