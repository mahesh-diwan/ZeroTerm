# Design

## Color Strategy

Restrained — pure black surface, crimson primary carries brand weight, teal accent for live-demo highlights. Primary ≤10% surface area.

## Palette (OKLCH)

```css
:root {
  --bg: oklch(0.05 0 0); /* pure near-black */
  --surface: oklch(0.08 0 0); /* elevated panels */
  --surface-hover: oklch(0.11 0 0); /* hover state */
  --border: oklch(0.18 0 0); /* hairline borders */
  --ink: oklch(0.95 0.005 20); /* warm near-white */
  --ink-muted: oklch(0.55 0.005 20); /* secondary text */
  --primary: oklch(0.45 0.15 20); /* crimson seed */
  --primary-hover: oklch(0.5 0.15 20);
  --primary-weak: oklch(0.45 0.15 20 / 0.15);
  --accent: oklch(0.65 0.15 160); /* teal for live demo */
  --accent-weak: oklch(0.65 0.15 160 / 0.12);
  --focus: oklch(0.65 0.15 160); /* visible focus ring */
}
```

## Typography

- **Display**: `Geist` (variable, 100-900) — geometric grotesk, technical precision
- **Body**: `JetBrains Mono` (variable, 100-800) — coding font, reads at small sizes
- **Scale**: clamp(2.5rem, 5vw + 1rem, 5.5rem) / clamp(1.125rem, 1.5vw + 0.5rem, 1.25rem)
- **Letter-spacing**: display ≥ -0.02em; body 0
- **Line-height**: display 1.05; body 1.6

## Layout Archetype

Asymmetrical Bento — CSS Grid masonry with varying card spans. Hero spans full width. Feature grid: 2-col base, hero feature spans 2 rows. Install section: horizontal tabs + terminal mockup. Roadmap: horizontal timeline cards.

Mobile (< 768px): single column, `w-full px-4 py-8`, all spans reset to 1.

## Component Patterns

### Double-Bezel Card

```css
.card-shell {
  @apply rounded-[2rem] p-[6px] bg-[oklch(0.05_0_0)] border border-[oklch(0.18_0_0)];
}
.card-core {
  @apply rounded-[calc(2rem-6px)] bg-[oklch(0.08_0_0)] p-6 md:p-8;
  box-shadow: inset 0 1px 1px oklch(1 0 0 / 0.05);
}
```

### Nested CTA Button

```css
.btn-primary {
  @apply rounded-full px-8 py-3 bg-[oklch(0.45_0.15_20)] text-[oklch(1_0_0)] font-medium;
  transition:
    transform 0.15s cubic-bezier(0.32, 0.72, 0, 1),
    background 0.2s;
}
.btn-primary:hover {
  @apply bg-[oklch(0.50_0.15_20)];
}
.btn-primary:active {
  @apply scale-[0.98];
}
.btn-icon {
  @apply w-8 h-8 rounded-full bg-[oklch(1_0_0/0.15)] flex items-center justify-center ml-3;
  transition: transform 0.2s cubic-bezier(0.32, 0.72, 0, 1);
}
.btn-primary:hover .btn-icon {
  transform: translateX(2px) scale(1.05);
}
```

### Terminal Mockup (Hero)

Live animated terminal using the same glyph atlas logic as the real app. 24×80 cells, cursor blink, typewriter effect cycling through commands (`cargo build --release`, `ssh prod`, `vim main.rs`). Canvas-based for 60fps.

## Motion Choreography

- **Ease**: `cubic-bezier(0.32, 0.72, 0, 1)` for all transitions
- **Entry**: `translate-y-16 blur-md opacity-0` → `translate-y-0 blur-0 opacity-100` over 800ms, stagger 100ms
- **Scroll reveal**: IntersectionObserver, trigger at 15% viewport
- **Reduced motion**: `@media (prefers-reduced-motion: reduce)` → all durations 0.01ms, no blur

## Spacing Rhythm

- Section vertical: `py-24 md:py-32 lg:py-40`
- Card internal: `p-6 md:p-8`
- Grid gap: `gap-6 md:gap-8`
- Inline: `gap-3` (tight), `gap-4` (standard), `gap-6` (loose)

## Z-Index Scale

- `z-10`: sticky nav
- `z-20`: dropdown/popover
- `z-30`: modal backdrop
- `z-40`: modal
- `z-50`: toast/tooltip

## Grid Definition

```css
.bento-grid {
  display: grid;
  grid-template-columns: repeat(12, 1fr);
  grid-auto-rows: minmax(280px, auto);
  gap: 1.5rem 2rem;
}
@media (max-width: 767px) {
  .bento-grid {
    grid-template-columns: 1fr;
    grid-auto-rows: auto;
  }
}
```

Feature cards use `col-span-6 row-span-2` (hero), `col-span-6` (standard), `col-span-4` (compact). Install section spans full width.

## Live Terminal Shader (Hero)

- Same `swash` + `wgpu` glyph atlas as desktop app
- 24 rows × 80 cols, 14px font, 1.2 line height
- Commands typed at 30ms/char, 1s pause, clear, repeat
- Cursor blink 530ms on/off
- No WebGL fallback — static SVG frame if canvas unavailable
