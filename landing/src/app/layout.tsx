import type { Metadata, Viewport } from 'next';
import { Geist, Geist_Mono } from 'next/font/google';
import Link from 'next/link';
import './globals.css';

const geist = Geist({
  subsets: ['latin'],
  variable: '--font-geist',
  display: 'swap',
});

const geistMono = Geist_Mono({
  subsets: ['latin'],
  variable: '--font-geist-mono',
  display: 'swap',
});

export const metadata: Metadata = {
  title: 'ZeroTerm — Zero latency. Zero bloat. Zero config.',
  description: 'GPU-accelerated terminal emulator built in Rust. 120 FPS at 4K, <50MB RAM, native multiplexing. No Electron, no bloat.',
  keywords: ['terminal', 'rust', 'gpu', 'wgpu', 'multiplexer', 'pty', 'ssh', 'developer-tools'],
  authors: [{ name: 'ZeroTerm Contributors' }],
  creator: 'ZeroTerm Contributors',
  publisher: 'ZeroTerm',
  robots: 'index, follow',
  openGraph: {
    type: 'website',
    locale: 'en_US',
    url: 'https://zeroterm.dev',
    siteName: 'ZeroTerm',
    title: 'ZeroTerm — Zero latency. Zero bloat. Zero config.',
    description: 'GPU-accelerated terminal emulator built in Rust. 120 FPS at 4K, <50MB RAM, native multiplexing.',
    images: [
      {
        url: '/og-image.png',
        width: 1200,
        height: 630,
        alt: 'ZeroTerm - GPU accelerated terminal',
      },
    ],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'ZeroTerm — Zero latency. Zero bloat. Zero config.',
    description: 'GPU-accelerated terminal emulator built in Rust.',
    images: ['/og-image.png'],
  },
};

export const viewport: Viewport = {
  themeColor: '#0a0a0f',
  width: 'device-width',
  initialScale: 1,
  maximumScale: 5,
};

const navLinks = [
  { href: '/', label: 'Home' },
  { href: '/download', label: 'Download' },
  { href: '/docs', label: 'Docs' },
];

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable} antialiased`}>
      <head>
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />
        <link rel="icon" href="/favicon.ico" sizes="any" />
        <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
        <link rel="manifest" href="/manifest.json" />
      </head>
      <body className="min-h-screen bg-[var(--bg)] text-[var(--fg)]">
        <header className="sticky top-0 z-50 bg-[var(--bg)]/80 backdrop-blur-lg border-b border-[var(--border)]">
          <div className="max-w-5xl mx-auto px-6 h-14 flex items-center justify-between">
            <Link href="/" className="font-bold text-lg hover:text-[var(--accent)] transition-colors">
              ZeroTerm
            </Link>
            <nav className="flex items-center gap-6">
              {navLinks.map((l) => (
                <Link key={l.href} href={l.href} className="text-sm text-[var(--fg-muted)] hover:text-[var(--accent)] transition-colors">
                  {l.label}
                </Link>
              ))}
            </nav>
          </div>
        </header>
        {children}
        <footer className="py-12 px-6 border-t border-[var(--border)]">
          <div className="max-w-5xl mx-auto">
            <div className="grid md:grid-cols-4 gap-8 mb-8">
              <div>
                <h4 className="font-bold text-lg mb-4">ZeroTerm</h4>
                <p className="text-[var(--fg-muted)] text-sm leading-relaxed">
                  GPU-accelerated terminal emulator. Zero latency. Zero bloat. Zero config.
                </p>
              </div>
              <div>
                <h4 className="font-bold text-lg mb-4">Links</h4>
                <ul className="space-y-2 text-sm text-[var(--fg-muted)]">
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm" className="hover:text-[var(--accent)] transition-colors">GitHub</Link></li>
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/issues" className="hover:text-[var(--accent)] transition-colors">Issues</Link></li>
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/releases" className="hover:text-[var(--accent)] transition-colors">Releases</Link></li>
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/blob/main/LICENSE" className="hover:text-[var(--accent)] transition-colors">License (MIT)</Link></li>
                </ul>
              </div>
              <div>
                <h4 className="font-bold text-lg mb-4">Community</h4>
                <ul className="space-y-2 text-sm text-[var(--fg-muted)]">
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/discussions" className="hover:text-[var(--accent)] transition-colors">Discussions</Link></li>
                  <li><Link href="https://discord.gg/zeroterm" className="hover:text-[var(--accent)] transition-colors">Discord (soon)</Link></li>
                </ul>
              </div>
              <div>
                <h4 className="font-bold text-lg mb-4">Resources</h4>
                <ul className="space-y-2 text-sm text-[var(--fg-muted)]">
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/blob/main/.opencode/product.md" className="hover:text-[var(--accent)] transition-colors">Product Spec</Link></li>
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/blob/main/.opencode/design.md" className="hover:text-[var(--accent)] transition-colors">Design Doc</Link></li>
                  <li><Link href="https://github.com/mahesh-diwan/ZeroTerm/blob/main/.opencode/roadmap.md" className="hover:text-[var(--accent)] transition-colors">Roadmap</Link></li>
                </ul>
              </div>
            </div>
            <div className="pt-8 border-t border-[var(--border)] flex flex-col md:flex-row items-center justify-between gap-4">
              <p className="text-sm text-[var(--fg-muted)]">
                &copy; 2026 ZeroTerm Contributors. MIT Licensed. Built with Rust + wgpu.
              </p>
              <div className="flex items-center gap-6">
                <a href="https://github.com/mahesh-diwan/ZeroTerm" className="text-[var(--fg-muted)] hover:text-[var(--accent)] transition-colors" aria-label="GitHub">
                  <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0C5.374 0 0 5.373 0 12c0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.305-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23A11.509 11.509 0 0112 5.803c1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.872.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576C20.566 21.797 24 17.3 24 12c0-6.627-5.373-12-12-12z"/></svg>
                </a>
              </div>
            </div>
          </div>
        </footer>
      </body>
    </html>
  );
}
