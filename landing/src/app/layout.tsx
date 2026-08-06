import type { Metadata, Viewport } from 'next';
import { Geist, Geist_Mono } from 'next/font/google';
import Link from 'next/link';
import './globals.css';

const geist = Geist({
  subsets: ['latin'],
  variable: '--font-sans',
  display: 'swap',
});

const geistMono = Geist_Mono({
  subsets: ['latin'],
  variable: '--font-mono',
  display: 'swap',
});

export const metadata: Metadata = {
  title: 'ZeroTerm — GPU-Accelerated Terminal Emulator',
  description:
    'GPU-accelerated terminal emulator written in Rust with wgpu. Tabs, splits, SSH, image protocols, and optional local AI — no Electron, no JS runtime.',
  keywords: [
    'terminal', 'rust', 'gpu', 'wgpu', 'multiplexer', 'pty',
    'ssh', 'developer-tools', 'terminal-emulator',
  ],
  authors: [{ name: 'ZeroTerm Contributors' }],
  creator: 'ZeroTerm Contributors',
  openGraph: {
    type: 'website',
    locale: 'en_US',
    url: 'https://zeroterm.dev',
    siteName: 'ZeroTerm',
    title: 'ZeroTerm — GPU-Accelerated Terminal Emulator',
    description:
      'GPU-accelerated terminal emulator written in Rust with wgpu. Tabs, splits, SSH, image protocols, and optional local AI.',
    images: [{ url: '/og-image.png', width: 1200, height: 630, alt: 'ZeroTerm' }],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'ZeroTerm — GPU-Accelerated Terminal Emulator',
    description: 'GPU-accelerated terminal emulator written in Rust — no Electron, no JS runtime.',
    images: ['/og-image.png'],
  },
};

export const viewport: Viewport = {
  themeColor: '#08080e',
  width: 'device-width',
  initialScale: 1,
};

const navLinks = [
  { href: '/', label: 'Home' },
  { href: '/download', label: 'Download' },
  { href: '/docs', label: 'Docs' },
];

function Nav() {
  return (
    <header className="fixed top-0 inset-x-0 z-50 flex justify-center pt-4 pointer-events-none">
      <div className="pointer-events-auto flex items-center gap-6 px-5 h-11 rounded-full bg-surface/80 backdrop-blur-xl border border-border text-sm">
        <Link
          href="/"
          className="font-semibold tracking-tight hover:text-accent transition-colors"
        >
          ZeroTerm
        </Link>
        <div className="w-px h-4 bg-border" />
        {navLinks.map((l) => (
          <Link
            key={l.href}
            href={l.href}
            className="text-fg-muted hover:text-fg transition-colors"
          >
            {l.label}
          </Link>
        ))}
        <div className="w-px h-4 bg-border" />
        <a
          href="https://github.com/mahesh-diwan/ZeroTerm"
          className="text-fg-muted hover:text-fg transition-colors"
          aria-label="GitHub"
        >
          <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
            <path d="M12 0C5.374 0 0 5.373 0 12c0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.305-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23A11.509 11.509 0 0112 5.803c1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.872.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576C20.566 21.797 24 17.3 24 12c0-6.627-5.373-12-12-12z" />
          </svg>
        </a>
      </div>
    </header>
  );
}

function Footer() {
  return (
    <footer className="border-t border-border">
      <div className="max-w-6xl mx-auto px-6 py-16">
        <div className="grid sm:grid-cols-2 lg:grid-cols-4 gap-10 mb-12">
          <div>
            <h4 className="font-semibold mb-4">ZeroTerm</h4>
            <p className="text-sm text-fg-muted leading-relaxed">
              GPU-accelerated terminal emulator built in Rust. Zero latency. Zero bloat. Zero config.
            </p>
          </div>
          <div>
            <h4 className="font-semibold mb-4">Links</h4>
            <ul className="space-y-2 text-sm text-fg-muted">
              <li><Link href="https://github.com/mahesh-diwan/ZeroTerm" className="hover:text-accent transition-colors">GitHub</Link></li>
              <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/releases" className="hover:text-accent transition-colors">Releases</Link></li>
              <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/blob/main/LICENSE" className="hover:text-accent transition-colors">MIT License</Link></li>
            </ul>
          </div>
          <div>
            <h4 className="font-semibold mb-4">Community</h4>
            <ul className="space-y-2 text-sm text-fg-muted">
              <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/issues" className="hover:text-accent transition-colors">Issues</Link></li>
              <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/discussions" className="hover:text-accent transition-colors">Discussions</Link></li>
            </ul>
          </div>
          <div>
            <h4 className="font-semibold mb-4">Resources</h4>
            <ul className="space-y-2 text-sm text-fg-muted">
              <li><Link href="/docs" className="hover:text-accent transition-colors">Documentation</Link></li>
              <li><Link href="/download" className="hover:text-accent transition-colors">Download</Link></li>
            </ul>
          </div>
        </div>
        <div className="pt-8 border-t border-border flex flex-col sm:flex-row items-center justify-between gap-4 text-sm text-fg-muted">
          <p>© 2026 ZeroTerm Contributors. MIT Licensed.</p>
          <p>Built with Rust + wgpu</p>
        </div>
      </div>
    </footer>
  );
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
      <body className="min-h-dvh bg-bg text-fg font-sans antialiased">
        <Nav />
        <main>{children}</main>
        <Footer />
      </body>
    </html>
  );
}
