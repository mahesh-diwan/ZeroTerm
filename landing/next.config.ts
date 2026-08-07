import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Static export: the site is fully client-side, so `next build` emits a
  // plain HTML/JS bundle under ./out that GitHub Pages can serve with no
  // server runtime (no Vercel/Node needed).
  output: "export",
  trailingSlash: true,
};

export default nextConfig;
