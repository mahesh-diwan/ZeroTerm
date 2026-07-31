# Product

## Register

brand

## Platform

web

## Users

Developers and engineers who live in the terminal. They value performance, control, and native tooling. They're tired of Electron-based terminals that consume 200MB+ RAM and have input latency. Primary audience: Rust/C++/Go engineers, CLI power users. Secondary: DevOps/SRE who need reliable SSH multiplexing.

## Product Purpose

ZeroTerm is a GPU-accelerated, cross-platform terminal emulator written in Rust. It delivers zero latency, zero bloat, zero config, zero cloud, zero tools. Native Metal/Vulkan/DX12 rendering via wgpu. Hand-written VT100 parser. Portable PTY backend. Works out of the box — TOML + Lua for power users.

Success = developers switching from Alacritty/WezTerm/Kitty/iTerm2 because it's faster, lighter, and feels native everywhere.

## Positioning

The only terminal that's truly zero-config yet infinitely hackable — native GPU rendering, Rust core, zero Electron.

## Conversion & proof

- Primary CTA: Install from source (`curl ... | bash` → `cargo build --release`). Binary releases coming soon.
- Secondary CTA: Star on GitHub / watch releases.
- The line a visitor remembers after 10 seconds: "Zero latency. Zero bloat. Zero config. Written in Rust."
- Belief ladder:
  1. Performance matters (120 FPS, <1ms keystroke-to-pixel)
  2. Rust + wgpu = real engineering, not Electron wrapper
  3. Try it — single command install, no account, no cloud
- Proof on hand: GitHub repo with 16 passing tests, working binary, live demo in hero

## Brand Personality

Fast, minimal, powerful. Technical confidence without arrogance. Show, don't tell — live terminal demo in hero proves it.

## Anti-references

Generic AI SaaS landing pages (gradient text, soft pastels, hero-metric template, identical card grids, eyebrow headers on every section). Docs-only technical sites (dense text walls, no visual hierarchy). Electron-based terminal sites that hide performance behind marketing copy.

## Design Principles

1. **Practice what you preach** — the landing page itself must feel fast, minimal, powerful
2. **Show, don't tell** — live terminal demo in hero, real code snippets, real stats
3. **Expert confidence** — speak to engineers as peers, not prospects
4. **Zero fluff** — every element earns its place; no decorative filler
5. **Native feel everywhere** — design adapts to platform conventions, not brand mandates

## Accessibility & Inclusion

WCAG AA contrast. `prefers-reduced-motion` respected — all animations reduce to instant transitions. Color-blind safe palette (no red/green status alone). Keyboard navigable. Semantic HTML.
