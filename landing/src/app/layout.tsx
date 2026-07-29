import type { Metadata, Viewport } from 'next';
import { Geist, Geist_Mono } from 'next/font/google';
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
        {children}
      </body>
    </html>
  );
}