#!/usr/bin/env bash
# linuxdeploy-based AppImage recipe. Downloads linuxdeploy + appimagetool,
# stages the release binary with a .desktop entry, and produces
# dist/ZeroTerm-<arch>.AppImage.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${REPO_ROOT}/target/release/zeroterm"
DIST_DIR="${REPO_ROOT}/dist"
ARCH="$(uname -m)"
APPIMAGE="${DIST_DIR}/ZeroTerm-${ARCH}.AppImage"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/zeroterm-appimage"

if [[ ! -x "${BINARY}" ]]; then
	echo "Binary not found at ${BINARY}. Build first: cargo build --release -p zeroterm" >&2
	exit 1
fi
if ! command -v curl &>/dev/null; then
	echo "curl is required to download the AppImage tooling" >&2
	exit 1
fi

# Fetch tooling on first run (cached afterwards)
mkdir -p "${CACHE_DIR}"
LINUXDEPLOY="${CACHE_DIR}/linuxdeploy-${ARCH}.AppImage"
APPIMAGETOOL="${CACHE_DIR}/appimagetool-${ARCH}.AppImage"
export PATH="${CACHE_DIR}:${PATH}"
# AppImages extracted to disk (no FUSE on most CI runners)
export APPIMAGE_EXTRACT_AND_RUN=1

if [[ ! -x "${LINUXDEPLOY}" ]]; then
	echo "Downloading linuxdeploy..."
	curl -fL --retry 3 --retry-delay 2 -o "${LINUXDEPLOY}" \
		"https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage"
	chmod +x "${LINUXDEPLOY}"
fi
if [[ ! -x "${APPIMAGETOOL}" ]]; then
	echo "Downloading appimagetool..."
	curl -fL --retry 3 --retry-delay 2 -o "${APPIMAGETOOL}" \
		"https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${ARCH}.AppImage"
	chmod +x "${APPIMAGETOOL}"
fi

# Stage binary + desktop entry into the AppDir
APP_DIR="${REPO_ROOT}/build/zeroterm.AppDir"
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin"
cp "${BINARY}" "${APP_DIR}/usr/bin/zeroterm"

cat >"${APP_DIR}/zeroterm.desktop" <<EOF
[Desktop Entry]
Name=ZeroTerm
Comment=GPU-accelerated terminal emulator
Exec=zeroterm
Icon=zeroterm
Terminal=false
Type=Application
Categories=TerminalEmulator;
StartupNotify=true
EOF

cat >"${APP_DIR}/zeroterm.svg" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <rect width="48" height="48" rx="8" fill="#1a1a2e"/>
  <text x="24" y="32" font-family="monospace" font-size="28" fill="#00ff88" text-anchor="middle" font-weight="bold">&gt;_</text>
</svg>
SVG

# linuxdeploy populates the AppDir (AppRun, icons, desktop) and, with
# --output appimage, produces ZeroTerm-<arch>.AppImage via appimagetool
mkdir -p "${DIST_DIR}"
(
	cd "${REPO_ROOT}"
	"${LINUXDEPLOY}" --appimage-extract-and-run --appdir "${APP_DIR}" \
		--desktop-file "${APP_DIR}/zeroterm.desktop" \
		--icon-file "${APP_DIR}/zeroterm.svg" \
		--output appimage
)
mv -f "${REPO_ROOT}/ZeroTerm-${ARCH}.AppImage" "${APPIMAGE}"

echo "Created ${APPIMAGE}"
