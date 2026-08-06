#!/usr/bin/env bash
set -euo pipefail

APP=zeroterm
ARCH=x86_64
APP_DIR="${APP}.AppDir"
APPIMAGE="${APP}-${ARCH}.AppImage"

if ! command -v cargo &>/dev/null; then
	echo "Rust not installed. Install from: https://rustup.rs"
	exit 1
fi

echo "Building release binary..."
cargo build --release

rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin"
mkdir -p "${APP_DIR}/usr/share/applications"
mkdir -p "${APP_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${APP_DIR}/usr/share/metainfo"

cp target/release/"${APP}" "${APP_DIR}/usr/bin/"

cat >"${APP_DIR}/${APP}.desktop" <<EOF
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

cat >"${APP_DIR}/usr/share/applications/${APP}.desktop" <<EOF
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

cat >"${APP_DIR}/usr/share/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml" <<EOF
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
  <url type="homepage">https://github.com/mahesh-diwan/ZeroTerm</url>
  <project_license>MIT</project_license>
</component>
EOF

# Minimal SVG icon (a simple terminal glyph)
cat >"${APP_DIR}/${APP}.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <rect width="48" height="48" rx="8" fill="#1a1a2e"/>
  <text x="24" y="32" font-family="monospace" font-size="28" fill="#00ff88" text-anchor="middle" font-weight="bold">&gt;_</text>
</svg>
SVG

cp "${APP_DIR}/${APP}.svg" "${APP_DIR}/usr/share/icons/hicolor/scalable/apps/"

# AppRun
cat >"${APP_DIR}/AppRun" <<'EOF'
#!/usr/bin/env bash
exec "${APPDIR}/usr/bin/zeroterm" "$@"
EOF
chmod +x "${APP_DIR}/AppRun"

# .DirIcon
ln -sf "${APP}.svg" "${APP_DIR}/.DirIcon"

if command -v appimagetool &>/dev/null; then
	appimagetool "${APP_DIR}" "${APPIMAGE}"
	echo "AppImage created: ${APPIMAGE}"
elif command -v linuxdeploy &>/dev/null; then
	linuxdeploy --appdir "${APP_DIR}" --output appimage
	echo "AppImage created via linuxdeploy"
else
	echo "Neither appimagetool nor linuxdeploy found."
	echo "Install from: https://github.com/AppImage/AppImageKit"
	echo "AppDir prepared at: ${APP_DIR}"
	echo "Run: appimagetool ${APP_DIR} ${APPIMAGE}"
	exit 1
fi
