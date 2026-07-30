'use client';

import { useState, useEffect } from 'react';
import Link from 'next/link';

const platforms = [
  {
    name: 'Linux',
    icon: '🐧',
    gradient: 'from-orange-500 to-red-500',
    install: 'curl -LsSf https://github.com/mahesh-diwan/ZeroTerm/releases/latest/download/zeroterm-installer.sh | sh',
    alt: 'sudo dpkg -i zeroterm_0.1.0_amd64.deb',
    pkg: 'yay -S zeroterm',
    assets: ['zeroterm-x86_64-unknown-linux-gnu.tar.gz', 'zeroterm_0.1.0_amd64.deb', 'zeroterm-0.1.0-1.x86_64.rpm'],
  },
  {
    name: 'macOS',
    icon: '🍎',
    gradient: 'from-purple-500 to-pink-500',
    install: 'brew install zeroterm',
    alt: 'curl -LsSf https://github.com/mahesh-diwan/ZeroTerm/releases/latest/download/zeroterm-installer.sh | sh',
    pkg: 'cargo install zeroterm',
    assets: ['ZeroTerm-0.1.0-x86_64.dmg', 'ZeroTerm-0.1.0-aarch64.dmg', 'zeroterm-x86_64-apple-darwin.tar.gz'],
  },
  {
    name: 'Windows',
    icon: '🪟',
    gradient: 'from-blue-500 to-cyan-500',
    install: 'winget install zeroterm',
    alt: 'scoop install zeroterm',
    pkg: 'cargo install zeroterm',
    assets: ['ZeroTerm-0.1.0-x64.msi', 'ZeroTerm-0.1.0-x64-portable.zip'],
  },
];

const releaseNotes = [
  { tag: 'v0.1.0', date: '2026-07-30', notes: ['Initial public beta release', 'GPU-accelerated rendering with wgpu', 'Native multiplexing (tabs + splits)', 'SSH integration with persistent sessions', 'Kitty/Sixel/iTerm2 image protocols', 'Local AI integration (Ollama/LM Studio)', 'TOML + Lua configuration'] },
];

function CommandCopy({ cmd, label }: { cmd: string; label: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center justify-between gap-4 p-3 bg-[var(--bg)] border border-[var(--border)] rounded-xl">
      <code className="text-sm text-[var(--fg)] truncate">{cmd}</code>
      <button
        onClick={() => { navigator.clipboard.writeText(cmd); setCopied(true); setTimeout(() => setCopied(false), 2000); }}
        className="shrink-0 px-3 py-1 rounded-lg bg-[var(--accent)]/20 text-[var(--accent)] text-xs font-medium hover:bg-[var(--accent)]/30 transition-colors"
      >
        {copied ? 'Copied!' : 'Copy'}
      </button>
    </div>
  );
}

export default function DownloadPage() {
  return (
    <div className="min-h-screen">
      <section className="py-16 px-6">
        <div className="max-w-5xl mx-auto">
          <div className="text-center mb-12">
            <h1 className="text-4xl md:text-5xl font-bold">Download ZeroTerm</h1>
            <p className="mt-4 text-lg text-[var(--fg-muted)]">Version 0.1.0 &mdash; Public Beta</p>
          </div>
          <div className="grid md:grid-cols-3 gap-6 mb-16">
            {platforms.map((p) => (
              <div key={p.name} className="bg-[var(--card)] border border-[var(--border)] rounded-2xl p-6 hover:border-[var(--accent)]/50 transition-colors">
                <div className="text-4xl mb-4">{p.icon}</div>
                <h2 className="text-2xl font-bold mb-4">{p.name}</h2>
                <div className="space-y-3">
                  <CommandCopy cmd={p.install} label="Install" />
                  <p className="text-xs text-[var(--fg-muted)]">Alternative: <code className="text-[var(--accent)]">{p.alt}</code></p>
                  <p className="text-xs text-[var(--fg-muted)]">Package manager: <code className="text-[var(--accent)]">{p.pkg}</code></p>
                </div>
                <div className="mt-6 pt-4 border-t border-[var(--border)]">
                  <p className="text-xs font-medium text-[var(--fg-muted)] mb-2">Downloads:</p>
                  <ul className="space-y-1">
                    {p.assets.map((a) => (
                      <li key={a}>
                        <a href={`https://github.com/mahesh-diwan/ZeroTerm/releases/download/v0.1.0/${a}`} className="text-xs text-[var(--accent)] hover:underline">
                          {a}
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            ))}
          </div>
          <div className="bg-[var(--card)] border border-[var(--border)] rounded-2xl p-6">
            <h2 className="text-2xl font-bold mb-6">Release Notes</h2>
            {releaseNotes.map((r) => (
              <div key={r.tag}>
                <div className="flex items-baseline gap-4 mb-4">
                  <h3 className="text-lg font-semibold text-[var(--accent)]">{r.tag}</h3>
                  <span className="text-sm text-[var(--fg-muted)]">{r.date}</span>
                </div>
                <ul className="space-y-2">
                  {r.notes.map((n) => (
                    <li key={n} className="flex items-start gap-2 text-sm text-[var(--fg-muted)]">
                      <span className="text-[var(--accent)] mt-0.5">-</span>
                      {n}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}
