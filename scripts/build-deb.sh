#!/usr/bin/env bash
set -euo pipefail

APP=zeroterm
PKG_DIR="${APP}_$(git describe --tags --always 2>/dev/null || echo "0.1.0")_amd64"

if ! command -v cargo &>/dev/null; then
	echo "Rust not installed. Install from: https://rustup.rs"
	exit 1
fi

echo "Building release binary..."
cargo build --release

rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/share/applications"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${PKG_DIR}/usr/share/metainfo"

cp target/release/"${APP}" "${PKG_DIR}/usr/bin/"

cat >"${PKG_DIR}/DEBIAN/control" <<EOF
Package: ${APP}
Version: $(git describe --tags --always 2>/dev/null || echo "0.1.0")
Section: x11
Priority: optional
Architecture: amd64
Depends: libc6, libx11-6
Maintainer: ZeroTerm Developers <dev@zeroterm.dev>
Description: GPU-accelerated terminal emulator
 ZeroTerm is a modern, GPU-accelerated terminal emulator
 built with Rust and wgpu. It provides fast rendering,
 multi-tab support, and Lua configuration.
EOF

cat >"${PKG_DIR}/usr/share/applications/${APP}.desktop" <<EOF
[Desktop Entry]
Name=ZeroTerm
Comment=GPU-accelerated terminal emulator
Exec=${APP}
Icon=${APP}
Terminal=false
Type=Application
Categories=TerminalEmulator;
StartupNotify=true
EOF

cat >"${PKG_DIR}/usr/share/icons/hicolor/scalable/apps/${APP}.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <rect width="48" height="48" rx="8" fill="#1a1a2e"/>
  <text x="24" y="32" font-family="monospace" font-size="28" fill="#00ff88" text-anchor="middle" font-weight="bold">&gt;_</text>
</svg>
SVG

cat >"${PKG_DIR}/usr/share/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop">
  <id>io.github.zeroterm.ZeroTerm</id>
  <name>ZeroTerm</name>
  <summary>GPU-accelerated terminal emulator</summary>
  <description>
    <p>ZeroTerm is a modern, GPU-accelerated terminal emulator built with Rust and wgpu.</p>
  </description>
  <categories>
    <category>TerminalEmulator</category>
  </categories>
  <url type="homepage">https://github.com/zeroterm/zeroterm</url>
  <project_license>MIT</project_license>
</component>
EOF

dpkg-deb --build "${PKG_DIR}"
echo "Debian package created: ${PKG_DIR}.deb"
