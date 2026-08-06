# ZeroTerm Landing

The marketing site for [ZeroTerm](https://github.com/mahesh-diwan/ZeroTerm), a
GPU-accelerated terminal emulator written in Rust.

Built with [Next.js](https://nextjs.org) (App Router), TypeScript, and
Tailwind CSS.

## Development

```bash
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000).

## Content policy

This site is kept in sync with the actual state of the project: install
methods listed here must exist (see `scripts/install.sh`), roadmap phases
reflect what is merged, and platform claims match what CI verifies. If you add
a claim that cannot be verified from the repository or its releases, it does
not belong here.

## Deployment

The site is a static export (see `next.config.ts`); deploy `out/` to any
static host.
