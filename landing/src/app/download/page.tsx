'use client';

import { useState, useEffect } from 'react';
import Link from 'next/link';

const platforms = [
  {
    name: 'Linux',
    icon: '🐧',
    gradient: 'from-orange-500 to-red-500',
    install: 'curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash',
    alt: 'git clone https://github.com/mahesh-diwan/ZeroTerm.git && cd ZeroTerm && cargo run --release',
    pkg: 'not on crates.io yet — build from source',
    assets: ['ZeroTerm-x86_64.AppImage', 'zeroterm_0.3.0_amd64.deb', 'zeroterm-0.3.0-1.x86_64.rpm'],
  },
  {
    name: 'macOS',
    icon: '🍎',
    gradient: 'from-purple-500 to-pink-500',
    install: 'curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash',
    alt: 'git clone https://github.com/mahesh-diwan/ZeroTerm.git && cd ZeroTerm && cargo run --release',
    pkg: 'not on Homebrew yet — build from source',
    assets: ['zeroterm-v0.3.0-macos-arm64.zip'],
  },
  {
    name: 'Windows',
    icon: '🪟',
    gradient: 'from-blue-500 to-cyan-500',
    install: 'curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash',
    alt: 'cargo build --release -p zeroterm',
    pkg: 'experimental — not yet verified in CI',
    assets: ['zeroterm-v0.3.0-windows-x86_64.zip'],
  },
];

const releaseNotes = [
  { tag: 'v0.3.0', date: '2026-08-06', notes: ['Fixed split panes never appearing in the split tree', '`clear` no longer wipes scrollback', 'Window resizes preserve scrollback', 'Invalid UTF-8 no longer swallows following text', 'Release pipeline publishes prebuilt binaries to GitHub Releases', '`scripts/install.sh` resolves the latest tag and installs prebuilt packages'] },
  { tag: 'v0.2.0', date: '2026-07-30', notes: ['GPU-accelerated rendering with wgpu', 'Native multiplexing (tabs + splits)', 'SSH integration with persistent sessions',    'Kitty/Sixel/iTerm2 image protocols', 'Output-block navigation', 'TOML + Lua configuration', 'E2E-encrypted sync, WASM plugins'] },
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

const DOWNLOAD_FILE = 'ZeroTerm-x86_64.AppImage';
const TOTAL_MB = 24.8;
const DL_LINE = `Downloading ${DOWNLOAD_FILE}`;
const EX_LINE = 'Extracting...';
const INSTALL_LINE = 'Installing to ~/.local/bin';

type Phase = 'type-dl' | 'download' | 'type-ex' | 'extract' | 'type-install' | 'done';

const TYPE_TARGET: Partial<Record<Phase, string>> = {
  'type-dl': DL_LINE,
  'type-ex': EX_LINE,
  'type-install': INSTALL_LINE,
};

function PacmanBar({ pct, len, mouth, dim }: { pct: number; len: number; mouth: boolean; dim?: boolean }) {
  const i = Math.min(len - 1, Math.floor((pct / 100) * len));
  const rest = Math.max(0, len - i - 1);
  return (
    <span className={dim ? 'opacity-50' : ''}>
      <span className="text-[var(--fg-muted)]">[{' '.repeat(i)}</span>
      <span className="text-[var(--accent)]">{mouth ? 'ᗧ' : 'ᗣ'}</span>
      <span className="text-[var(--fg-muted)]">{'·'.repeat(rest)}]</span>
    </span>
  );
}

function PacmanDownloadDemo() {
  const [phase, setPhase] = useState<Phase>('type-dl');
  const [typed, setTyped] = useState(0);
  const [pct, setPct] = useState(0);
  const [dlDone, setDlDone] = useState(false);
  const [exDone, setExDone] = useState(false);

  useEffect(() => {
    const target = TYPE_TARGET[phase];
    if (!target) return;
    if (typed < target.length) {
      const id = setTimeout(() => setTyped(typed + 1), 18);
      return () => clearTimeout(id);
    }
    const id = setTimeout(() => {
      if (phase === 'type-dl') setPhase('download');
      else if (phase === 'type-ex') setPhase('extract');
      else setPhase('done');
    }, 300);
    return () => clearTimeout(id);
  }, [phase, typed]);

  useEffect(() => {
    if (phase !== 'download' && phase !== 'extract') return;
    const step = phase === 'download' ? 0.5 : 2;
    const id = setInterval(() => setPct((p) => Math.min(100, p + step)), 40);
    return () => clearInterval(id);
  }, [phase]);

  useEffect(() => {
    if (pct < 100) return;
    if (phase === 'download') {
      setPhase('type-ex');
      setTyped(0);
      setDlDone(true);
    } else if (phase === 'extract') {
      setPhase('type-install');
      setTyped(0);
      setExDone(true);
    }
    setPct(0);
  }, [phase, pct]);

  useEffect(() => {
    if (phase !== 'done') return;
    const id = setTimeout(() => {
      setPhase('type-dl');
      setTyped(0);
      setPct(0);
      setDlDone(false);
      setExDone(false);
    }, 4000);
    return () => clearTimeout(id);
  }, [phase]);

  const mouth = phase === 'download' || phase === 'extract'
    ? Math.floor(pct / (phase === 'download' ? 2 : 6)) % 2 === 0
    : true;
  const speed = 3.8 + 1.4 * (pct / 100);
  const downloaded = TOTAL_MB * (pct / 100);
  const typing = TYPE_TARGET[phase];

  return (
    <div className="bg-[var(--card)] border border-[var(--border)] rounded-2xl p-6 mb-16">
      <h2 className="text-2xl font-bold mb-1">Watch it install</h2>
      <p className="text-sm text-[var(--fg-muted)] mb-6">A live preview of the ZeroTerm download flow.</p>
      <div className="rounded-xl border border-[var(--border)] overflow-hidden bg-[#06060c]">
        <div className="flex items-center gap-1.5 px-4 py-2.5 bg-[var(--bg)] border-b border-[var(--border)]">
          <div className="w-2.5 h-2.5 rounded-full bg-red-500/80" />
          <div className="w-2.5 h-2.5 rounded-full bg-yellow-500/80" />
          <div className="w-2.5 h-2.5 rounded-full bg-green-500/80" />
          <span className="ml-3 text-xs text-[var(--fg-muted)] font-mono">zeroterm install</span>
        </div>
        <div className="p-4 min-h-[190px] font-mono text-sm leading-7 whitespace-pre text-[var(--fg)]">
          <div>{phase === 'type-dl' ? typing : DL_LINE}</div>
          {phase === 'download' && (
            <div>
              <PacmanBar pct={pct} len={12} mouth={mouth} /> {Math.floor(pct)}% {speed.toFixed(1)} MB/s  {downloaded.toFixed(1)} MB / {TOTAL_MB} MB
            </div>
          )}
          {dlDone && phase !== 'download' && (
            <div>
              <PacmanBar pct={100} len={12} mouth={false} dim /> 100%  5.2 MB/s  {TOTAL_MB} MB / {TOTAL_MB} MB
            </div>
          )}
          {(exDone || phase === 'type-ex') && <div>{phase === 'type-ex' ? typing : EX_LINE}</div>}
          {phase === 'extract' && (
            <div>
              <PacmanBar pct={pct} len={6} mouth={mouth} /> {Math.floor(pct)}%
            </div>
          )}
          {exDone && phase !== 'extract' && (
            <div>
              <PacmanBar pct={100} len={6} mouth={false} dim /> 100%
            </div>
          )}
          {exDone && phase !== 'extract' && <div>{phase === 'type-install' ? typing : INSTALL_LINE}</div>}
          {phase === 'done' && (
            <div>
              <span className="text-green-500">✓</span> Done. Run 'zeroterm'
            </div>
          )}
          {(phase === 'type-dl' || phase === 'type-ex' || phase === 'type-install') && (
            <span className="inline-block w-2 h-4 bg-[var(--accent)] animate-cursor ml-1" />
          )}
        </div>
      </div>
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
            <p className="mt-4 text-lg text-[var(--fg-muted)]">Version 0.3.0</p>
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
                        <a href={`https://github.com/mahesh-diwan/ZeroTerm/releases/download/v0.3.0/${a}`} className="text-xs text-[var(--accent)] hover:underline">
                          {a}
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            ))}
          </div>
          <PacmanDownloadDemo />
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
