'use client';

import { useState, useEffect, useRef } from 'react';
import Link from 'next/link';

// ─── data ───────────────────────────────────────────────────────────────────

const features = [
  { title: 'GPU Accelerated', desc: 'Rendering on the GPU via wgpu — Metal, DX12, or Vulkan. No Electron. No JS runtime.', tag: 'Performance' },
  { title: 'Pure Rust', desc: 'One language across the entire stack, from the VT parser to the renderer.', tag: 'Core' },
  { title: 'Native Multiplexing', desc: 'Tabs, splits, sessions built in. No tmux or screen config needed.', tag: 'UX' },
  { title: 'SSH First', desc: 'Native SSH client. Persistent sessions survive disconnects.', tag: 'Network' },
  { title: 'Graphics Protocols', desc: 'Kitty, Sixel, iTerm2 inline images in your terminal.', tag: 'Graphics' },
  { title: 'Block Output', desc: 'Every command becomes a block — copy it, rerun it, see its exit status.', tag: 'UX' },
  { title: 'Cross-Platform', desc: 'Metal on macOS, Vulkan on Linux — Windows builds are still experimental.', tag: 'Platform' },
  { title: 'Zero Config', desc: 'Works out of the box. TOML and Lua for power users.', tag: 'Setup' },
];

const stats = [
  { value: '9', suffix: '', label: 'Rust crates' },
  { value: '348+', suffix: '', label: 'tests passing' },
  { value: '0', suffix: '', label: 'JS runtime' },
  { value: 'MIT', suffix: '', label: 'open source' },
];

const platforms = [
  { name: 'macOS', badge: 'Metal' },
  { name: 'Windows', badge: 'DX12' },
  { name: 'Linux', badge: 'Vulkan' },
];

const installers = [
  { id: 'script', label: 'Script', cmd: 'curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash' },
  { id: 'source', label: 'Source', cmd: 'git clone https://github.com/mahesh-diwan/ZeroTerm.git && cd ZeroTerm && cargo run --release' },
];

const faqs = [
  { q: 'What is ZeroTerm?', a: 'ZeroTerm is a GPU-accelerated terminal emulator built from scratch in Rust. It uses wgpu for rendering and supports native multiplexing, SSH, image protocols, and output-block navigation.' },
  { q: 'How do I install it?', a: 'Run the install script or build from source. ZeroTerm is not yet on crates.io, Homebrew, or Flathub; prebuilt binaries are attached to GitHub Releases.' },
  { q: 'Does it support tmux?', a: 'ZeroTerm has built-in multiplexing — tabs, splits, and session management. No tmux or screen needed, though tmux works if you prefer it.' },
  { q: 'Is it available on Windows?', a: 'Linux and macOS are the primary targets and run the full CI suite. A Windows build target exists, but it is not yet verified in CI — treat it as experimental.' },
  { q: 'Can I use it over SSH?', a: 'Yes. ZeroTerm has a native SSH client with persistent sessions. Disconnect without killing remote work.' },
  { q: 'Is it open source?', a: 'Yes. ZeroTerm is MIT licensed. Source code is available on GitHub.' },
];

const roadmap = [
  { phase: 1, title: 'The Engine', status: 'Complete', items: ['PTY Integration', 'VT100 Parser', 'Screen Buffer', 'wgpu Rendering', 'Input Handling'] },
  { phase: 2, title: 'Multiplexing', status: 'Complete', items: ['Tab System', 'Splits (Tiling)', 'SSH Integration', 'Session Restore'] },
  { phase: 3, title: 'Modern UX', status: 'Complete', items: ['Block Output', 'Graphics Protocols', 'Line Editor', 'GUI Settings'] },
  { phase: 4, title: 'Ecosystem', status: 'In Progress', items: ['macOS Native', 'Linux Native', 'Encrypted Sync', 'WASM Plugins', 'Windows Native (unverified)'] },
];

// ─── hooks ──────────────────────────────────────────────────────────────────

function useInView(threshold = 0.15) {
  const ref = useRef<HTMLDivElement>(null);
  const [inView, setInView] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const obs = new IntersectionObserver(([e]) => { if (e.isIntersecting) { setInView(true); obs.disconnect(); } }, { threshold });
    obs.observe(el);
    return () => obs.disconnect();
  }, [threshold]);
  return [ref, inView] as const;
}

function AnimatedSection({ children, className = '', delay = 0 }: { children: React.ReactNode; className?: string; delay?: number }) {
  const [ref, inView] = useInView();
  return (
    <div
      ref={ref}
      className={`transition-all duration-700 ease-[cubic-bezier(0.16,1,0.3,1)] ${className} ${
        inView ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6'
      }`}
      style={{ transitionDelay: `${delay}ms` }}
    >
      {children}
    </div>
  );
}

// ─── Terminal Demo ──────────────────────────────────────────────────────────

type DemoLine =
  | { kind: 'cmd'; prompt: string; cmd: string }
  | { kind: 'out'; out: string; accent?: boolean };

const demoLines: DemoLine[] = [
  { kind: 'cmd', prompt: 'user@zeroterm:~', cmd: 'zeroterm --version' },
  { kind: 'out', out: 'zeroterm 0.3.0' },
  { kind: 'cmd', prompt: 'user@zeroterm:~', cmd: 'echo "GPU accelerated"' },
  { kind: 'out', out: 'GPU accelerated' },
  { kind: 'cmd', prompt: 'user@zeroterm:~', cmd: 'cat /proc/cpuinfo | grep cores' },
  { kind: 'out', out: 'cpu cores        : 8' },
  { kind: 'cmd', prompt: 'user@zeroterm:~', cmd: 'ssh prod-server' },
  { kind: 'out', out: 'Connected to prod-server (persistent session)', accent: true },
  { kind: 'cmd', prompt: 'user@prod:~', cmd: '' },
];

const lineStart: number[] = (() => {
  const starts: number[] = [];
  let idx = 0;
  for (const line of demoLines) {
    starts.push(idx);
    idx += 1 + (line.kind === 'cmd' ? line.cmd.length : 0);
  }
  return starts;
})();

const delays = (() => {
  const d: number[] = [];
  for (const line of demoLines) {
    if (line.kind === 'cmd') {
      d.push(150 + Math.random() * 120);
      for (let k = 0; k < line.cmd.length; k++) d.push(24 + Math.random() * 16);
    } else {
      d.push(160 + Math.random() * 120);
    }
  }
  return d;
})();

const HOLD_MS = 2600;

function TerminalDemo() {
  const [tick, setTick] = useState(0);
  const [run, setRun] = useState(0);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const step = (t: number) => {
      setTick(t);
      if (t < delays.length) {
        timer = setTimeout(() => step(t + 1), delays[t]);
      } else {
        timer = setTimeout(() => {
          setTick(0);
          setRun((r) => r + 1);
        }, HOLD_MS);
      }
    };
    step(0);
    return () => clearTimeout(timer);
  }, [run]);

  let cursorLine = -1;
  let cursorChars = 0;
  demoLines.forEach((line, i) => {
    if (line.kind === 'cmd' && tick > lineStart[i]) {
      cursorLine = i;
      cursorChars = Math.min(Math.max(tick - lineStart[i] - 1, 0), line.cmd.length);
    }
  });

  return (
    <div className="rounded-xl border border-border overflow-hidden bg-[#06060c] shadow-2xl shadow-accent-dim/20">
      <div className="flex items-center gap-1.5 px-4 py-3 bg-surface border-b border-border">
        <div className="w-2.5 h-2.5 rounded-full bg-red-500/80" />
        <div className="w-2.5 h-2.5 rounded-full bg-yellow-500/80" />
        <div className="w-2.5 h-2.5 rounded-full bg-green-500/80" />
        <span className="ml-3 text-xs text-fg-muted/60 font-mono">zeroterm</span>
      </div>
      <div className="p-4 min-h-[280px]">
        <pre className="text-sm leading-relaxed font-mono">
          {demoLines.map((line, i) => {
            const start = lineStart[i];
            if (tick <= start) return null;
            if (line.kind === 'cmd') {
              const chars = Math.min(Math.max(tick - start - 1, 0), line.cmd.length);
              return (
                <div key={i}>
                  <span className="text-fg-muted/80">{line.prompt}</span>
                  <span className="text-accent">$ </span>
                  <span className="text-fg/80">{line.cmd.slice(0, chars)}</span>
                  {i === cursorLine && (
                    <span className="inline-block w-2 h-4 bg-accent/80 animate-cursor ml-1 align-middle" />
                  )}
                </div>
              );
            }
            return (
              <div key={i}>
                <span className={line.accent ? 'text-accent' : 'text-fg'}>{line.out}</span>
              </div>
            );
          })}
        </pre>
      </div>
    </div>
  );
}

// ─── Sections ───────────────────────────────────────────────────────────────

function Hero() {
  return (
    <section className="relative overflow-hidden">
      <div className="absolute inset-0 bg-grid-subtle pointer-events-none" />
      <div className="absolute top-1/4 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[600px] h-[600px] bg-accent-glow rounded-full blur-[120px] pointer-events-none" />
      <div className="relative max-w-6xl mx-auto px-6 pt-28 pb-20 lg:pt-36 lg:pb-28">
        <div className="grid lg:grid-cols-2 gap-12 lg:gap-16 items-center">
          <AnimatedSection>
            <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full border border-accent/20 bg-accent-dim/5 text-accent text-xs font-medium mb-6">
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
              Now in Public Beta
            </div>
            <h1 className="text-4xl sm:text-5xl lg:text-6xl font-bold tracking-tighter leading-[1.05]">
              Zero latency.
              <br />
              Zero bloat.
              <br />
              <span className="text-accent">ZeroTerm.</span>
            </h1>
            <p className="mt-5 text-lg text-fg-muted max-w-xl leading-relaxed text-balance">
              GPU-accelerated terminal emulator built in Rust. 120 FPS at 4K, under 50MB RAM, native
              multiplexing, SSH, and image protocols — all in one binary.
            </p>
            <div className="mt-8 flex flex-col sm:flex-row gap-3">
              <Link
                href="https://github.com/mahesh-diwan/ZeroTerm/releases"
                className="inline-flex items-center justify-center gap-2 px-6 py-3 rounded-xl bg-accent text-bg font-semibold text-sm hover:brightness-110 transition-all active:scale-[0.98]"
              >
                Download Now
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" /></svg>
              </Link>
              <Link
                href="https://github.com/mahesh-diwan/ZeroTerm"
                className="inline-flex items-center justify-center gap-2 px-6 py-3 rounded-xl border border-border text-fg text-sm font-medium hover:bg-surface-hover transition-all active:scale-[0.98]"
              >
                <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0C5.374 0 0 5.373 0 12c0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.305-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23A11.509 11.509 0 0112 5.803c1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.872.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576C20.566 21.797 24 17.3 24 12c0-6.627-5.373-12-12-12z" /></svg>
                GitHub
              </Link>
            </div>
          </AnimatedSection>
          <AnimatedSection delay={200}>
            <TerminalDemo />
          </AnimatedSection>
        </div>
      </div>
    </section>
  );
}

function Stats() {
  return (
    <section className="border-y border-border">
      <div className="max-w-6xl mx-auto px-6 py-12">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-8">
          {stats.map((s, i) => (
            <AnimatedSection key={s.label} delay={i * 80} className="text-center">
              <div className="text-3xl md:text-4xl font-bold tracking-tighter">
                <span className="text-accent">{s.value}</span>
                <span className="text-fg-muted/50 text-2xl md:text-3xl"> {s.suffix}</span>
              </div>
              <div className="mt-1 text-sm text-fg-muted">{s.label}</div>
            </AnimatedSection>
          ))}
        </div>
      </div>
    </section>
  );
}

function About() {
  return (
    <section className="py-24 lg:py-32">
      <div className="max-w-6xl mx-auto px-6">
        <AnimatedSection>
          <div className="grid lg:grid-cols-2 gap-12 lg:gap-20">
            <div>
              <h2 className="text-2xl sm:text-3xl lg:text-4xl font-bold tracking-tight leading-tight text-balance">
                A terminal built for how developers actually work.
              </h2>
            </div>
            <div className="space-y-6">
              <p className="text-fg-muted leading-relaxed text-balance">
                ZeroTerm is a GPU-accelerated terminal emulator written entirely in Rust. It uses wgpu for rendering,
                a fully custom VT parser, and native OS multiplexing — no Electron, no JavaScript runtime, no bloat.
              </p>
              <div className="space-y-3">
                {[
                  'GPU-accelerated rendering via wgpu — Metal, DX12, or Vulkan',
                  'Native multiplexing — tabs, splits, session management',
                  'Built-in SSH client with persistent sessions',
                  'Kitty / Sixel / iTerm2 image protocol support',
                  'Output-block navigation and search',
                  'TOML + Lua config for power users',
                ].map((item, i) => (
                  <div key={i} className="flex items-start gap-3 text-sm text-fg leading-relaxed">
                    <svg className="w-4 h-4 mt-0.5 shrink-0 text-accent" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    {item}
                  </div>
                ))}
              </div>
            </div>
          </div>
        </AnimatedSection>
      </div>
    </section>
  );
}

function Features() {
  return (
    <section className="py-24 lg:py-32 border-t border-border">
      <div className="max-w-6xl mx-auto px-6">
        <AnimatedSection className="max-w-2xl mb-16">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight">Everything you need, nothing you don&apos;t.</h2>
          <p className="mt-4 text-fg-muted leading-relaxed text-balance">
            From GPU-accelerated rendering to native multiplexing. ZeroTerm ships every feature as a single, native binary.
          </p>
        </AnimatedSection>
        <div className="grid sm:grid-cols-2 lg:grid-cols-4 gap-4">
          {features.map((f, i) => (
            <AnimatedSection key={f.title} delay={i * 60} className="group p-5 rounded-xl border border-border hover:border-border-light bg-surface hover:bg-surface-hover transition-all duration-300">
              <div className="text-[10px] uppercase tracking-widest text-fg-dim font-medium mb-3">{f.tag}</div>
              <h3 className="font-semibold mb-1.5">{f.title}</h3>
              <p className="text-sm text-fg-muted leading-relaxed">{f.desc}</p>
            </AnimatedSection>
          ))}
        </div>
      </div>
    </section>
  );
}

function Platforms() {
  return (
    <section className="py-24 lg:py-32 border-t border-border bg-surface/30">
      <div className="max-w-6xl mx-auto px-6">
        <AnimatedSection className="max-w-2xl mb-16">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight">Native everywhere.</h2>
          <p className="mt-4 text-fg-muted leading-relaxed text-balance">
            Not a web app wrapped in Tauri. Native GPU APIs on every platform.
          </p>
        </AnimatedSection>
        <div className="grid sm:grid-cols-3 gap-4">
          {platforms.map((p, i) => (
            <AnimatedSection key={p.name} delay={i * 80} className="p-6 rounded-xl border border-border bg-surface">
              <div className="inline-flex items-center gap-2 px-2.5 py-1 rounded-md bg-accent-dim/10 text-accent text-xs font-mono font-medium mb-4">{p.badge}</div>
              <h3 className="text-lg font-semibold mb-2">{p.name}</h3>
              <p className="text-sm text-fg-muted leading-relaxed">
                {p.name === 'macOS' && 'Metal via wgpu. Transparent titlebar. Notarized .app bundle.'}
                {p.name === 'Windows' && 'DirectX 12 via wgpu. ConPTY. Acrylic and Mica effects.'}
                {p.name === 'Linux' && 'Vulkan via wgpu. Wayland and X11. .deb, .rpm, and Flatpak.'}
              </p>
            </AnimatedSection>
          ))}
        </div>
      </div>
    </section>
  );
}

function InstallSection() {
  const [active, setActive] = useState('script');
  const [copied, setCopied] = useState(false);
  const activeCmd = installers.find((i) => i.id === active)?.cmd ?? '';

  return (
    <section className="py-24 lg:py-32">
      <div className="max-w-6xl mx-auto px-6">
        <AnimatedSection className="max-w-2xl mb-16">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight">Install in seconds.</h2>
          <p className="mt-4 text-fg-muted leading-relaxed text-balance">One command, no dependencies. Prebuilt binaries on every release.</p>
        </AnimatedSection>
        <AnimatedSection delay={100} className="max-w-2xl mx-auto">
          <div className="rounded-xl border border-border overflow-hidden">
            <div className="flex gap-1 px-4 pt-3 pb-2 bg-surface border-b border-border">
              {installers.map((inst) => (
                <button
                  key={inst.id}
                  onClick={() => setActive(inst.id)}
                  className={`px-3 py-1.5 rounded-md text-xs font-medium transition-all ${
                    active === inst.id ? 'bg-accent text-bg' : 'text-fg-muted hover:text-fg hover:bg-surface-hover'
                  }`}
                >
                  {inst.label}
                </button>
              ))}
            </div>
            <div className="p-4 bg-bg font-mono text-sm flex items-center justify-between">
              <code className="text-fg/90">{activeCmd}</code>
              <button
                onClick={() => { navigator.clipboard.writeText(activeCmd); setCopied(true); setTimeout(() => setCopied(false), 1500); }}
                className="shrink-0 px-3 py-1 rounded-md text-xs font-medium text-fg-muted hover:text-fg hover:bg-surface-hover transition-all"
              >
                {copied ? 'Copied' : 'Copy'}
              </button>
            </div>
          </div>
          <p className="mt-4 text-center text-sm text-fg-muted">
            Or download from{' '}
            <Link href="https://github.com/mahesh-diwan/ZeroTerm/releases" className="text-accent hover:underline">GitHub Releases</Link>
          </p>
        </AnimatedSection>
      </div>
    </section>
  );
}

function FAQ() {
  const [open, setOpen] = useState<number | null>(null);

  return (
    <section className="py-24 lg:py-32 border-t border-border bg-surface/30">
      <div className="max-w-3xl mx-auto px-6">
        <AnimatedSection className="text-center mb-16">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight">Frequently asked questions.</h2>
        </AnimatedSection>
        <div className="space-y-2">
          {faqs.map((faq, i) => (
            <AnimatedSection key={i} delay={i * 60}>
              <button
                onClick={() => setOpen(open === i ? null : i)}
                className="w-full flex items-center justify-between gap-4 px-5 py-4 rounded-xl border border-border hover:border-border-light bg-surface hover:bg-surface-hover transition-all text-left"
              >
                <span className="font-medium text-sm">{faq.q}</span>
                <svg
                  className={`w-4 h-4 shrink-0 text-fg-muted transition-transform duration-300 ${open === i ? 'rotate-45' : ''}`}
                  fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                </svg>
              </button>
              <div
                className={`overflow-hidden transition-all duration-300 ease-[cubic-bezier(0.16,1,0.3,1)] ${
                  open === i ? 'max-h-48 opacity-100' : 'max-h-0 opacity-0'
                }`}
              >
                <p className="px-5 py-4 text-sm text-fg-muted leading-relaxed">{faq.a}</p>
              </div>
            </AnimatedSection>
          ))}
        </div>
      </div>
    </section>
  );
}

function Roadmap() {
  return (
    <section className="py-24 lg:py-32 border-t border-border">
      <div className="max-w-6xl mx-auto px-6">
        <AnimatedSection className="max-w-2xl mb-16">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight">Roadmap.</h2>
          <p className="mt-4 text-fg-muted leading-relaxed text-balance">Transparent development. Community-driven priorities.</p>
        </AnimatedSection>
        <div className="grid sm:grid-cols-2 lg:grid-cols-4 gap-4">
          {roadmap.map((r, i) => (
            <AnimatedSection key={r.phase} delay={i * 80} className="p-5 rounded-xl border border-border bg-surface">
              <div className="flex items-center gap-3 mb-4">
                <div className={`w-8 h-8 rounded-full flex items-center justify-center text-xs font-bold border ${
                  r.status === 'Complete' ? 'border-accent text-accent' : 'border-border text-fg-muted'
                }`}>
                  {r.phase}
                </div>
                <span className={`text-[10px] uppercase tracking-widest font-medium ${
                  r.status === 'Complete' ? 'text-accent' : 'text-fg-dim'
                }`}>
                  {r.status}
                </span>
              </div>
              <h3 className="font-semibold mb-3">{r.title}</h3>
              <ul className="space-y-1.5">
                {r.items.map((item) => (
                  <li key={item} className="flex items-center gap-2 text-sm text-fg-muted">
                    <span className="w-1 h-1 rounded-full bg-fg-dim shrink-0" />
                    {item}
                  </li>
                ))}
              </ul>
            </AnimatedSection>
          ))}
        </div>
      </div>
    </section>
  );
}

// ─── Page ────────────────────────────────────────────────────────────────────

export default function Home() {
  return (
    <>
      <Hero />
      <Stats />
      <About />
      <Features />
      <Platforms />
      <InstallSection />
      <FAQ />
      <Roadmap />
    </>
  );
}
